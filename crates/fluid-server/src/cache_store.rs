//! CacheStore — on-disk bypass caches for generated semantic artifacts.
//!
//! Function capsules/lines/translations, selection explanations, and validated
//! file-orientation cards are written *outside* the source tree so the core law
//! "zero byte contamination" holds (核心律 1). Layout is split below `.fluid/`
//! into `capsules/`, `selections/`, and `orientations/`.
//!
//! Capsule key (技术方案 §6, refining ADR-0003/0021): function source span +
//! normalized file-orientation coordinates + provider/model + capsule-specific
//! prompt/schema versions. Per-function granularity keeps unchanged siblings hot
//! when a regenerated file card has the same normalized coordinates.
//!
//! Hash = FNV-1a 64-bit, computed inline. A disk-persisted key needs a hash that
//! is stable across processes, platforms and toolchain versions — `std`'s
//! `DefaultHasher` makes no such guarantee, so we use a fixed algorithm instead.
//! No external crate, matching this project's "write the small util yourself"
//! habit (cf. S1's hand-rolled TempDir). A 64-bit key is non-cryptographic; a
//! collision would surface a wrong cached entry, but the probability across a
//! single project's function spans is negligible. 何时回头: if collisions ever
//! bite, swap in SHA-256 (an algorithm change just wipes the cache — cheap).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::orientation::{
    FileOrientationCard, FunctionRole, OrientationCacheIdentity, OrientationValidationContext,
};
use crate::web_evidence::{EvidenceStatus, SourceLink};

/// Product-specific versions for function capsules. Keeping these separate from
/// line/translation prompts prevents an unrelated prompt bump from flushing the
/// whole bypass cache.
pub const CAPSULE_PROMPT_VERSION: &str = "capsule-p1";
pub const CAPSULE_SCHEMA_VERSION: u32 = 1;

/// A function-granularity semantic capsule bound to one backend-validated file
/// orientation and its exact role (技术方案 §3, ADR-0021).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capsule {
    #[serde(rename = "fnId")]
    pub fn_id: String,
    pub signature: String,
    pub summary: String,
    pub complexity: String,
    pub io: String,
    #[serde(rename = "orientationId")]
    pub orientation_id: String,
    pub role: FunctionRole,
}

/// A line-level ghost annotation attached to a key line (技术方案 §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineAnnotation {
    #[serde(rename = "fnId")]
    pub fn_id: String,
    #[serde(rename = "lineNumber")]
    pub line_number: u32,
    pub text: String,
    pub color: String,
}

/// One cache entry = a function's capsule plus its line annotations, stored
/// together (技术方案 §6: 行级注释随所属函数胶囊一同存取).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapsuleEntry {
    pub capsule: Capsule,
    pub lines: Vec<LineAnnotation>,
}

/// Exact inputs that identify one function-capsule artifact. The file-level
/// orientation ID is deliberately not included: a regenerated card whose
/// normalized coordinate projection is unchanged may reuse unaffected sibling
/// capsules, while any actual actor/flow/role/evidence change produces a miss.
#[derive(Debug, Clone, Copy)]
pub struct CapsuleCacheIdentity<'a> {
    pub fn_source: &'a str,
    pub orientation_context_hash: &'a str,
    pub provider_base_url: &'a str,
    pub model: &'a str,
    pub prompt_version: &'a str,
    pub schema_version: u32,
}

impl CapsuleCacheIdentity<'_> {
    pub fn key(&self) -> String {
        let mut hash = FNV_OFFSET;
        for part in [
            "capsule-cache-v2",
            self.fn_source,
            self.orientation_context_hash,
            self.provider_base_url,
            self.model,
            self.prompt_version,
        ] {
            hash = fnv1a_step(hash, part.as_bytes());
            hash = fnv1a_step(hash, &[0]);
        }
        hash = fnv1a_step(hash, &self.schema_version.to_le_bytes());
        format!("{hash:016x}")
    }
}

/// A cached whole-document translation (文档翻译): the Simplified-Chinese Markdown
/// produced from an English doc, code blocks preserved verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Translation {
    pub text: String,
}

