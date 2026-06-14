//! CacheStore — on-disk bypass cache for generated capsules (S5).
//!
//! Bypass cache = generated function capsules + line annotations, written
//! *outside* the source tree so the core law "zero byte contamination" holds
//! (核心律 1). Layout: `<project>/.fluid/capsules/<key>.json`.
//!
//! Key (技术方案 §6, refining ADR-0003): `hash(function source span)` folded
//! together with the model version and prompt version. Changing any of the three
//! changes the key → cache miss → recompute. Per-function granularity means
//! editing one function only invalidates that function's entry; sibling
//! functions in the same file stay hot.
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

/// A function-granularity semantic capsule (技术方案 §3). S5 stores these
/// verbatim; S6 will populate them from the LLM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capsule {
    #[serde(rename = "fnId")]
    pub fn_id: String,
    pub signature: String,
    pub summary: String,
    pub complexity: String,
    pub io: String,
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

/// On-disk bypass cache rooted at `<project>/.fluid/capsules/`.
pub struct CacheStore {
    dir: PathBuf,
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
            model_version: model_version.into(),
            prompt_version: prompt_version.into(),
        }
    }

    /// Cache key for a function: stable hex of FNV-1a over
    /// (model_version, prompt_version, function source span). Public so S6 can
    /// reuse it and tests can assert miss-on-change.
    pub fn key(&self, fn_source: &str) -> String {
        let mut hash = FNV_OFFSET;
        for part in [
            self.model_version.as_str(),
            self.prompt_version.as_str(),
            fn_source,
        ] {
            // NUL separator so concatenation can't alias across fields.
            hash = fnv1a_step(hash, part.as_bytes());
            hash = fnv1a_step(hash, &[0]);
        }
        format!("{hash:016x}")
    }

    /// Look up a function's cached entry. Returns `None` on miss or on any read/
    /// parse error (a corrupt entry is treated as absent → recompute). Reads a
    /// file only; never touches any downstream generator.
    pub fn get(&self, fn_source: &str) -> Option<CapsuleEntry> {
        let path = self.path_for(fn_source);
        let bytes = std::fs::read(&path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Persist a function's entry under `.fluid/capsules/<key>.json`. Creates the
    /// cache directory on demand. Never writes into the source tree.
    pub fn put(&self, fn_source: &str, entry: &CapsuleEntry) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let json = serde_json::to_vec_pretty(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(self.path_for(fn_source), json)
    }

    fn path_for(&self, fn_source: &str) -> PathBuf {
        self.dir.join(format!("{}.json", self.key(fn_source)))
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

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

    fn sample_entry(fn_id: &str) -> CapsuleEntry {
        CapsuleEntry {
            capsule: Capsule {
                fn_id: fn_id.to_string(),
                signature: "def f(x): ...".to_string(),
                summary: "把 x 加一并返回".to_string(),
                complexity: "simple".to_string(),
                io: "x:int -> int".to_string(),
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

    #[test]
    fn put_then_get_hits_and_round_trips() {
        let dir = tempdir_guard::TempDir::new();
        let cache = store(dir.path());
        let src = "def f(x):\n    return x + 1\n";
        let entry = sample_entry("f#1");

        assert!(cache.get(src).is_none(), "cold cache must miss");
        cache.put(src, &entry).unwrap();
        // Hit returns exactly what was stored (no downstream involved).
        assert_eq!(cache.get(src), Some(entry));
    }

    #[test]
    fn changed_fn_span_misses() {
        let dir = tempdir_guard::TempDir::new();
        let cache = store(dir.path());
        let src = "def f(x):\n    return x + 1\n";
        cache.put(src, &sample_entry("f#1")).unwrap();

        // A single edited byte in the function span → different key → miss.
        let edited = "def f(x):\n    return x + 2\n";
        assert_ne!(cache.key(src), cache.key(edited));
        assert!(cache.get(edited).is_none());
    }

    #[test]
    fn model_or_prompt_version_change_invalidates() {
        let dir = tempdir_guard::TempDir::new();
        let src = "def f(x):\n    return x + 1\n";
        store(dir.path()).put(src, &sample_entry("f#1")).unwrap();

        // Same source, bumped model version → miss (ADR-0003: model bump 失效).
        let bumped_model = CacheStore::new(dir.path(), "model-v2", "prompt-v1");
        assert!(bumped_model.get(src).is_none());
        // Same source, bumped prompt version → miss.
        let bumped_prompt = CacheStore::new(dir.path(), "model-v1", "prompt-v2");
        assert!(bumped_prompt.get(src).is_none());
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
        cache.put(src, &sample_entry("f#1")).unwrap();

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
        // Hand-write garbage at the key's path.
        let cap_dir = dir.path().join(".fluid").join("capsules");
        std::fs::create_dir_all(&cap_dir).unwrap();
        std::fs::write(cap_dir.join(format!("{}.json", cache.key(src))), b"{ not json").unwrap();

        assert!(cache.get(src).is_none(), "corrupt entry must read as miss");
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
