//! User-level persisted project reading state (S-WSTATE-1, ADR-0030).
//!
//! The store is deliberately independent from routes and startup selection. It
//! owns only the versioned JSON model, canonical project identity, strict input
//! validation, platform user-data location, and whole-record atomic commits.

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::{atomic::AtomicBool, Arc};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const READING_STATE_SCHEMA_VERSION: u32 = 1;

const INDEX_FILE_NAME: &str = "reading-state-index.json";
const PROJECT_RECORDS_DIR: &str = "reading-states";
const MAX_RECORD_BYTES: u64 = 1024 * 1024;
const MAX_PROJECT_ROOT_BYTES: usize = 32 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_ITEMS: usize = 4096;
const MAX_READING_POSITIONS: usize = 8192;
const MAX_TIMESTAMP_BYTES: usize = 128;
const MAX_BLOCK_DIGEST_BYTES: usize = 256;
const MAX_ABS_OFFSET_PX: f64 = 1_000_000.0;
const MAX_TOTAL_LINES: u32 = 100_000_000;
const MAX_OCCURRENCE: u32 = 10_000_000;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadingStateIndex {
    pub schema_version: u32,
    pub recent_project_root: Option<String>,
}

impl Default for ReadingStateIndex {
    fn default() -> Self {
        Self {
            schema_version: READING_STATE_SCHEMA_VERSION,
            recent_project_root: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistedProjectReadingState {
    pub schema_version: u32,
    pub project_root: String,
    pub snapshot: ProjectReadingSnapshot,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectReadingSnapshot {
    pub expanded_directories: Vec<String>,
    pub open_files: Vec<String>,
    pub active_file: Option<String>,
    pub reading_positions: BTreeMap<String, ReadingAnchor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ReadingAnchor {
    Code {
        #[serde(rename = "topLine")]
        top_line: u32,
        #[serde(rename = "offsetPx")]
        offset_px: f64,
        #[serde(rename = "totalLines")]
        total_lines: u32,
    },
    Markdown {
        #[serde(rename = "blockDigest")]
        block_digest: String,
        occurrence: u32,
        #[serde(rename = "offsetPx")]
        offset_px: f64,
        #[serde(rename = "scrollRatio")]
        scroll_ratio: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReadingStateWarningKind {
    CorruptJson,
    UnsupportedSchema,
    ProjectRootMismatch,
    InvalidPath,
    InvalidValue,
    RecordTooLarge,
    InvalidRecord,
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingStateWarning {
    pub kind: ReadingStateWarningKind,
    pub file: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadingStateLoad<T> {
    pub value: Option<T>,
    pub warnings: Vec<ReadingStateWarning>,
}

impl<T> ReadingStateLoad<T> {
    fn missing() -> Self {
        Self {
            value: None,
            warnings: Vec::new(),
        }
    }

    fn warning(path: &Path, kind: ReadingStateWarningKind, message: impl Into<String>) -> Self {
        Self {
            value: None,
            warnings: vec![ReadingStateWarning {
                kind,
                file: path.display().to_string(),
                message: message.into(),
            }],
        }
    }
}

#[derive(Debug)]
pub enum ReadingStateError {
    Io(io::Error),
    InvalidProjectRoot(String),
    InvalidIndex(String),
    InvalidSnapshot(String),
}

impl fmt::Display for ReadingStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "reading-state IO failed: {error}"),
            Self::InvalidProjectRoot(reason) => write!(f, "invalid project root: {reason}"),
            Self::InvalidIndex(reason) => write!(f, "invalid reading-state index: {reason}"),
            Self::InvalidSnapshot(reason) => write!(f, "invalid project reading state: {reason}"),
        }
    }
}

impl std::error::Error for ReadingStateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ReadingStateError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// The parameterized variants are all exercised in tests on every host; a
// production binary naturally constructs only its compile-target platform.
#[cfg_attr(not(test), allow(dead_code))]
pub enum UserDataPlatform {
    Windows,
    MacOs,
    Unix,
}

/// Resolve the current platform's user-level Fluid data directory without
/// creating it. Failure disables only reading-state persistence; callers may
/// continue opening projects normally.
pub fn user_data_root() -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        return user_data_root_for(
            UserDataPlatform::Windows,
            std::env::var_os("LOCALAPPDATA").as_deref(),
            None,
            None,
        );
    }
    #[cfg(target_os = "macos")]
    {
        return user_data_root_for(
            UserDataPlatform::MacOs,
            None,
            None,
            std::env::var_os("HOME").as_deref(),
        );
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return user_data_root_for(
            UserDataPlatform::Unix,
            None,
            std::env::var_os("XDG_DATA_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
        );
    }
    #[allow(unreachable_code)]
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Fluid cannot resolve a user data directory on this platform",
    ))
}

/// Parameterized platform contract so Windows, macOS, and XDG behavior is
/// covered deterministically on every build host.
pub fn user_data_root_for(
    platform: UserDataPlatform,
    local_app_data: Option<&OsStr>,
    xdg_data_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> io::Result<PathBuf> {
    let base = match platform {
        UserDataPlatform::Windows => {
            PathBuf::from(nonempty_os(local_app_data).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "LOCALAPPDATA is not set; cannot resolve Fluid reading-state storage",
                )
            })?)
        }
        UserDataPlatform::MacOs => PathBuf::from(nonempty_os(home).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "HOME is not set; cannot resolve Fluid reading-state storage",
            )
        })?)
        .join("Library")
        .join("Application Support"),
        UserDataPlatform::Unix => match nonempty_os(xdg_data_home) {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from(nonempty_os(home).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "XDG_DATA_HOME and HOME are unset; cannot resolve Fluid reading-state storage",
                )
            })?)
            .join(".local")
            .join("share"),
        },
    };
    Ok(base.join("Fluid"))
}