/// Coarse semantic kind returned for one arbitrary code selection. The model may
/// classify a selection, but the backend constrains the value to this closed set
/// before it can be cached or sent to the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionKind {
    #[serde(rename = "模块")]
    Module,
    #[serde(rename = "类型")]
    Type,
    #[serde(rename = "函数")]
    Function,
    #[serde(rename = "方法")]
    Method,
    #[serde(rename = "变量")]
    Variable,
    #[serde(rename = "表达式")]
    Expression,
    #[serde(rename = "未知")]
    Unknown,
}

/// The stable selection-explanation product shared by the WebSocket response and
/// the on-disk bypass cache. Evidence metadata is injected by the backend rather
/// than trusted to the model's JSON reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionExplanation {
    pub selected_text: String,
    pub kind: SelectionKind,
    pub meaning: String,
    pub role_here: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    pub evidence_status: EvidenceStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionCacheEntry {
    pub explanation: SelectionExplanation,
}

/// On-disk bypass cache rooted at `<project>/.fluid/capsules/`.
#[derive(Clone)]
pub struct CacheStore {
    dir: PathBuf,
    // File-orientation artifacts are isolated from function capsules.
    orientation_dir: PathBuf,
    selection_dir: PathBuf,
    model_version: String,
    prompt_version: String,
}

impl CacheStore {
    /// Build a cache rooted under `project_root`. Nothing is created on disk
    /// until the first `put`.
    pub fn new(
        project_root: &Path,
        model_version: impl Into<String>,
        prompt_version: impl Into<String>,
    ) -> Self {
        Self {
            dir: project_root.join(".fluid").join("capsules"),
            orientation_dir: project_root.join(".fluid").join("orientations"),
            selection_dir: project_root.join(".fluid").join("selections"),
            model_version: model_version.into(),
            prompt_version: prompt_version.into(),
        }
    }

