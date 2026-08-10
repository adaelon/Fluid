//! Project-scoped persisted query threads.
//!
//! This module owns the durable record boundary under
//! `<project>/.fluid/query-threads/v1/`. It deliberately does not expose HTTP or
//! WebSocket behavior: later slices consume this store after the schema, source
//! identity, validation, and atomic file semantics are independently testable.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::context_assembler::{validate_query_map, QueryMap};
use crate::web_evidence::{EvidenceStatus, SourceLink};

pub const QUERY_THREAD_SCHEMA_VERSION: u32 = 1;
const SOURCE_REVISION_SCHEMA_TAG: &[u8] = b"fluid-query-source-v1\0";
const THREAD_ID_HEX_LEN: usize = 32;
static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The source scope frozen into a query thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum QueryScopeSpec {
    Current { paths: Vec<String> },
    Selected { paths: Vec<String> },
}

impl QueryScopeSpec {
    pub fn paths(&self) -> &[String] {
        match self {
            Self::Current { paths } | Self::Selected { paths } => paths,
        }
    }
}

/// Persisted metadata for the web/project evidence attached to one complete turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryEvidenceState {
    pub status: EvidenceStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// One complete, durable question/answer pair. Partial streaming output is never
/// represented by this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistedQueryTurn {
    pub question: String,
    pub answer: String,
    pub map: QueryMap,
    pub evidence: Option<QueryEvidenceState>,
    pub code_evidence_ids: Vec<String>,
    pub completed_at: String,
}

/// The complete project-level record. Prompt budgeting derives a bounded
/// `QueryTrace` from this record and must never truncate `turns` in place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryThread {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub scope: QueryScopeSpec,
    pub source_revision: String,
    pub original_question: String,
    pub turns: Vec<PersistedQueryTurn>,
}