fn nonempty_os(value: Option<&OsStr>) -> Option<&OsStr> {
    value.filter(|item| !item.is_empty())
}

/// Canonical absolute project identity shared with `ProjectReader::new`.
pub fn canonical_project_root(project_root: &Path) -> Result<String, ReadingStateError> {
    let canonical = project_root.canonicalize()?;
    if !canonical.is_dir() {
        return Err(ReadingStateError::InvalidProjectRoot(
            "project root is not a directory".into(),
        ));
    }
    let identity = canonical.to_str().ok_or_else(|| {
        ReadingStateError::InvalidProjectRoot(
            "project root cannot be represented by the JSON API".into(),
        )
    })?;
    if identity.len() > MAX_PROJECT_ROOT_BYTES {
        return Err(ReadingStateError::InvalidProjectRoot(
            "project root exceeds the persisted identity limit".into(),
        ));
    }
    Ok(identity.to_string())
}

/// SHA-256 of the canonical root string bytes. The digest is a safe file name;
/// the full root remains inside the record for collision/mismatch validation.
pub fn canonical_project_key(project_root: &Path) -> Result<String, ReadingStateError> {
    let root = canonical_project_root(project_root)?;
    Ok(hex_lower(&Sha256::digest(root.as_bytes())))
}

#[derive(Clone)]
pub struct ReadingStateStore {
    root: PathBuf,
    #[cfg(test)]
    fail_next_atomic_replace: Arc<AtomicBool>,
}