    /// Look up a function capsule by source + normalized orientation identity.
    /// Corrupt or legacy entries (which lack orientation binding) are misses.
    pub fn get_capsule(&self, identity: &CapsuleCacheIdentity<'_>) -> Option<CapsuleEntry> {
        let path = self.capsule_path_for(identity);
        let bytes = std::fs::read(&path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Persist a bound function capsule under `.fluid/capsules/<key>.json`.
    pub fn put_capsule(
        &self,
        identity: &CapsuleCacheIdentity<'_>,
        entry: &CapsuleEntry,
    ) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let json = serde_json::to_vec_pretty(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(self.capsule_path_for(identity), json)
    }

    fn capsule_path_for(&self, identity: &CapsuleCacheIdentity<'_>) -> PathBuf {
        self.dir.join(format!("{}.json", identity.key()))
    }

    /// Cache key for a single manual-line annotation (S9 explain-line). Folds the
    /// target line number and a discriminant into the function-span key, so a
    /// line entry never aliases the function's capsule entry (different bytes →
    /// different hash) even for the same source.
    pub fn line_key(&self, fn_source: &str, line_number: u32) -> String {
        let mut hash = FNV_OFFSET;
        for part in [
            self.model_version.as_str(),
            self.prompt_version.as_str(),
            "explain-line",
            fn_source,
        ] {
            // NUL separator so concatenation can't alias across fields.
            hash = fnv1a_step(hash, part.as_bytes());
            hash = fnv1a_step(hash, &[0]);
        }
        hash = fnv1a_step(hash, &line_number.to_le_bytes());
        hash = fnv1a_step(hash, &[0]);
        format!("{hash:016x}")
    }

    /// Look up a single manual-line annotation. Same miss/corrupt semantics as
    /// `get` (S9): a missing or unreadable entry reads as absent → recompute.
    pub fn get_line(&self, fn_source: &str, line_number: u32) -> Option<LineAnnotation> {
        let path = self.line_path_for(fn_source, line_number);
        let bytes = std::fs::read(&path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Persist a single manual-line annotation under `.fluid/capsules/<line_key>.json`.
    pub fn put_line(
        &self,
        fn_source: &str,
        line_number: u32,
        line: &LineAnnotation,
    ) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let json = serde_json::to_vec_pretty(line)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(self.line_path_for(fn_source, line_number), json)
    }

    fn line_path_for(&self, fn_source: &str, line_number: u32) -> PathBuf {
        self.dir
            .join(format!("{}.json", self.line_key(fn_source, line_number)))
    }

    /// Cache key for a whole-document translation (文档翻译). Folds the full file
    /// source + a "translate" discriminant into the model/prompt versions, so a
    /// translation entry never aliases a capsule/line entry, and editing the doc or
    /// switching the model invalidates it (same ADR-0003 semantics as capsules).
    pub fn translate_key(&self, source: &str) -> String {
        let mut hash = FNV_OFFSET;
        for part in [
            self.model_version.as_str(),
            self.prompt_version.as_str(),
            "translate",
            source,
        ] {
            // NUL separator so concatenation can't alias across fields.
            hash = fnv1a_step(hash, part.as_bytes());
            hash = fnv1a_step(hash, &[0]);
        }
        format!("{hash:016x}")
    }

    /// Look up a cached translation. Same miss/corrupt semantics as `get`: a missing
    /// or unreadable entry reads as absent → re-translate (reopen unchanged doc =
    /// zero token).
    pub fn get_translation(&self, source: &str) -> Option<Translation> {
        let path = self
            .dir
            .join(format!("{}.json", self.translate_key(source)));
        let bytes = std::fs::read(&path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Persist a translation under `.fluid/capsules/<translate_key>.json`. Creates the
    /// cache dir on demand. Never writes into the source tree (zero byte contamination).
    pub fn put_translation(&self, source: &str, t: &Translation) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let json = serde_json::to_vec_pretty(t)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(
            self.dir
                .join(format!("{}.json", self.translate_key(source))),
            json,
        )
    }

    /// Stable identity for one selection explanation. Unlike capsule entries, the
    /// provider/model are explicit request-snapshot inputs: a concurrent settings
    /// swap cannot make an old proxy response land under a new model's cache key.
    pub fn selection_key(
        &self,
        full_file_source: &str,
        start_byte: u64,
        end_byte: u64,
        provider_base_url: &str,
        model: &str,
        web_mode: bool,
    ) -> String {
        let mut hash = FNV_OFFSET;
        for part in [SELECTION_PROMPT_VERSION, full_file_source] {
            hash = fnv1a_step(hash, part.as_bytes());
            hash = fnv1a_step(hash, &[0]);
        }
        hash = fnv1a_step(hash, &start_byte.to_le_bytes());
        hash = fnv1a_step(hash, &[0]);
        hash = fnv1a_step(hash, &end_byte.to_le_bytes());
        hash = fnv1a_step(hash, &[0]);
        for part in [
            provider_base_url,
            model,
            self.prompt_version.as_str(),
            if web_mode { "web:on" } else { "web:off" },
        ] {
            hash = fnv1a_step(hash, part.as_bytes());
            hash = fnv1a_step(hash, &[0]);
        }
        format!("{hash:016x}")
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_selection(
        &self,
        full_file_source: &str,
        start_byte: u64,
        end_byte: u64,
        provider_base_url: &str,
        model: &str,
        web_mode: bool,
    ) -> Option<SelectionCacheEntry> {
        let key = self.selection_key(
            full_file_source,
            start_byte,
            end_byte,
            provider_base_url,
            model,
            web_mode,
        );
        let bytes = std::fs::read(self.selection_dir.join(format!("{key}.json"))).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn put_selection(
        &self,
        full_file_source: &str,
        start_byte: u64,
        end_byte: u64,
        provider_base_url: &str,
        model: &str,
        web_mode: bool,
        entry: &SelectionCacheEntry,
    ) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.selection_dir)?;
        let key = self.selection_key(
            full_file_source,
            start_byte,
            end_byte,
            provider_base_url,
            model,
            web_mode,
        );
        let json = serde_json::to_vec_pretty(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(self.selection_dir.join(format!("{key}.json")), json)
    }

    /// Read and revalidate one file-orientation artifact. Corrupt, stale, or
    /// structurally invalid entries are ordinary misses and never reach a child
    /// prompt.
    pub fn get_orientation(
        &self,
        identity: &OrientationCacheIdentity<'_>,
        context: &OrientationValidationContext<'_>,
    ) -> Option<FileOrientationCard> {
        if identity.full_file_source != context.source {
            return None;
        }
        let key = identity.key();
        let bytes = std::fs::read(self.orientation_dir.join(format!("{key}.json"))).ok()?;
        let card: FileOrientationCard = serde_json::from_slice(&bytes).ok()?;
        if card.orientation_id != key || card.schema_version != identity.schema_version {
            return None;
        }
        card.validate(context).ok()?;
        Some(card)
    }

    /// Validate and persist one file-orientation artifact under
    /// `.fluid/orientations/<orientationKey>.json`. Validation happens before
    /// directory creation, so a rejected model product leaves no cache residue.
    pub fn put_orientation(
        &self,
        identity: &OrientationCacheIdentity<'_>,
        context: &OrientationValidationContext<'_>,
        card: &FileOrientationCard,
    ) -> std::io::Result<()> {
        if identity.full_file_source != context.source {
            return Err(invalid_cache_data(
                "orientation identity source does not match validation source",
            ));
        }
        let key = identity.key();
        if card.orientation_id != key {
            return Err(invalid_cache_data(
                "orientationId does not match the orientation cache key",
            ));
        }
        if card.schema_version != identity.schema_version {
            return Err(invalid_cache_data(
                "card schemaVersion does not match the cache identity",
            ));
        }
        card.validate(context)
            .map_err(|error| invalid_cache_data(error.to_string()))?;

        std::fs::create_dir_all(&self.orientation_dir)?;
        let json = serde_json::to_vec_pretty(card).map_err(invalid_cache_data)?;
        std::fs::write(self.orientation_dir.join(format!("{key}.json")), json)
    }
}

fn invalid_cache_data(
    error: impl Into<Box<dyn std::error::Error + Send + Sync>>,
) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error)
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
/// Selection-only prompt/schema generation. Bump this without invalidating
/// capsules, line annotations, or translations that share `prompt_version`.
const SELECTION_PROMPT_VERSION: &str = "explain-selection-p3";

fn fnv1a_step(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_role(fn_id: &str) -> FunctionRole {
        FunctionRole {
            fn_id: fn_id.to_string(),
            lane: crate::orientation::FunctionLane::Core,
            flow_ids: vec!["request-flow".into()],
            stage: "dispatch request".into(),
            receives_from_actor_ids: vec!["caller".into()],
            consumes: vec!["Request".into()],
            sends_to_actor_ids: vec!["worker".into()],
            produces: vec!["Work".into()],
            why: "Moves a request into the worker.".into(),
            evidence_ids: vec!["E1".into()],
        }
    }

    fn sample_entry(fn_id: &str) -> CapsuleEntry {
        CapsuleEntry {
            capsule: Capsule {
                fn_id: fn_id.to_string(),
                signature: "def f(x): ...".to_string(),
                summary: "把 x 加一并返回".to_string(),
                complexity: "simple".to_string(),
                io: "x:int -> int".to_string(),
                orientation_id: "orientation-1".to_string(),
                role: sample_role(fn_id),
            },
            lines: vec![LineAnnotation {
                fn_id: fn_id.to_string(),
                line_number: 2,
                text: "返回 x+1".to_string(),
                color: "#7ee787".to_string(),
            }],
        }
    }

    fn store(dir: &Path) -> CacheStore {
        CacheStore::new(dir, "model-v1", "prompt-v1")
    }

    fn capsule_identity<'a>(
        fn_source: &'a str,
        orientation_context_hash: &'a str,
    ) -> CapsuleCacheIdentity<'a> {
        CapsuleCacheIdentity {
            fn_source,
            orientation_context_hash,
            provider_base_url: "https://provider.test/v1",
            model: "model-v1",
            prompt_version: CAPSULE_PROMPT_VERSION,
            schema_version: CAPSULE_SCHEMA_VERSION,
        }
    }

    #[test]
    fn capsule_identity_changes_only_when_source_coordinates_or_provider_contract_change() {
        let dir = tempdir_guard::TempDir::new();
        let cache = store(dir.path());
        let source = "def f(x):\n    return x + 1\n";
        let base = capsule_identity(source, "coordinates-v1");
        cache.put_capsule(&base, &sample_entry("f#1")).unwrap();

        let changed_source = CapsuleCacheIdentity {
            fn_source: "def f(x):\n    return x + 2\n",
            ..base
        };
        let changed_coordinates = CapsuleCacheIdentity {
            orientation_context_hash: "coordinates-v2",
            ..base
        };
        let changed_provider = CapsuleCacheIdentity {
            provider_base_url: "https://other-provider.test/v1",
            ..base
        };
        let changed_model = CapsuleCacheIdentity {
            model: "model-v2",
            ..base
        };
        let changed_prompt = CapsuleCacheIdentity {
            prompt_version: "capsule-p-next",
            ..base
        };
        let changed_schema = CapsuleCacheIdentity {
            schema_version: CAPSULE_SCHEMA_VERSION + 1,
            ..base
        };

        for changed in [
            changed_source,
            changed_coordinates,
            changed_provider,
            changed_model,
            changed_prompt,
            changed_schema,
        ] {
            assert_ne!(base.key(), changed.key());
            assert!(cache.get_capsule(&changed).is_none());
        }
    }

    #[test]
    fn same_function_and_normalized_coordinates_share_one_capsule_key() {
        let dir = tempdir_guard::TempDir::new();
        let cache = store(dir.path());
        let source = "def sibling():\n    return 1\n";
        let first_card = capsule_identity(source, "same-normalized-coordinates");
        let regenerated_card = capsule_identity(source, "same-normalized-coordinates");
        let entry = sample_entry("sibling#1");
        cache.put_capsule(&first_card, &entry).unwrap();

        assert_eq!(first_card.key(), regenerated_card.key());
        assert_eq!(cache.get_capsule(&regenerated_card), Some(entry));
    }

    #[test]
    fn put_then_get_hits_and_round_trips() {
        let dir = tempdir_guard::TempDir::new();
        let cache = store(dir.path());
        let src = "def f(x):\n    return x + 1\n";
        let entry = sample_entry("f#1");
        let identity = capsule_identity(src, "coordinates-v1");

        assert!(
            cache.get_capsule(&identity).is_none(),
            "cold cache must miss"
        );
        cache.put_capsule(&identity, &entry).unwrap();
        // Hit returns exactly what was stored (no downstream involved).
        assert_eq!(cache.get_capsule(&identity), Some(entry));
    }

    #[test]
    fn changed_fn_span_misses() {
        let dir = tempdir_guard::TempDir::new();
        let cache = store(dir.path());
        let src = "def f(x):\n    return x + 1\n";
        let identity = capsule_identity(src, "coordinates-v1");
        cache.put_capsule(&identity, &sample_entry("f#1")).unwrap();

        // A single edited byte in the function span → different key → miss.
        let edited = "def f(x):\n    return x + 2\n";
        let edited_identity = capsule_identity(edited, "coordinates-v1");
        assert_ne!(identity.key(), edited_identity.key());
        assert!(cache.get_capsule(&edited_identity).is_none());
    }

    #[test]
    fn model_or_prompt_version_change_invalidates() {
        let dir = tempdir_guard::TempDir::new();
        let src = "def f(x):\n    return x + 1\n";
        let cache = store(dir.path());
        let identity = capsule_identity(src, "coordinates-v1");
        cache.put_capsule(&identity, &sample_entry("f#1")).unwrap();

        let bumped_model = CapsuleCacheIdentity {
            model: "model-v2",
            ..identity
        };
        assert!(cache.get_capsule(&bumped_model).is_none());
        let bumped_prompt = CapsuleCacheIdentity {
            prompt_version: "capsule-p-next",
            ..identity
        };
        assert!(cache.get_capsule(&bumped_prompt).is_none());
    }

    #[test]
    fn writes_under_dot_fluid_and_leaves_source_untouched() {
        let dir = tempdir_guard::TempDir::new();
        // A source file outside the cache; its bytes/mtime must not change.
        let src_file = dir.path().join("a.py");
        std::fs::write(&src_file, "def f(x):\n    return x + 1\n").unwrap();
        let before = std::fs::metadata(&src_file).unwrap().modified().unwrap();

        let cache = store(dir.path());
        let src = "def f(x):\n    return x + 1\n";
        let identity = capsule_identity(src, "coordinates-v1");
        cache.put_capsule(&identity, &sample_entry("f#1")).unwrap();

        // Entry landed under .fluid/capsules/.
        let written = dir.path().join(".fluid").join("capsules");
        let entries: Vec<_> = std::fs::read_dir(&written)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].ends_with(".json"));

        // Source file untouched (zero byte contamination, 核心律 1).
        assert_eq!(
            std::fs::read(&src_file).unwrap(),
            b"def f(x):\n    return x + 1\n"
        );
        let after = std::fs::metadata(&src_file).unwrap().modified().unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn corrupt_entry_reads_as_miss() {
        let dir = tempdir_guard::TempDir::new();
        let cache = store(dir.path());
        let src = "def f(x):\n    return x + 1\n";
        let identity = capsule_identity(src, "coordinates-v1");
        // Hand-write garbage at the key's path.
        let cap_dir = dir.path().join(".fluid").join("capsules");
        std::fs::create_dir_all(&cap_dir).unwrap();
        std::fs::write(
            cap_dir.join(format!("{}.json", identity.key())),
            b"{ not json",
        )
        .unwrap();

        assert!(
            cache.get_capsule(&identity).is_none(),
            "corrupt entry must read as miss"
        );
    }

    fn sample_line(fn_id: &str, n: u32) -> LineAnnotation {
        LineAnnotation {
            fn_id: fn_id.to_string(),
            line_number: n,
            text: "把结果赋给 x".to_string(),
            color: "#f0883e".to_string(),
        }
    }

    #[test]
    fn line_put_then_get_hits_and_round_trips() {
        let dir = tempdir_guard::TempDir::new();
        let cache = store(dir.path());
        let src = "def f(x):\n    y = x + 1\n    return y\n";
        let line = sample_line("f#1", 2);

        assert!(
            cache.get_line(src, 2).is_none(),
            "cold line cache must miss"
        );
        cache.put_line(src, 2, &line).unwrap();
        assert_eq!(cache.get_line(src, 2), Some(line));
    }

    #[test]
    fn line_key_differs_from_capsule_key_and_per_line() {
        let dir = tempdir_guard::TempDir::new();
        let cache = store(dir.path());
        let src = "def f(x):\n    y = x + 1\n    return y\n";
        // A line key never aliases the function's capsule key for the same source.
        assert_ne!(
            cache.line_key(src, 2),
            capsule_identity(src, "coordinates-v1").key()
        );
        // Different target lines get distinct keys.
        assert_ne!(cache.line_key(src, 2), cache.line_key(src, 3));
    }

    #[test]
    fn line_cache_misses_on_changed_span_or_version() {
        let dir = tempdir_guard::TempDir::new();
        let src = "def f(x):\n    y = x + 1\n    return y\n";
        store(dir.path())
            .put_line(src, 2, &sample_line("f#1", 2))
            .unwrap();

        // Edited span → different line key → miss.
        let edited = "def f(x):\n    y = x + 2\n    return y\n";
        assert!(store(dir.path()).get_line(edited, 2).is_none());
        // Bumped model version → miss (same invalidation as capsules, ADR-0003).
        let bumped = CacheStore::new(dir.path(), "model-v2", "prompt-v1");
        assert!(bumped.get_line(src, 2).is_none());
    }

    #[test]
    fn translation_round_trips_and_misses_on_change() {
        let dir = tempdir_guard::TempDir::new();
        let cache = store(dir.path());
        let src = "# Title\n\nHello world.\n";
        let t = Translation {
            text: "# 标题\n\n你好世界。\n".to_string(),
        };

        assert!(
            cache.get_translation(src).is_none(),
            "cold translation cache must miss"
        );
        cache.put_translation(src, &t).unwrap();
        assert_eq!(cache.get_translation(src), Some(t));

        // Edited doc → different key → miss.
        let edited = "# Title\n\nHello, world!\n";
        assert_ne!(cache.translate_key(src), cache.translate_key(edited));
        assert!(cache.get_translation(edited).is_none());

        // Bumped model version → miss (ADR-0003), and a translate key never aliases
        // the capsule key for the same bytes.
        let bumped = CacheStore::new(dir.path(), "model-v2", "prompt-v1");
        assert!(bumped.get_translation(src).is_none());
        assert_ne!(
            cache.translate_key(src),
            capsule_identity(src, "coordinates-v1").key()
        );
    }

    fn sample_selection(meaning: &str) -> SelectionCacheEntry {
        SelectionCacheEntry {
            explanation: SelectionExplanation {
                selected_text: "from_str".into(),
                kind: SelectionKind::Function,
                meaning: meaning.into(),
                role_here: "把 JSON 文本解析为值".into(),
                origin: Some("serde_json".into()),
                evidence_status: EvidenceStatus::WebCited,
                sources: vec![SourceLink {
                    title: "serde_json docs".into(),
                    url: "https://docs.rs/serde_json".into(),
                }],
                warning: None,
            },
        }
    }

    #[test]
    fn selection_round_trips_and_force_style_put_overwrites_same_identity() {
        let dir = tempdir_guard::TempDir::new();
        let cache = store(dir.path());
        let source = "fn f() { serde_json::from_str(input); }\n";
        let identity = (10, 18, "https://provider.test/v1", "model-v1", true);

        assert!(cache
            .get_selection(source, identity.0, identity.1, identity.2, identity.3, identity.4,)
            .is_none());
        cache
            .put_selection(
                source,
                identity.0,
                identity.1,
                identity.2,
                identity.3,
                identity.4,
                &sample_selection("first"),
            )
            .unwrap();
        assert_eq!(
            cache
                .get_selection(source, identity.0, identity.1, identity.2, identity.3, identity.4,)
                .unwrap()
                .explanation
                .meaning,
            "first"
        );

        cache
            .put_selection(
                source,
                identity.0,
                identity.1,
                identity.2,
                identity.3,
                identity.4,
                &sample_selection("refreshed"),
            )
            .unwrap();
        assert_eq!(
            cache
                .get_selection(source, identity.0, identity.1, identity.2, identity.3, identity.4,)
                .unwrap()
                .explanation
                .meaning,
            "refreshed"
        );
    }

    #[test]
    fn selection_key_changes_with_every_frozen_identity_field() {
        let dir = tempdir_guard::TempDir::new();
        let cache = store(dir.path());
        let source = "fn f() { serde_json::from_str(input); }\n";
        let base = cache.selection_key(source, 10, 18, "https://p.test/v1", "model-v1", true);

        let changed = [
            cache.selection_key(
                "fn f() { serde_json::from_slice(input); }\n",
                10,
                18,
                "https://p.test/v1",
                "model-v1",
                true,
            ),
            cache.selection_key(source, 11, 18, "https://p.test/v1", "model-v1", true),
            cache.selection_key(source, 10, 19, "https://p.test/v1", "model-v1", true),
            cache.selection_key(source, 10, 18, "https://other.test/v1", "model-v1", true),
            cache.selection_key(source, 10, 18, "https://p.test/v1", "model-v2", true),
            cache.selection_key(source, 10, 18, "https://p.test/v1", "model-v1", false),
            CacheStore::new(dir.path(), "model-v1", "prompt-v2").selection_key(
                source,
                10,
                18,
                "https://p.test/v1",
                "model-v1",
                true,
            ),
        ];

        for key in changed {
            assert_ne!(base, key);
        }
    }

    /// Minimal self-cleaning temp dir (same pattern as project_reader's S1 tests;
    /// kept local so each test module stays self-contained).
    mod tempdir_guard {
        use std::path::{Path, PathBuf};

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new() -> Self {
                let unique = format!(
                    "fluid-cache-test-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                );
                let path = std::env::temp_dir().join(unique);
                std::fs::create_dir_all(&path).unwrap();
                TempDir(path)
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