/// Freshness is derived from current project bytes every time a thread is read.
/// It is never persisted into the durable thread record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueryFreshness {
    Fresh,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueryStaleReason {
    SourceChanged,
    SourceMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryThreadFreshness {
    pub freshness: QueryFreshness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<QueryStaleReason>,
}

impl QueryThreadFreshness {
    pub fn fresh() -> Self {
        Self {
            freshness: QueryFreshness::Fresh,
            stale_reason: None,
        }
    }

    pub fn stale(reason: QueryStaleReason) -> Self {
        Self {
            freshness: QueryFreshness::Stale,
            stale_reason: Some(reason),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryForkUnavailableReason {
    Fresh,
    SourceMissing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryThreadStoreWarning {
    pub file: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryThreadScan {
    pub threads: Vec<QueryThread>,
    pub warnings: Vec<QueryThreadStoreWarning>,
}

#[derive(Debug)]
pub enum QueryHistoryError {
    Io(io::Error),
    InvalidScope(String),
    InvalidThread(String),
    SourceMissing(String),
    SourceForbidden(String),
    ForkUnavailable(QueryForkUnavailableReason),
    StorageEscapesProject(PathBuf),
    InvalidRecord { path: PathBuf, reason: String },
}

impl fmt::Display for QueryHistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "query history IO failed: {error}"),
            Self::InvalidScope(reason) => write!(f, "invalid query scope: {reason}"),
            Self::InvalidThread(reason) => write!(f, "invalid query thread: {reason}"),
            Self::SourceMissing(path) => write!(f, "query source is missing: {path}"),
            Self::SourceForbidden(path) => {
                write!(f, "query source escapes the project root: {path}")
            }
            Self::ForkUnavailable(QueryForkUnavailableReason::Fresh) => {
                write!(f, "fresh query thread does not need fork-current")
            }
            Self::ForkUnavailable(QueryForkUnavailableReason::SourceMissing) => {
                write!(f, "source-missing query thread cannot be forked")
            }
            Self::StorageEscapesProject(path) => write!(
                f,
                "query history storage escapes the project root: {}",
                path.display()
            ),
            Self::InvalidRecord { path, reason } => {
                write!(
                    f,
                    "invalid query history record {}: {reason}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for QueryHistoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for QueryHistoryError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Normalize one query scope into its stable project-relative identity.
pub fn normalize_query_scope(scope: &QueryScopeSpec) -> Result<QueryScopeSpec, QueryHistoryError> {
    let mut paths = scope
        .paths()
        .iter()
        .map(|path| normalize_project_path(path))
        .collect::<Result<Vec<_>, _>>()?;

    paths.sort_by(|left, right| js_utf16_cmp(left, right));
    paths.dedup();

    match scope {
        QueryScopeSpec::Current { .. } if paths.len() == 1 => Ok(QueryScopeSpec::Current { paths }),
        QueryScopeSpec::Current { .. } => Err(QueryHistoryError::InvalidScope(
            "current scope must contain exactly one path".into(),
        )),
        QueryScopeSpec::Selected { .. } if paths.len() >= 2 => {
            Ok(QueryScopeSpec::Selected { paths })
        }
        QueryScopeSpec::Selected { .. } => Err(QueryHistoryError::InvalidScope(
            "selected scope must contain at least two distinct paths".into(),
        )),
    }
}

/// Derive the v1 title from the first non-blank line of the original question.
pub fn default_thread_title(question: &str) -> Option<String> {
    question
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// One store is bound to one canonical project root for its entire lifetime.
#[derive(Clone)]
pub struct QueryThreadStore {
    project_root: PathBuf,
    dir: PathBuf,
}

impl QueryThreadStore {
    /// Bind a store to an existing project directory. No `.fluid` path is created
    /// until the first successful `put`.
    pub fn new(project_root: &Path) -> Result<Self, QueryHistoryError> {
        let project_root = project_root.canonicalize()?;
        if !project_root.is_dir() {
            return Err(QueryHistoryError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "project root is not a directory",
            )));
        }
        let dir = project_root.join(".fluid").join("query-threads").join("v1");
        Ok(Self { project_root, dir })
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn storage_dir(&self) -> &Path {
        &self.dir
    }

    /// Generate an opaque, server-owned ID. Path use remains gated by
    /// `validate_thread_id`, so even a caller-controlled record cannot escape.
    pub fn generate_thread_id(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(b"fluid-query-thread-id-v1\0");
        hash.update(self.project_root.to_string_lossy().as_bytes());
        hash.update(std::process::id().to_be_bytes());
        hash.update(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                .to_be_bytes(),
        );
        hash.update(
            UNIQUE_COUNTER
                .fetch_add(1, AtomicOrdering::Relaxed)
                .to_be_bytes(),
        );
        let digest = hash.finalize();
        hex_lower(&digest[..THREAD_ID_HEX_LEN / 2])
    }

    /// Compute the stable source identity from normalized paths and exact bytes.
    pub fn source_revision(&self, scope: &QueryScopeSpec) -> Result<String, QueryHistoryError> {
        let scope = normalize_query_scope(scope)?;
        let mut hash = Sha256::new();
        hash.update(SOURCE_REVISION_SCHEMA_TAG);
        for path in scope.paths() {
            let bytes = self.read_source_bytes(path)?;
            hash_length_prefixed(&mut hash, path.as_bytes());
            hash_length_prefixed(&mut hash, &bytes);
        }
        Ok(hex_lower(&hash.finalize()))
    }

    /// Compare a persisted source revision with current project bytes. Missing
    /// or now-forbidden paths are presented as unavailable source rather than a
    /// fatal list/read error.
    pub fn freshness(
        &self,
        thread: &QueryThread,
    ) -> Result<QueryThreadFreshness, QueryHistoryError> {
        match self.source_revision(&thread.scope) {
            Ok(revision) if revision == thread.source_revision => Ok(QueryThreadFreshness::fresh()),
            Ok(_) => Ok(QueryThreadFreshness::stale(QueryStaleReason::SourceChanged)),
            Err(QueryHistoryError::SourceMissing(_) | QueryHistoryError::SourceForbidden(_)) => {
                Ok(QueryThreadFreshness::stale(QueryStaleReason::SourceMissing))
            }
            Err(error) => Err(error),
        }
    }

    /// Create and persist a zero-turn thread for the current project bytes.
    pub fn create_thread(
        &self,
        scope: QueryScopeSpec,
        original_question: &str,
        created_at: &str,
    ) -> Result<QueryThread, QueryHistoryError> {
        let scope = normalize_query_scope(&scope)?;
        let source_revision = self.source_revision(&scope)?;
        let thread = build_zero_turn_thread(
            self.generate_thread_id(),
            scope,
            source_revision,
            original_question,
            created_at,
        )?;
        self.put(&thread)?;
        Ok(thread)
    }

    /// Fork only a source-changed thread. The new record copies the question and
    /// normalized scope, then binds them to current bytes without copying turns.
    pub fn fork_thread_current(
        &self,
        source: &QueryThread,
        created_at: &str,
    ) -> Result<QueryThread, QueryHistoryError> {
        validate_query_thread(source)?;
        let source_revision = match self.source_revision(&source.scope) {
            Ok(revision) if revision == source.source_revision => {
                return Err(QueryHistoryError::ForkUnavailable(
                    QueryForkUnavailableReason::Fresh,
                ))
            }
            Ok(revision) => revision,
            Err(QueryHistoryError::SourceMissing(_) | QueryHistoryError::SourceForbidden(_)) => {
                return Err(QueryHistoryError::ForkUnavailable(
                    QueryForkUnavailableReason::SourceMissing,
                ))
            }
            Err(error) => return Err(error),
        };
        let thread = build_zero_turn_thread(
            self.generate_thread_id(),
            source.scope.clone(),
            source_revision,
            &source.original_question,
            created_at,
        )?;
        self.put(&thread)?;
        Ok(thread)
    }

    /// Atomically replace one validated thread record using a same-directory
    /// temporary file. A failed validation or write leaves the previous record.
    pub fn put(&self, thread: &QueryThread) -> Result<(), QueryHistoryError> {
        validate_query_thread(thread)?;
        let dir = self.ensure_storage_dir()?;
        let destination = dir.join(record_file_name(&thread.id)?);
        reject_non_regular_destination(&destination)?;
        let json = serde_json::to_vec_pretty(thread).map_err(|error| {
            QueryHistoryError::InvalidThread(format!("cannot serialize thread: {error}"))
        })?;

        let (temp_path, mut temp_file) = create_temp_file(&dir, &thread.id)?;
        let write_result = (|| -> io::Result<()> {
            temp_file.write_all(&json)?;
            temp_file.sync_all()?;
            drop(temp_file);
            fs::rename(&temp_path, &destination)?;
            sync_directory_best_effort(&dir);
            Ok(())
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temp_path);
            return Err(QueryHistoryError::Io(error));
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<QueryThread>, QueryHistoryError> {
        let file_name = record_file_name(id)?;
        let Some(dir) = self.existing_storage_dir()? else {
            return Ok(None);
        };
        let path = dir.join(file_name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(invalid_record(&path, "record is not a regular file"));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(QueryHistoryError::Io(error)),
        }
        let thread = self.read_record(&dir, &path)?;
        if thread.id != id {
            return Err(invalid_record(
                &path,
                format!("record id {:?} does not match file name", thread.id),
            ));
        }
        Ok(Some(thread))
    }

    /// Scan every v1 JSON record. One corrupt/unknown record becomes a warning
    /// and cannot hide valid neighbors or be deleted as a side effect.
    pub fn list(&self) -> Result<QueryThreadScan, QueryHistoryError> {
        let Some(dir) = self.existing_storage_dir()? else {
            return Ok(QueryThreadScan {
                threads: Vec::new(),
                warnings: Vec::new(),
            });
        };
        let mut threads = Vec::new();
        let mut warnings = Vec::new();
        for entry_result in fs::read_dir(&dir)? {
            let entry = match entry_result {
                Ok(entry) => entry,
                Err(error) => {
                    warnings.push(QueryThreadStoreWarning {
                        file: "<directory-entry>".into(),
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().extension().and_then(|part| part.to_str()) != Some("json") {
                continue;
            }
            match entry.file_type() {
                Ok(file_type) if file_type.is_file() => {}
                Ok(_) => {
                    warnings.push(QueryThreadStoreWarning {
                        file: file_name,
                        message: "record is not a regular file".into(),
                    });
                    continue;
                }
                Err(error) => {
                    warnings.push(QueryThreadStoreWarning {
                        file: file_name,
                        message: error.to_string(),
                    });
                    continue;
                }
            }
            let entry_path = entry.path();
            let stem = entry_path.file_stem().and_then(|part| part.to_str());
            let Some(id) = stem.filter(|id| validate_thread_id(id).is_ok()) else {
                warnings.push(QueryThreadStoreWarning {
                    file: file_name,
                    message: "file name is not a valid thread id".into(),
                });
                continue;
            };
            match self.read_record(&dir, &entry_path) {
                Ok(thread) if thread.id == id => threads.push(thread),
                Ok(thread) => warnings.push(QueryThreadStoreWarning {
                    file: file_name,
                    message: format!("record id {:?} does not match file name", thread.id),
                }),
                Err(error) => warnings.push(QueryThreadStoreWarning {
                    file: file_name,
                    message: error.to_string(),
                }),
            }
        }
        threads.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(QueryThreadScan { threads, warnings })
    }

    /// Delete only a validated ID below this store's current project root.
    pub fn delete(&self, id: &str) -> Result<bool, QueryHistoryError> {
        let file_name = record_file_name(id)?;
        let Some(dir) = self.existing_storage_dir()? else {
            return Ok(false);
        };
        let path = dir.join(file_name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                Err(invalid_record(&path, "record is not a regular file"))
            }
            Ok(_) => {
                fs::remove_file(path)?;
                sync_directory_best_effort(&dir);
                Ok(true)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(QueryHistoryError::Io(error)),
        }
    }

    fn read_source_bytes(&self, relative: &str) -> Result<Vec<u8>, QueryHistoryError> {
        let joined = self.project_root.join(Path::new(relative));
        let canonical = match joined.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(QueryHistoryError::SourceMissing(relative.into()))
            }
            Err(error) => return Err(QueryHistoryError::Io(error)),
        };
        if !canonical.starts_with(&self.project_root) {
            return Err(QueryHistoryError::SourceForbidden(relative.into()));
        }
        if !canonical.is_file() {
            return Err(QueryHistoryError::SourceMissing(relative.into()));
        }
        fs::read(canonical).map_err(QueryHistoryError::Io)
    }

    fn ensure_storage_dir(&self) -> Result<PathBuf, QueryHistoryError> {
        let mut ancestor = self.dir.as_path();
        while !ancestor.exists() {
            ancestor = ancestor
                .parent()
                .ok_or_else(|| QueryHistoryError::StorageEscapesProject(self.dir.clone()))?;
        }
        let canonical_ancestor = ancestor.canonicalize()?;
        if !canonical_ancestor.starts_with(&self.project_root) {
            return Err(QueryHistoryError::StorageEscapesProject(canonical_ancestor));
        }
        fs::create_dir_all(&self.dir)?;
        let canonical_dir = self.dir.canonicalize()?;
        if !canonical_dir.starts_with(&self.project_root) || !canonical_dir.is_dir() {
            return Err(QueryHistoryError::StorageEscapesProject(canonical_dir));
        }
        Ok(canonical_dir)
    }

    fn existing_storage_dir(&self) -> Result<Option<PathBuf>, QueryHistoryError> {
        let canonical_dir = match self.dir.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(QueryHistoryError::Io(error)),
        };
        if !canonical_dir.starts_with(&self.project_root) || !canonical_dir.is_dir() {
            return Err(QueryHistoryError::StorageEscapesProject(canonical_dir));
        }
        Ok(Some(canonical_dir))
    }

    fn read_record(&self, dir: &Path, path: &Path) -> Result<QueryThread, QueryHistoryError> {
        let canonical = path
            .canonicalize()
            .map_err(|error| invalid_record(path, error.to_string()))?;
        if !canonical.starts_with(dir) {
            return Err(invalid_record(path, "record escapes the storage directory"));
        }
        let bytes = fs::read(&canonical).map_err(QueryHistoryError::Io)?;
        let thread: QueryThread = serde_json::from_slice(&bytes)
            .map_err(|error| invalid_record(path, error.to_string()))?;
        validate_query_thread(&thread).map_err(|error| invalid_record(path, error.to_string()))?;
        Ok(thread)
    }
}

/// Pure constructor used by create/fork orchestration after the caller has
/// resolved the project-bound source revision and server-owned identity.
pub fn build_zero_turn_thread(
    id: String,
    scope: QueryScopeSpec,
    source_revision: String,
    original_question: &str,
    created_at: &str,
) -> Result<QueryThread, QueryHistoryError> {
    let scope = normalize_query_scope(&scope)?;
    let title = default_thread_title(original_question).ok_or_else(|| {
        QueryHistoryError::InvalidThread("originalQuestion must not be blank".into())
    })?;
    let thread = QueryThread {
        schema_version: QUERY_THREAD_SCHEMA_VERSION,
        id,
        title,
        created_at: created_at.to_string(),
        updated_at: created_at.to_string(),
        scope,
        source_revision,
        original_question: original_question.to_string(),
        turns: Vec::new(),
    };
    validate_query_thread(&thread)?;
    Ok(thread)
}

pub fn validate_query_thread(thread: &QueryThread) -> Result<(), QueryHistoryError> {
    if thread.schema_version != QUERY_THREAD_SCHEMA_VERSION {
        return Err(QueryHistoryError::InvalidThread(format!(
            "unsupported schemaVersion {}",
            thread.schema_version
        )));
    }
    validate_thread_id(&thread.id)?;
    let expected_title = default_thread_title(&thread.original_question).ok_or_else(|| {
        QueryHistoryError::InvalidThread("originalQuestion must not be blank".into())
    })?;
    if thread.title != expected_title {
        return Err(QueryHistoryError::InvalidThread(
            "title must equal the first non-blank line of originalQuestion".into(),
        ));
    }
    if thread.created_at.trim().is_empty() || thread.updated_at.trim().is_empty() {
        return Err(QueryHistoryError::InvalidThread(
            "createdAt and updatedAt must not be blank".into(),
        ));
    }
    if !is_lower_hex(&thread.source_revision, 64) {
        return Err(QueryHistoryError::InvalidThread(
            "sourceRevision must be a 64-character lowercase SHA-256 hex string".into(),
        ));
    }
    let normalized = normalize_query_scope(&thread.scope)?;
    if normalized != thread.scope {
        return Err(QueryHistoryError::InvalidThread(
            "scope paths must already be normalized, sorted, and deduplicated".into(),
        ));
    }

    for (index, turn) in thread.turns.iter().enumerate() {
        if turn.question.trim().is_empty()
            || turn.answer.trim().is_empty()
            || turn.completed_at.trim().is_empty()
        {
            return Err(QueryHistoryError::InvalidThread(format!(
                "turn {index} is incomplete"
            )));
        }
        if index == 0 && turn.question != thread.original_question {
            return Err(QueryHistoryError::InvalidThread(
                "the first turn question must equal originalQuestion".into(),
            ));
        }
        validate_query_map(&turn.map).map_err(|reason| {
            QueryHistoryError::InvalidThread(format!("turn {index} map: {reason}"))
        })?;
        let available: HashSet<&str> = turn
            .map
            .evidence
            .iter()
            .map(|reference| reference.id.as_str())
            .collect();
        let mut seen = HashSet::new();
        for evidence_id in &turn.code_evidence_ids {
            if !seen.insert(evidence_id.as_str()) || !available.contains(evidence_id.as_str()) {
                return Err(QueryHistoryError::InvalidThread(format!(
                    "turn {index} has duplicate or unknown codeEvidenceId {evidence_id:?}"
                )));
            }
        }
        if let Some(evidence) = &turn.evidence {
            validate_query_evidence(index, evidence)?;
        }
    }

    let expected_updated_at = thread
        .turns
        .last()
        .map(|turn| turn.completed_at.as_str())
        .unwrap_or(thread.created_at.as_str());
    if thread.updated_at != expected_updated_at {
        return Err(QueryHistoryError::InvalidThread(
            "updatedAt must equal createdAt or the latest completedAt".into(),
        ));
    }
    Ok(())
}

fn validate_query_evidence(
    turn_index: usize,
    evidence: &QueryEvidenceState,
) -> Result<(), QueryHistoryError> {
    let mut sources = HashSet::new();
    for source in &evidence.sources {
        if source.title.trim().is_empty()
            || source.url.trim().is_empty()
            || !sources.insert(source.url.as_str())
        {
            return Err(QueryHistoryError::InvalidThread(format!(
                "turn {turn_index} has an invalid or duplicate evidence source"
            )));
        }
    }
    if evidence.status == EvidenceStatus::WebCited && evidence.sources.is_empty() {
        return Err(QueryHistoryError::InvalidThread(format!(
            "turn {turn_index} web-cited evidence must include a source"
        )));
    }
    Ok(())
}

fn validate_thread_id(id: &str) -> Result<(), QueryHistoryError> {
    if !is_lower_hex(id, THREAD_ID_HEX_LEN) {
        return Err(QueryHistoryError::InvalidThread(format!(
            "thread id must be {THREAD_ID_HEX_LEN} lowercase hexadecimal characters"
        )));
    }
    Ok(())
}

fn normalize_project_path(path: &str) -> Result<String, QueryHistoryError> {
    if path.is_empty() || path.contains('\0') {
        return Err(QueryHistoryError::InvalidScope(
            "scope path must not be empty or contain NUL".into(),
        ));
    }
    let slash_path = path.replace('\\', "/");
    if slash_path.starts_with('/') || has_windows_drive_prefix(&slash_path) {
        return Err(QueryHistoryError::InvalidScope(format!(
            "scope path must be project-relative: {path:?}"
        )));
    }
    let mut normalized = Vec::new();
    for component in Path::new(&slash_path).components() {
        match component {
            Component::Normal(part) => normalized.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(QueryHistoryError::InvalidScope(format!(
                    "scope path contains traversal or a root prefix: {path:?}"
                )))
            }
        }
    }
    if normalized.is_empty() {
        return Err(QueryHistoryError::InvalidScope(format!(
            "scope path has no file component: {path:?}"
        )));
    }
    Ok(normalized.join("/"))
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// JavaScript's default string ordering compares UTF-16 code units. Rust's
/// `str` order compares UTF-8 bytes, which differs for some non-BMP paths.
fn js_utf16_cmp(left: &str, right: &str) -> Ordering {
    let mut left = left.encode_utf16();
    let mut right = right.encode_utf16();
    loop {
        match (left.next(), right.next()) {
            (Some(left), Some(right)) => match left.cmp(&right) {
                Ordering::Equal => {}
                ordering => return ordering,
            },
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (None, None) => return Ordering::Equal,
        }
    }
}

fn hash_length_prefixed(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn record_file_name(id: &str) -> Result<String, QueryHistoryError> {
    validate_thread_id(id)?;
    Ok(format!("{id}.json"))
}

fn invalid_record(path: &Path, reason: impl Into<String>) -> QueryHistoryError {
    QueryHistoryError::InvalidRecord {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

fn reject_non_regular_destination(path: &Path) -> Result<(), QueryHistoryError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(invalid_record(path, "destination is not a regular file"))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(QueryHistoryError::Io(error)),
    }
}

fn create_temp_file(dir: &Path, id: &str) -> Result<(PathBuf, File), QueryHistoryError> {
    for _ in 0..16 {
        let nonce = UNIQUE_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = dir.join(format!(
            ".{id}.{}.{}.{}.tmp",
            std::process::id(),
            now,
            nonce
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(QueryHistoryError::Io(error)),
        }
    }
    Err(QueryHistoryError::Io(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique query thread temporary file",
    )))
}

#[cfg(unix)]
fn sync_directory_best_effort(dir: &Path) {
    if let Ok(directory) = File::open(dir) {
        let _ = directory.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_directory_best_effort(_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orientation::{
        ActorBoundary, CodeEvidenceRef, OrientationActor, OrientationWalkthrough, WalkthroughStep,
    };

    const THREAD_ID: &str = "0123456789abcdef0123456789abcdef";
    const CREATED_AT: &str = "2026-08-10T10:00:00Z";
    const COMPLETED_AT: &str = "2026-08-10T10:01:00Z";

    fn current(path: &str) -> QueryScopeSpec {
        QueryScopeSpec::Current {
            paths: vec![path.into()],
        }
    }

    fn selected(paths: &[&str]) -> QueryScopeSpec {
        QueryScopeSpec::Selected {
            paths: paths.iter().map(|path| (*path).into()).collect(),
        }
    }

    fn sample_map(path: &str) -> QueryMap {
        QueryMap {
            actors: vec![OrientationActor {
                id: "file".into(),
                name: path.into(),
                role: "source file".into(),
                boundary: ActorBoundary::Project,
            }],
            direction: Vec::new(),
            core_function_ids: vec!["f#1".into()],
            supporting_function_ids: Vec::new(),
            walkthrough: OrientationWalkthrough {
                title: "Call path".into(),
                input: "request".into(),
                steps: vec![WalkthroughStep {
                    text: "Read the source".into(),
                    evidence_ids: vec!["E1".into()],
                }],
            },
            evidence: vec![CodeEvidenceRef {
                id: "E1".into(),
                file_path: path.into(),
                start_line: 1,
                end_line: 1,
                symbol: Some("f".into()),
            }],
        }
    }

    fn sample_thread(scope: QueryScopeSpec, revision: String) -> QueryThread {
        let original_question = "How does this work?".to_string();
        QueryThread {
            schema_version: QUERY_THREAD_SCHEMA_VERSION,
            id: THREAD_ID.into(),
            title: default_thread_title(&original_question).unwrap(),
            created_at: CREATED_AT.into(),
            updated_at: COMPLETED_AT.into(),
            scope,
            source_revision: revision,
            original_question: original_question.clone(),
            turns: vec![PersistedQueryTurn {
                question: original_question,
                answer: "It reads the file.".into(),
                map: sample_map("src/a.rs"),
                evidence: Some(QueryEvidenceState {
                    status: EvidenceStatus::ProjectSource,
                    sources: Vec::new(),
                    warning: None,
                }),
                code_evidence_ids: vec!["E1".into()],
                completed_at: COMPLETED_AT.into(),
            }],
        }
    }

    fn write_source(root: &Path, path: &str, bytes: &[u8]) {
        let destination = root.join(Path::new(path));
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, bytes).unwrap();
    }

    #[test]
    fn selected_scope_sorts_like_javascript_and_deduplicates_paths() {
        let normalized = normalize_query_scope(&selected(&[
            "src/b.rs",
            "src/\u{e000}.rs",
            "src/a.rs",
            "src/\u{10000}.rs",
            "src/b.rs",
        ]))
        .unwrap();

        assert_eq!(
            normalized.paths(),
            [
                "src/a.rs",
                "src/b.rs",
                "src/\u{10000}.rs",
                "src/\u{e000}.rs"
            ]
        );
    }

    #[test]
    fn scope_normalization_rejects_traversal_and_too_small_selected_sets() {
        assert!(matches!(
            normalize_query_scope(&current("../outside.rs")),
            Err(QueryHistoryError::InvalidScope(_))
        ));
        assert!(matches!(
            normalize_query_scope(&selected(&["src/a.rs", "./src/a.rs"])),
            Err(QueryHistoryError::InvalidScope(_))
        ));
    }

    #[test]
    fn source_revision_is_order_independent_and_changes_with_bytes_or_paths() {
        let dir = tempdir_guard::TempDir::new("revision");
        write_source(dir.path(), "src/a.rs", b"fn a() {}\n");
        write_source(dir.path(), "src/b.rs", b"fn b() {}\n");
        write_source(dir.path(), "src/c.rs", b"fn a() {}\n");
        let store = QueryThreadStore::new(dir.path()).unwrap();

        let first = store
            .source_revision(&selected(&["src/b.rs", "src/a.rs", "src/b.rs"]))
            .unwrap();
        let reordered = store
            .source_revision(&selected(&["src/a.rs", "src/b.rs"]))
            .unwrap();
        assert_eq!(first, reordered);
        assert!(is_lower_hex(&first, 64));

        let path_changed = store
            .source_revision(&selected(&["src/b.rs", "src/c.rs"]))
            .unwrap();
        assert_ne!(first, path_changed, "path bytes are part of the identity");

        fs::write(dir.path().join("src/a.rs"), b"fn a() { 1 }\n").unwrap();
        let bytes_changed = store
            .source_revision(&selected(&["src/a.rs", "src/b.rs"]))
            .unwrap();
        assert_ne!(first, bytes_changed);
    }

    #[test]
    fn source_revision_reports_missing_files_and_never_mutates_source() {
        let dir = tempdir_guard::TempDir::new("missing");
        write_source(dir.path(), "src/a.rs", b"fn a() {}\n");
        let source = dir.path().join("src/a.rs");
        let bytes_before = fs::read(&source).unwrap();
        let modified_before = fs::metadata(&source).unwrap().modified().unwrap();
        let store = QueryThreadStore::new(dir.path()).unwrap();

        assert!(matches!(
            store.source_revision(&current("src/missing.rs")),
            Err(QueryHistoryError::SourceMissing(path)) if path == "src/missing.rs"
        ));
        assert_eq!(
            store.source_revision(&current("src/a.rs")).unwrap(),
            "64216b7551d06ec2963ac2d6e613f6ffa8b7ca7d04d2abe7ed11a2a4de19ddf5"
        );
        assert_eq!(fs::read(&source).unwrap(), bytes_before);
        assert_eq!(
            fs::metadata(&source).unwrap().modified().unwrap(),
            modified_before
        );
    }

    #[test]
    fn put_get_list_delete_round_trip_and_keep_project_roots_isolated() {
        let left = tempdir_guard::TempDir::new("left-root");
        let right = tempdir_guard::TempDir::new("right-root");
        write_source(left.path(), "src/a.rs", b"left\n");
        write_source(right.path(), "src/a.rs", b"right\n");
        let left_store = QueryThreadStore::new(left.path()).unwrap();
        let right_store = QueryThreadStore::new(right.path()).unwrap();
        let left_revision = left_store.source_revision(&current("src/a.rs")).unwrap();
        let right_revision = right_store.source_revision(&current("src/a.rs")).unwrap();
        assert_ne!(left_revision, right_revision);

        let thread = sample_thread(current("src/a.rs"), left_revision);
        let left_source = left.path().join("src/a.rs");
        let source_bytes_before = fs::read(&left_source).unwrap();
        let source_modified_before = fs::metadata(&left_source).unwrap().modified().unwrap();
        left_store.put(&thread).unwrap();
        assert_eq!(fs::read(&left_source).unwrap(), source_bytes_before);
        assert_eq!(
            fs::metadata(&left_source).unwrap().modified().unwrap(),
            source_modified_before
        );
        assert_eq!(left_store.get(THREAD_ID).unwrap(), Some(thread.clone()));
        assert_eq!(right_store.get(THREAD_ID).unwrap(), None);
        let scan = left_store.list().unwrap();
        assert_eq!(scan.threads, vec![thread]);
        assert!(scan.warnings.is_empty());
        assert!(left_store.delete(THREAD_ID).unwrap());
        assert!(!left_store.delete(THREAD_ID).unwrap());
        assert_eq!(left_store.get(THREAD_ID).unwrap(), None);
    }

    #[test]
    fn zero_turn_thread_round_trips_for_first_question_retry() {
        let dir = tempdir_guard::TempDir::new("zero-turn");
        write_source(dir.path(), "src/a.rs", b"fn a() {}\n");
        let store = QueryThreadStore::new(dir.path()).unwrap();
        let revision = store.source_revision(&current("src/a.rs")).unwrap();
        let mut thread = sample_thread(current("src/a.rs"), revision);
        thread.turns.clear();
        thread.updated_at.clone_from(&thread.created_at);

        store.put(&thread).unwrap();
        assert_eq!(store.get(THREAD_ID).unwrap(), Some(thread));
    }

    #[test]
    fn list_orders_valid_threads_by_latest_update_then_id() {
        let dir = tempdir_guard::TempDir::new("list-order");
        write_source(dir.path(), "src/a.rs", b"fn a() {}\n");
        let store = QueryThreadStore::new(dir.path()).unwrap();
        let revision = store.source_revision(&current("src/a.rs")).unwrap();
        let earlier = sample_thread(current("src/a.rs"), revision.clone());
        let mut later = sample_thread(current("src/a.rs"), revision);
        later.id = "33333333333333333333333333333333".into();
        later.updated_at = "2026-08-10T11:01:00Z".into();
        later.turns[0].completed_at.clone_from(&later.updated_at);

        store.put(&earlier).unwrap();
        store.put(&later).unwrap();
        let ids: Vec<_> = store
            .list()
            .unwrap()
            .threads
            .into_iter()
            .map(|thread| thread.id)
            .collect();
        assert_eq!(ids, vec![later.id, earlier.id]);
    }

    #[test]
    fn atomic_overwrite_replaces_whole_record_and_failed_validation_keeps_old_bytes() {
        let dir = tempdir_guard::TempDir::new("atomic");
        write_source(dir.path(), "src/a.rs", b"fn a() {}\n");
        let store = QueryThreadStore::new(dir.path()).unwrap();
        let revision = store.source_revision(&current("src/a.rs")).unwrap();
        let original = sample_thread(current("src/a.rs"), revision);
        store.put(&original).unwrap();

        let mut replacement = original.clone();
        replacement.turns[0].answer = "The complete replacement answer.".into();
        store.put(&replacement).unwrap();
        assert_eq!(store.get(THREAD_ID).unwrap(), Some(replacement.clone()));
        assert_eq!(
            fs::read_dir(store.storage_dir())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry.path().extension().and_then(|part| part.to_str()) == Some("tmp")
                })
                .count(),
            0
        );

        let record = store.storage_dir().join(format!("{THREAD_ID}.json"));
        let bytes_before = fs::read(&record).unwrap();
        let mut invalid = replacement;
        invalid.schema_version = 2;
        assert!(store.put(&invalid).is_err());
        assert_eq!(fs::read(record).unwrap(), bytes_before);
    }

    #[test]
    fn corrupt_json_and_unknown_schema_are_warned_and_isolated() {
        let dir = tempdir_guard::TempDir::new("corrupt");
        write_source(dir.path(), "src/a.rs", b"fn a() {}\n");
        let store = QueryThreadStore::new(dir.path()).unwrap();
        let revision = store.source_revision(&current("src/a.rs")).unwrap();
        let valid = sample_thread(current("src/a.rs"), revision);
        store.put(&valid).unwrap();

        let corrupt_id = "11111111111111111111111111111111";
        fs::write(
            store.storage_dir().join(format!("{corrupt_id}.json")),
            b"{ not json",
        )
        .unwrap();
        let unknown_id = "22222222222222222222222222222222";
        let mut unknown = serde_json::to_value(&valid).unwrap();
        unknown["id"] = unknown_id.into();
        unknown["schemaVersion"] = 2.into();
        fs::write(
            store.storage_dir().join(format!("{unknown_id}.json")),
            serde_json::to_vec_pretty(&unknown).unwrap(),
        )
        .unwrap();

        let scan = store.list().unwrap();
        assert_eq!(scan.threads, vec![valid]);
        assert_eq!(scan.warnings.len(), 2);
        assert!(store
            .storage_dir()
            .join(format!("{corrupt_id}.json"))
            .exists());
        assert!(store
            .storage_dir()
            .join(format!("{unknown_id}.json"))
            .exists());
        assert!(matches!(
            store.get(unknown_id),
            Err(QueryHistoryError::InvalidRecord { .. })
        ));
    }

    #[test]
    fn thread_ids_block_path_traversal_for_every_store_operation() {
        let dir = tempdir_guard::TempDir::new("traversal");
        write_source(dir.path(), "src/a.rs", b"fn a() {}\n");
        let outside = dir.path().with_extension("outside.json");
        fs::write(&outside, b"keep").unwrap();
        let store = QueryThreadStore::new(dir.path()).unwrap();
        let revision = store.source_revision(&current("src/a.rs")).unwrap();
        let mut thread = sample_thread(current("src/a.rs"), revision);
        thread.id = "../../fluid-query-outside".into();

        assert!(matches!(
            store.put(&thread),
            Err(QueryHistoryError::InvalidThread(_))
        ));
        assert!(matches!(
            store.get("../../fluid-query-outside"),
            Err(QueryHistoryError::InvalidThread(_))
        ));
        assert!(matches!(
            store.delete("../../fluid-query-outside"),
            Err(QueryHistoryError::InvalidThread(_))
        ));
        assert_eq!(fs::read(&outside).unwrap(), b"keep");
        fs::remove_file(outside).unwrap();
    }

    #[test]
    fn generated_thread_ids_are_opaque_valid_and_distinct() {
        let dir = tempdir_guard::TempDir::new("ids");
        let store = QueryThreadStore::new(dir.path()).unwrap();
        let first = store.generate_thread_id();
        let second = store.generate_thread_id();
        validate_thread_id(&first).unwrap();
        validate_thread_id(&second).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn freshness_classifies_matching_changed_and_missing_sources() {
        let dir = tempdir_guard::TempDir::new("freshness");
        write_source(dir.path(), "src/a.rs", b"fn a() {}\n");
        let store = QueryThreadStore::new(dir.path()).unwrap();
        let revision = store.source_revision(&current("src/a.rs")).unwrap();
        let thread = sample_thread(current("src/a.rs"), revision);

        assert_eq!(
            store.freshness(&thread).unwrap(),
            QueryThreadFreshness::fresh()
        );

        fs::write(dir.path().join("src/a.rs"), b"fn a() { 1 }\n").unwrap();
        assert_eq!(
            store.freshness(&thread).unwrap(),
            QueryThreadFreshness::stale(QueryStaleReason::SourceChanged)
        );

        fs::remove_file(dir.path().join("src/a.rs")).unwrap();
        assert_eq!(
            store.freshness(&thread).unwrap(),
            QueryThreadFreshness::stale(QueryStaleReason::SourceMissing)
        );
    }

    #[test]
    fn create_and_fork_current_freeze_only_the_current_source_identity() {
        const FORKED_AT: &str = "2026-08-10T10:02:00Z";

        let dir = tempdir_guard::TempDir::new("create-fork");
        write_source(dir.path(), "src/a.rs", b"fn a() {}\n");
        let store = QueryThreadStore::new(dir.path()).unwrap();

        let created = store
            .create_thread(
                current("./src/a.rs"),
                "\n  How does this work?  \nMore detail",
                CREATED_AT,
            )
            .unwrap();
        assert_eq!(created.title, "How does this work?");
        assert_eq!(created.scope, current("src/a.rs"));
        assert_eq!(created.created_at, CREATED_AT);
        assert_eq!(created.updated_at, CREATED_AT);
        assert!(created.turns.is_empty());
        assert_eq!(store.get(&created.id).unwrap(), Some(created.clone()));
        assert!(matches!(
            store.fork_thread_current(&created, FORKED_AT),
            Err(QueryHistoryError::ForkUnavailable(
                QueryForkUnavailableReason::Fresh
            ))
        ));

        fs::write(dir.path().join("src/a.rs"), b"fn a() { 1 }\n").unwrap();
        let forked = store.fork_thread_current(&created, FORKED_AT).unwrap();
        assert_ne!(forked.id, created.id);
        assert_eq!(forked.original_question, created.original_question);
        assert_eq!(forked.title, created.title);
        assert_eq!(forked.scope, created.scope);
        assert_ne!(forked.source_revision, created.source_revision);
        assert_eq!(forked.created_at, FORKED_AT);
        assert_eq!(forked.updated_at, FORKED_AT);
        assert!(forked.turns.is_empty());
        assert_eq!(
            store.freshness(&forked).unwrap(),
            QueryThreadFreshness::fresh()
        );

        fs::remove_file(dir.path().join("src/a.rs")).unwrap();
        assert!(matches!(
            store.fork_thread_current(&created, FORKED_AT),
            Err(QueryHistoryError::ForkUnavailable(
                QueryForkUnavailableReason::SourceMissing
            ))
        ));
    }

    mod tempdir_guard {
        use std::path::{Path, PathBuf};

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new(label: &str) -> Self {
                let unique = format!(
                    "fluid-query-history-{label}-{}-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos(),
                    super::UNIQUE_COUNTER.fetch_add(1, super::AtomicOrdering::Relaxed)
                );
                let path = std::env::temp_dir().join(unique);
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