impl ReadingStateStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            #[cfg(test)]
            fail_next_atomic_replace: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn for_current_user() -> Result<Self, ReadingStateError> {
        Ok(Self::new(user_data_root()?))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn index_path(&self) -> PathBuf {
        self.root.join(INDEX_FILE_NAME)
    }

    pub fn project_record_path(&self, project_root: &Path) -> Result<PathBuf, ReadingStateError> {
        Ok(self
            .root
            .join(PROJECT_RECORDS_DIR)
            .join(format!("{}.json", canonical_project_key(project_root)?)))
    }

    pub fn load_index(&self) -> Result<ReadingStateLoad<ReadingStateIndex>, ReadingStateError> {
        let path = self.index_path();
        let Some(value) = read_json_value::<ReadingStateIndex>(&path)? else {
            return Ok(ReadingStateLoad::missing());
        };
        let value = match value {
            Ok(value) => value,
            Err(load) => return Ok(load),
        };
        if let Some(load) = schema_warning::<ReadingStateIndex>(&path, &value) {
            return Ok(load);
        }
        let index: ReadingStateIndex = match serde_json::from_value(value) {
            Ok(index) => index,
            Err(error) => {
                return Ok(ReadingStateLoad::warning(
                    &path,
                    ReadingStateWarningKind::InvalidRecord,
                    error.to_string(),
                ))
            }
        };
        match validate_index(&index) {
            Ok(()) => Ok(ReadingStateLoad {
                value: Some(index),
                warnings: Vec::new(),
            }),
            Err(validation) => Ok(ReadingStateLoad::warning(
                &path,
                validation.kind,
                validation.message,
            )),
        }
    }

    pub fn save_index(&self, index: &ReadingStateIndex) -> Result<(), ReadingStateError> {
        validate_index(index)
            .map_err(|validation| ReadingStateError::InvalidIndex(validation.message))?;
        self.write_record(&self.index_path(), index)
    }

    pub fn save_recent_project(&self, project_root: &Path) -> Result<(), ReadingStateError> {
        self.save_index(&ReadingStateIndex {
            schema_version: READING_STATE_SCHEMA_VERSION,
            recent_project_root: Some(canonical_project_root(project_root)?),
        })
    }

    // Retained as the explicit index-reset operation and covered by store tests;
    // S-WSTART-1 deliberately preserves an unavailable recent root instead.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn clear_recent_project(&self) -> Result<(), ReadingStateError> {
        self.save_index(&ReadingStateIndex::default())
    }

    pub fn load_project(
        &self,
        project_root: &Path,
    ) -> Result<ReadingStateLoad<PersistedProjectReadingState>, ReadingStateError> {
        let expected_root = canonical_project_root(project_root)?;
        let path = self.project_record_path(project_root)?;
        let Some(value) = read_json_value::<PersistedProjectReadingState>(&path)? else {
            return Ok(ReadingStateLoad::missing());
        };
        let value = match value {
            Ok(value) => value,
            Err(load) => return Ok(load),
        };
        if let Some(load) = schema_warning::<PersistedProjectReadingState>(&path, &value) {
            return Ok(load);
        }
        let record: PersistedProjectReadingState = match serde_json::from_value(value) {
            Ok(record) => record,
            Err(error) => {
                return Ok(ReadingStateLoad::warning(
                    &path,
                    ReadingStateWarningKind::InvalidRecord,
                    error.to_string(),
                ))
            }
        };
        if record.project_root != expected_root {
            return Ok(ReadingStateLoad::warning(
                &path,
                ReadingStateWarningKind::ProjectRootMismatch,
                "stored projectRoot does not match the requested canonical root",
            ));
        }
        match validate_project_record(&record) {
            Ok(()) => Ok(ReadingStateLoad {
                value: Some(record),
                warnings: Vec::new(),
            }),
            Err(validation) => Ok(ReadingStateLoad::warning(
                &path,
                validation.kind,
                validation.message,
            )),
        }
    }

    pub fn save_project(
        &self,
        project_root: &Path,
        snapshot: &ProjectReadingSnapshot,
        updated_at: &str,
    ) -> Result<(), ReadingStateError> {
        let record = PersistedProjectReadingState {
            schema_version: READING_STATE_SCHEMA_VERSION,
            project_root: canonical_project_root(project_root)?,
            snapshot: snapshot.clone(),
            updated_at: updated_at.to_string(),
        };
        validate_project_record(&record)
            .map_err(|validation| ReadingStateError::InvalidSnapshot(validation.message))?;
        self.write_record(&self.project_record_path(project_root)?, &record)
    }

    fn write_record<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), ReadingStateError> {
        let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| {
            ReadingStateError::InvalidSnapshot(format!("cannot serialize record: {error}"))
        })?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(ReadingStateError::InvalidSnapshot(
                "serialized record exceeds the size limit".into(),
            ));
        }
        let dir = path.parent().ok_or_else(|| {
            ReadingStateError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reading-state destination has no parent directory",
            ))
        })?;
        fs::create_dir_all(dir)?;
        reject_non_regular_destination(path)?;
        let (temp_path, mut temp_file) = create_temp_file(dir, path)?;
        let write_result = (|| -> io::Result<()> {
            temp_file.write_all(&bytes)?;
            temp_file.sync_all()?;
            drop(temp_file);
            #[cfg(test)]
            if self.fail_next_atomic_replace.swap(false, Ordering::SeqCst) {
                return Err(io::Error::other(
                    "injected failure before reading-state atomic replacement",
                ));
            }
            atomic_replace(&temp_path, path)?;
            sync_directory_best_effort(dir);
            Ok(())
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temp_path);
            return Err(ReadingStateError::Io(error));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn fail_next_atomic_replace_for_test(&self) {
        self.fail_next_atomic_replace.store(true, Ordering::SeqCst);
    }
}

#[derive(Debug)]
struct ValidationError {
    kind: ReadingStateWarningKind,
    message: String,
}

impl ValidationError {
    fn new(kind: ReadingStateWarningKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

fn validate_index(index: &ReadingStateIndex) -> Result<(), ValidationError> {
    if index.schema_version != READING_STATE_SCHEMA_VERSION {
        return Err(ValidationError::new(
            ReadingStateWarningKind::UnsupportedSchema,
            format!("unsupported schemaVersion {}", index.schema_version),
        ));
    }
    if let Some(root) = &index.recent_project_root {
        validate_stored_project_root(root)?;
    }
    Ok(())
}

fn validate_project_record(record: &PersistedProjectReadingState) -> Result<(), ValidationError> {
    if record.schema_version != READING_STATE_SCHEMA_VERSION {
        return Err(ValidationError::new(
            ReadingStateWarningKind::UnsupportedSchema,
            format!("unsupported schemaVersion {}", record.schema_version),
        ));
    }
    validate_stored_project_root(&record.project_root)?;
    validate_timestamp(&record.updated_at)?;
    validate_snapshot(&record.snapshot)
}

fn validate_stored_project_root(root: &str) -> Result<(), ValidationError> {
    if root.is_empty()
        || root.len() > MAX_PROJECT_ROOT_BYTES
        || root.contains('\0')
        || !Path::new(root).is_absolute()
    {
        return Err(ValidationError::new(
            ReadingStateWarningKind::InvalidPath,
            "project root must be a bounded absolute path",
        ));
    }
    Ok(())
}

fn validate_timestamp(timestamp: &str) -> Result<(), ValidationError> {
    if timestamp.trim().is_empty()
        || timestamp.len() > MAX_TIMESTAMP_BYTES
        || timestamp.contains('\0')
    {
        return Err(ValidationError::new(
            ReadingStateWarningKind::InvalidValue,
            "updatedAt must be a bounded non-blank string",
        ));
    }
    Ok(())
}

fn validate_snapshot(snapshot: &ProjectReadingSnapshot) -> Result<(), ValidationError> {
    validate_path_list(
        "expandedDirectories",
        &snapshot.expanded_directories,
        MAX_PATH_ITEMS,
    )?;
    validate_path_list("openFiles", &snapshot.open_files, MAX_PATH_ITEMS)?;
    if snapshot.reading_positions.len() > MAX_READING_POSITIONS {
        return Err(ValidationError::new(
            ReadingStateWarningKind::InvalidValue,
            "readingPositions exceeds the item limit",
        ));
    }
    if let Some(active) = &snapshot.active_file {
        validate_relative_path(active)?;
        if !snapshot.open_files.iter().any(|path| path == active) {
            return Err(ValidationError::new(
                ReadingStateWarningKind::InvalidValue,
                "activeFile must identify an entry in openFiles",
            ));
        }
    }
    for (path, anchor) in &snapshot.reading_positions {
        validate_relative_path(path)?;
        validate_anchor(path, anchor)?;
    }
    Ok(())
}

fn validate_path_list(
    field: &str,
    paths: &[String],
    max_items: usize,
) -> Result<(), ValidationError> {
    if paths.len() > max_items {
        return Err(ValidationError::new(
            ReadingStateWarningKind::InvalidValue,
            format!("{field} exceeds the item limit"),
        ));
    }
    let mut seen = HashSet::with_capacity(paths.len());
    for path in paths {
        validate_relative_path(path)?;
        if !seen.insert(path.as_str()) {
            return Err(ValidationError::new(
                ReadingStateWarningKind::InvalidPath,
                format!("{field} contains duplicate path {path:?}"),
            ));
        }
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), ValidationError> {
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || path.contains('\0')
        || path.contains('\\')
        || path.starts_with('/')
        || path.ends_with('/')
        || has_windows_drive_prefix(path)
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(ValidationError::new(
            ReadingStateWarningKind::InvalidPath,
            format!(
                "snapshot path must be normalized project-relative forward-slash text: {path:?}"
            ),
        ));
    }
    Ok(())
}

fn validate_anchor(path: &str, anchor: &ReadingAnchor) -> Result<(), ValidationError> {
    match anchor {
        ReadingAnchor::Code {
            top_line,
            offset_px,
            total_lines,
        } => {
            if *top_line == 0
                || *total_lines == 0
                || *top_line > *total_lines
                || *total_lines > MAX_TOTAL_LINES
                || !valid_offset(*offset_px)
            {
                return Err(ValidationError::new(
                    ReadingStateWarningKind::InvalidValue,
                    format!("invalid code reading anchor for {path:?}"),
                ));
            }
        }
        ReadingAnchor::Markdown {
            block_digest,
            occurrence,
            offset_px,
            scroll_ratio,
        } => {
            if !valid_block_digest(block_digest)
                || *occurrence > MAX_OCCURRENCE
                || !valid_offset(*offset_px)
                || !scroll_ratio.is_finite()
                || !(0.0..=1.0).contains(scroll_ratio)
            {
                return Err(ValidationError::new(
                    ReadingStateWarningKind::InvalidValue,
                    format!("invalid markdown reading anchor for {path:?}"),
                ));
            }
        }
    }
    Ok(())
}

fn valid_offset(value: f64) -> bool {
    value.is_finite() && value.abs() <= MAX_ABS_OFFSET_PX
}

fn valid_block_digest(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_BLOCK_DIGEST_BYTES
        && value
            .chars()
            .all(|character| !character.is_control() && !character.is_whitespace())
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// `None` means missing. `Some(Err(load))` is an isolated per-record warning.
fn read_json_value<T>(
    path: &Path,
) -> Result<Option<Result<Value, ReadingStateLoad<T>>>, ReadingStateError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Ok(Some(Err(ReadingStateLoad::warning(
                path,
                ReadingStateWarningKind::InvalidRecord,
                "record is not a regular file",
            ))))
        }
        Ok(metadata) if metadata.len() > MAX_RECORD_BYTES => {
            return Ok(Some(Err(ReadingStateLoad::warning(
                path,
                ReadingStateWarningKind::RecordTooLarge,
                "record exceeds the byte limit",
            ))))
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Ok(Some(Err(ReadingStateLoad::warning(
                path,
                ReadingStateWarningKind::Io,
                error.to_string(),
            ))))
        }
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Ok(Some(Err(ReadingStateLoad::warning(
                path,
                ReadingStateWarningKind::Io,
                error.to_string(),
            ))))
        }
    };
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Ok(Some(Err(ReadingStateLoad::warning(
            path,
            ReadingStateWarningKind::RecordTooLarge,
            "record grew beyond the byte limit while being read",
        ))));
    }
    match serde_json::from_slice(&bytes) {
        Ok(value) => Ok(Some(Ok(value))),
        Err(error) => Ok(Some(Err(ReadingStateLoad::warning(
            path,
            ReadingStateWarningKind::CorruptJson,
            error.to_string(),
        )))),
    }
}

fn schema_warning<T>(path: &Path, value: &Value) -> Option<ReadingStateLoad<T>> {
    match value.get("schemaVersion").and_then(Value::as_u64) {
        Some(version) if version == u64::from(READING_STATE_SCHEMA_VERSION) => None,
        Some(version) => Some(ReadingStateLoad::warning(
            path,
            ReadingStateWarningKind::UnsupportedSchema,
            format!("unsupported schemaVersion {version}"),
        )),
        None => Some(ReadingStateLoad::warning(
            path,
            ReadingStateWarningKind::InvalidRecord,
            "record is missing an integer schemaVersion",
        )),
    }
}

fn reject_non_regular_destination(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination is not a regular file",
            ))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn create_temp_file(dir: &Path, destination: &Path) -> io::Result<(PathBuf, File)> {
    let stem = destination
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("reading-state");
    for _ in 0..16 {
        let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = dir.join(format!(
            ".{stem}.{}.{}.{}.tmp",
            std::process::id(),
            now,
            nonce
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique reading-state temporary file",
    ))
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: both buffers are owned, NUL-terminated UTF-16 path strings and
    // remain alive for the duration of the Win32 call.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(unix)]
fn sync_directory_best_effort(dir: &Path) {
    if let Ok(directory) = File::open(dir) {
        let _ = directory.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_directory_best_effort(_dir: &Path) {}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    const UPDATED_AT: &str = "2026-08-12T12:34:56Z";

    fn sample_snapshot() -> ProjectReadingSnapshot {
        ProjectReadingSnapshot {
            expanded_directories: vec!["src".into(), "src/nested".into()],
            open_files: vec!["src/main.rs".into(), "README.md".into()],
            active_file: Some("src/main.rs".into()),
            reading_positions: BTreeMap::from([
                (
                    "src/main.rs".into(),
                    ReadingAnchor::Code {
                        top_line: 12,
                        offset_px: -3.5,
                        total_lines: 80,
                    },
                ),
                (
                    "README.md".into(),
                    ReadingAnchor::Markdown {
                        block_digest:
                            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                                .into(),
                        occurrence: 1,
                        offset_px: 4.0,
                        scroll_ratio: 0.25,
                    },
                ),
            ]),
        }
    }

    #[test]
    fn canonical_project_keys_are_stable_and_isolate_two_roots() {
        let temp = TempDir::new("keys");
        let left = temp.make_dir("left");
        let right = temp.make_dir("right");
        let store = ReadingStateStore::new(temp.path().join("user-data"));

        let left_key = canonical_project_key(&left).unwrap();
        assert_eq!(left_key, canonical_project_key(&left.join(".")).unwrap());
        assert_ne!(left_key, canonical_project_key(&right).unwrap());

        let left_snapshot = sample_snapshot();
        let mut right_snapshot = sample_snapshot();
        right_snapshot.open_files = vec!["README.md".into()];
        right_snapshot.active_file = Some("README.md".into());
        store
            .save_project(&left, &left_snapshot, UPDATED_AT)
            .unwrap();
        store
            .save_project(&right, &right_snapshot, UPDATED_AT)
            .unwrap();

        let left_load = store.load_project(&left).unwrap();
        let right_load = store.load_project(&right).unwrap();
        assert!(left_load.warnings.is_empty());
        assert!(right_load.warnings.is_empty());
        assert_eq!(left_load.value.unwrap().snapshot, left_snapshot);
        assert_eq!(right_load.value.unwrap().snapshot, right_snapshot);
        assert_ne!(
            store.project_record_path(&left).unwrap(),
            store.project_record_path(&right).unwrap()
        );
    }

    #[test]
    fn recent_project_index_round_trips_and_unknown_schema_is_left_untouched() {
        let temp = TempDir::new("index");
        let project = temp.make_dir("project");
        let store = ReadingStateStore::new(temp.path().join("user-data"));

        assert_eq!(store.load_index().unwrap(), ReadingStateLoad::missing());
        store.save_recent_project(&project).unwrap();

        let restarted = ReadingStateStore::new(store.root().to_path_buf());
        let loaded = restarted.load_index().unwrap();
        assert!(loaded.warnings.is_empty());
        assert_eq!(
            loaded.value.unwrap().recent_project_root,
            Some(canonical_project_root(&project).unwrap())
        );

        restarted.clear_recent_project().unwrap();
        assert_eq!(
            ReadingStateStore::new(store.root().to_path_buf())
                .load_index()
                .unwrap()
                .value,
            Some(ReadingStateIndex::default())
        );

        let path = store.index_path();
        let unknown = serde_json::json!({
            "schemaVersion": 99,
            "recentProjectRoot": null,
        });
        fs::write(&path, serde_json::to_vec_pretty(&unknown).unwrap()).unwrap();
        let before = fs::read(&path).unwrap();
        let rejected = store.load_index().unwrap();
        assert_eq!(
            rejected.warnings[0].kind,
            ReadingStateWarningKind::UnsupportedSchema
        );
        assert!(rejected.value.is_none());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn bad_records_are_classified_without_hiding_a_valid_neighbor() {
        let temp = TempDir::new("bad-records");
        let bad_root = temp.make_dir("bad");
        let good_root = temp.make_dir("good");
        let store = ReadingStateStore::new(temp.path().join("user-data"));
        store
            .save_project(&good_root, &sample_snapshot(), UPDATED_AT)
            .unwrap();
        let bad_path = store.project_record_path(&bad_root).unwrap();
        fs::create_dir_all(bad_path.parent().unwrap()).unwrap();

        fs::write(&bad_path, b"{not-json").unwrap();
        assert_eq!(
            store.load_project(&bad_root).unwrap().warnings[0].kind,
            ReadingStateWarningKind::CorruptJson
        );

        let canonical_bad = canonical_project_root(&bad_root).unwrap();
        let record = PersistedProjectReadingState {
            schema_version: READING_STATE_SCHEMA_VERSION,
            project_root: canonical_bad,
            snapshot: sample_snapshot(),
            updated_at: UPDATED_AT.into(),
        };
        let mut json = serde_json::to_value(&record).unwrap();
        json["schemaVersion"] = 99.into();
        fs::write(&bad_path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
        assert_eq!(
            store.load_project(&bad_root).unwrap().warnings[0].kind,
            ReadingStateWarningKind::UnsupportedSchema
        );

        json = serde_json::to_value(&record).unwrap();
        json["projectRoot"] = canonical_project_root(&good_root).unwrap().into();
        fs::write(&bad_path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
        assert_eq!(
            store.load_project(&bad_root).unwrap().warnings[0].kind,
            ReadingStateWarningKind::ProjectRootMismatch
        );

        for invalid_path in [
            "C:/outside.rs",
            "/outside.rs",
            "../outside.rs",
            "src\\main.rs",
            "src/./main.rs",
            "src//main.rs",
        ] {
            json = serde_json::to_value(&record).unwrap();
            json["snapshot"]["openFiles"] = serde_json::json!([invalid_path]);
            json["snapshot"]["activeFile"] = serde_json::Value::Null;
            fs::write(&bad_path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
            assert_eq!(
                store.load_project(&bad_root).unwrap().warnings[0].kind,
                ReadingStateWarningKind::InvalidPath,
                "path should be rejected: {invalid_path:?}"
            );
        }

        json = serde_json::to_value(&record).unwrap();
        json["unexpected"] = serde_json::json!(true);
        fs::write(&bad_path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
        assert_eq!(
            store.load_project(&bad_root).unwrap().warnings[0].kind,
            ReadingStateWarningKind::InvalidRecord
        );

        let good = store.load_project(&good_root).unwrap();
        assert!(good.warnings.is_empty());
        assert_eq!(good.value.unwrap().snapshot, sample_snapshot());
        assert!(bad_path.exists(), "invalid records must not be deleted");
    }

    #[test]
    fn successful_replacement_is_restart_readable_and_cleans_temp_file() {
        let temp = TempDir::new("successful-replace");
        let project = temp.make_dir("project");
        let store = ReadingStateStore::new(temp.path().join("user-data"));
        store
            .save_project(&project, &sample_snapshot(), UPDATED_AT)
            .unwrap();
        let path = store.project_record_path(&project).unwrap();
        let before = fs::read(&path).unwrap();

        let mut replacement = sample_snapshot();
        replacement.expanded_directories = vec!["docs".into()];
        replacement.active_file = Some("README.md".into());
        store
            .save_project(&project, &replacement, "2026-08-12T12:35:00Z")
            .unwrap();

        assert_ne!(fs::read(&path).unwrap(), before);
        assert_eq!(temporary_file_count(path.parent().unwrap()), 0);
        let restarted = ReadingStateStore::new(store.root().to_path_buf());
        let loaded = restarted.load_project(&project).unwrap();
        assert!(loaded.warnings.is_empty());
        assert_eq!(loaded.value.unwrap().snapshot, replacement);
    }

    #[test]
    fn injected_atomic_failure_keeps_old_bytes_and_cleans_temp_file() {
        let temp = TempDir::new("atomic");
        let project = temp.make_dir("project");
        let store = ReadingStateStore::new(temp.path().join("user-data"));
        let original = sample_snapshot();
        store.save_project(&project, &original, UPDATED_AT).unwrap();
        let path = store.project_record_path(&project).unwrap();
        let before = fs::read(&path).unwrap();

        let mut replacement = original;
        replacement.expanded_directories = vec!["different".into()];
        store.fail_next_atomic_replace_for_test();
        assert!(store
            .save_project(&project, &replacement, "2026-08-12T12:35:00Z")
            .is_err());

        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(temporary_file_count(path.parent().unwrap()), 0);
    }

    #[test]
    fn invalid_snapshot_values_never_replace_the_last_valid_record() {
        let temp = TempDir::new("invalid-values");
        let project = temp.make_dir("project");
        let store = ReadingStateStore::new(temp.path().join("user-data"));
        store
            .save_project(&project, &sample_snapshot(), UPDATED_AT)
            .unwrap();
        let path = store.project_record_path(&project).unwrap();
        let before = fs::read(&path).unwrap();

        let mut invalid_active = sample_snapshot();
        invalid_active.active_file = Some("src/not-open.rs".into());
        assert!(matches!(
            store.save_project(&project, &invalid_active, UPDATED_AT),
            Err(ReadingStateError::InvalidSnapshot(_))
        ));

        let mut invalid_number = sample_snapshot();
        let ReadingAnchor::Markdown { scroll_ratio, .. } = invalid_number
            .reading_positions
            .get_mut("README.md")
            .unwrap()
        else {
            panic!("sample README anchor should be markdown");
        };
        *scroll_ratio = f64::NAN;
        assert!(matches!(
            store.save_project(&project, &invalid_number, UPDATED_AT),
            Err(ReadingStateError::InvalidSnapshot(_))
        ));

        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(temporary_file_count(path.parent().unwrap()), 0);
    }

    #[test]
    fn oversized_record_is_reported_without_reading_or_deleting_it() {
        let temp = TempDir::new("oversized");
        let project = temp.make_dir("project");
        let store = ReadingStateStore::new(temp.path().join("user-data"));
        let path = store.project_record_path(&project).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let oversized = vec![b' '; MAX_RECORD_BYTES as usize + 1];
        fs::write(&path, &oversized).unwrap();

        let loaded = store.load_project(&project).unwrap();
        assert!(loaded.value.is_none());
        assert_eq!(
            loaded.warnings[0].kind,
            ReadingStateWarningKind::RecordTooLarge
        );
        assert_eq!(fs::metadata(&path).unwrap().len(), oversized.len() as u64);
    }

    #[test]
    fn platform_user_data_roots_follow_windows_macos_and_xdg_contracts() {
        assert_eq!(
            user_data_root_for(
                UserDataPlatform::Windows,
                Some(OsStr::new(r"C:\Users\reader\AppData\Local")),
                None,
                None,
            )
            .unwrap(),
            PathBuf::from(r"C:\Users\reader\AppData\Local").join("Fluid")
        );
        assert_eq!(
            user_data_root_for(
                UserDataPlatform::MacOs,
                None,
                None,
                Some(OsStr::new("/Users/reader")),
            )
            .unwrap(),
            PathBuf::from("/Users/reader/Library/Application Support/Fluid")
        );
        assert_eq!(
            user_data_root_for(
                UserDataPlatform::Unix,
                None,
                Some(OsStr::new("/var/data/reader")),
                Some(OsStr::new("/home/reader")),
            )
            .unwrap(),
            PathBuf::from("/var/data/reader/Fluid")
        );
        assert_eq!(
            user_data_root_for(
                UserDataPlatform::Unix,
                None,
                None,
                Some(OsStr::new("/home/reader")),
            )
            .unwrap(),
            PathBuf::from("/home/reader/.local/share/Fluid")
        );
    }

    fn temporary_file_count(dir: &Path) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension() == Some(OsStr::new("tmp")))
            .count()
    }

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "fluid-reading-state-{label}-{}-{}",
                std::process::id(),
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn make_dir(&self, name: &str) -> PathBuf {
            let path = self.path.join(name);
            fs::create_dir_all(&path).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
