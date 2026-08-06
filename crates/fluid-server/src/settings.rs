//! LLM backend settings (U5a, ADR-0018).
//!
//! Holds the runtime-editable config (base_url / model / api_key), the key
//! masking used for write-only display (the full key is never sent to the
//! frontend), and the pure `.env` write-back used to persist a change.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use crate::llm_proxy::{DEFAULT_BASE_URL, DEFAULT_MODEL};

const API_KEY_ENV: &str = "OPENCODE_API_KEY";
const BASE_URL_ENV: &str = "OPENCODE_BASE_URL";
const MODEL_ENV: &str = "FLUID_LLM_MODEL";

/// The three values that define which LLM backend Fluid talks to. `api_key` is a
/// secret: it lives in memory plus the platform config file/process environment,
/// and is *never* serialized out to the frontend (see `mask_key` for the only
/// thing the UI ever sees).
#[derive(Clone)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

impl LlmConfig {
    /// Read only the process environment. On non-Windows, main may have loaded a
    /// discovered `.env` first; Windows uses `from_env_file` for its fixed path.
    /// Missing/empty base_url and model fall back to the built-in defaults.
    pub fn from_env() -> Self {
        let nonempty = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
        Self {
            base_url: nonempty(BASE_URL_ENV).unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            model: nonempty(MODEL_ENV).unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            api_key: std::env::var(API_KEY_ENV).ok().unwrap_or_default(),
        }
    }

    /// Read a specific `.env` file without mutating the process environment.
    /// A process variable wins whenever it is explicitly present, including an
    /// explicitly empty value; the file is only consulted when that variable is
    /// absent. Missing files are equivalent to an empty file.
    pub fn from_env_file(path: &Path) -> Result<Self, dotenvy::Error> {
        let file_values = match dotenvy::from_path_iter(path) {
            Ok(iter) => iter.collect::<Result<HashMap<_, _>, _>>()?,
            Err(error) if error.not_found() => HashMap::new(),
            Err(error) => return Err(error),
        };
        Ok(Self::from_sources(
            |key| std::env::var(key).ok(),
            &file_values,
        ))
    }

    fn from_sources(
        env: impl Fn(&str) -> Option<String>,
        file_values: &HashMap<String, String>,
    ) -> Self {
        let value = |key: &str| match env(key) {
            Some(explicit) => Some(explicit),
            None => file_values.get(key).cloned(),
        };
        Self {
            base_url: value(BASE_URL_ENV)
                .filter(|item| !item.is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            model: value(MODEL_ENV)
                .filter(|item| !item.is_empty())
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            api_key: value(API_KEY_ENV).unwrap_or_default(),
        }
    }

    /// Whether a usable key is set (a configured proxy implies this is true).
    pub fn key_set(&self) -> bool {
        !self.api_key.trim().is_empty()
    }
}

/// Resolve the fixed Windows config location from `%LOCALAPPDATA%`.
/// Kept parameterized so the path contract is covered on every test platform.
pub fn windows_env_path(local_app_data: Option<&OsStr>) -> io::Result<PathBuf> {
    let Some(base) = local_app_data.filter(|value| !value.is_empty()) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "LOCALAPPDATA is not set; cannot resolve Fluid config path",
        ));
    };
    Ok(PathBuf::from(base).join("Fluid").join(".env"))
}

/// The masked key hint shown to the frontend: `···` + the last 4 chars, or `None`
/// when no key is set. This is the *only* derivative of the key that ever leaves
/// the backend (write-only key, ADR-0018).
pub fn mask_key(key: &str) -> Option<String> {
    let k = key.trim();
    if k.is_empty() {
        return None;
    }
    let tail: Vec<char> = k.chars().rev().take(4).collect();
    let last4: String = tail.into_iter().rev().collect();
    Some(format!("···{last4}"))
}

/// Pure: produce new `.env` text with the three LLM lines set to `cfg`, updating
/// any existing `KEY=...` line in place and appending the ones that are missing,
/// leaving every other line / comment untouched and in order. Idempotent.
pub fn rewrite_env(existing: &str, cfg: &LlmConfig) -> String {
    let wanted: [(&str, &str); 3] = [
        ("OPENCODE_API_KEY", cfg.api_key.as_str()),
        ("OPENCODE_BASE_URL", cfg.base_url.as_str()),
        ("FLUID_LLM_MODEL", cfg.model.as_str()),
    ];
    let mut seen = [false; 3];
    let mut out: Vec<String> = Vec::new();

    for line in existing.lines() {
        let trimmed = line.trim_start();
        let mut replaced = false;
        for (i, (k, v)) in wanted.iter().enumerate() {
            // Match `KEY=` at the start of the (left-trimmed) line, so commented
            // lines like `# OPENCODE_API_KEY=...` are left alone.
            if let Some(rest) = trimmed.strip_prefix(k) {
                if rest.starts_with('=') {
                    out.push(format!("{k}={v}"));
                    seen[i] = true;
                    replaced = true;
                    break;
                }
            }
        }
        if !replaced {
            out.push(line.to_string());
        }
    }
    for (i, (k, v)) in wanted.iter().enumerate() {
        if !seen[i] {
            out.push(format!("{k}={v}"));
        }
    }

    let mut result = out.join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    result
}

/// Persist the runtime settings to the selected `.env`, creating its parent
/// directory on first save. Existing unrelated lines and comments are retained.
pub fn persist_env(path: &Path, cfg: &LlmConfig) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let existing = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    std::fs::write(path, rewrite_env(&existing, cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_values(items: &[(&str, &str)]) -> HashMap<String, String> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn cfg(base: &str, model: &str, key: &str) -> LlmConfig {
        LlmConfig {
            base_url: base.into(),
            model: model.into(),
            api_key: key.into(),
        }
    }

    #[test]
    fn mask_key_shows_last_four_or_none() {
        assert_eq!(mask_key(""), None);
        assert_eq!(mask_key("   "), None);
        assert_eq!(mask_key("sk-abcd1234").as_deref(), Some("···1234"));
        // Shorter than 4 → shows what's there.
        assert_eq!(mask_key("ab").as_deref(), Some("···ab"));
    }

    #[test]
    fn windows_config_path_is_fixed_below_local_app_data() {
        let base = PathBuf::from("local-app-data");
        assert_eq!(
            windows_env_path(Some(base.as_os_str())).unwrap(),
            base.join("Fluid").join(".env")
        );
        assert_eq!(
            windows_env_path(None).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn explicit_environment_values_override_the_fixed_file() {
        let file = file_values(&[
            (API_KEY_ENV, "file-key"),
            (BASE_URL_ENV, "https://file/v1"),
            (MODEL_ENV, "file-model"),
        ]);
        let cfg = LlmConfig::from_sources(
            |key| match key {
                API_KEY_ENV => Some("env-key".into()),
                BASE_URL_ENV => Some("https://env/v1".into()),
                MODEL_ENV => Some("env-model".into()),
                _ => None,
            },
            &file,
        );
        assert_eq!(cfg.api_key, "env-key");
        assert_eq!(cfg.base_url, "https://env/v1");
        assert_eq!(cfg.model, "env-model");

        // An explicitly empty process value still suppresses the file. Base/model
        // then use built-in defaults; the key remains unset.
        let empty = LlmConfig::from_sources(|_| Some(String::new()), &file);
        assert_eq!(empty.api_key, "");
        assert_eq!(empty.base_url, DEFAULT_BASE_URL);
        assert_eq!(empty.model, DEFAULT_MODEL);
    }

    #[test]
    fn fixed_file_values_are_used_when_environment_is_absent() {
        let file = file_values(&[
            (API_KEY_ENV, "file-key"),
            (BASE_URL_ENV, "https://file/v1"),
            (MODEL_ENV, "file-model"),
        ]);
        let cfg = LlmConfig::from_sources(|_| None, &file);
        assert_eq!(cfg.api_key, "file-key");
        assert_eq!(cfg.base_url, "https://file/v1");
        assert_eq!(cfg.model, "file-model");
    }

    #[test]
    fn persist_env_creates_the_fixed_directory_on_first_save() {
        let root = std::env::temp_dir().join(format!(
            "fluid-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("Fluid").join(".env");
        persist_env(&path, &cfg("https://b/v1", "m", "k")).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("OPENCODE_API_KEY=k\n"));
        assert!(written.contains("OPENCODE_BASE_URL=https://b/v1\n"));
        assert!(written.contains("FLUID_LLM_MODEL=m\n"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rewrite_env_updates_existing_lines_in_place() {
        let existing = "# comment\nOPENCODE_API_KEY=old\nOPENCODE_BASE_URL=https://a/v1\nFLUID_LLM_MODEL=glm-5.1\n";
        let out = rewrite_env(existing, &cfg("https://b/v1", "gpt-4o", "new"));
        assert_eq!(
            out,
            "# comment\nOPENCODE_API_KEY=new\nOPENCODE_BASE_URL=https://b/v1\nFLUID_LLM_MODEL=gpt-4o\n"
        );
    }

    #[test]
    fn rewrite_env_appends_missing_lines_and_keeps_others() {
        let existing = "# just a note\nSOMETHING_ELSE=keepme\n";
        let out = rewrite_env(existing, &cfg("https://b/v1", "gpt-4o", "k"));
        // Untouched lines stay first, in order; the three are appended.
        assert!(out.starts_with("# just a note\nSOMETHING_ELSE=keepme\n"));
        assert!(out.contains("\nOPENCODE_API_KEY=k\n"));
        assert!(out.contains("\nOPENCODE_BASE_URL=https://b/v1\n"));
        assert!(out.ends_with("FLUID_LLM_MODEL=gpt-4o\n"));
        assert!(out.contains("SOMETHING_ELSE=keepme"));
    }

    #[test]
    fn rewrite_env_leaves_commented_keys_alone() {
        let existing = "# OPENCODE_API_KEY=donttouch\n";
        let out = rewrite_env(existing, &cfg("https://b/v1", "m", "real"));
        // The comment is preserved verbatim; a real line is appended.
        assert!(out.contains("# OPENCODE_API_KEY=donttouch"));
        assert!(out.contains("\nOPENCODE_API_KEY=real\n"));
    }

    #[test]
    fn rewrite_env_from_empty_is_just_the_three_lines() {
        let out = rewrite_env("", &cfg("https://b/v1", "m", "k"));
        assert_eq!(
            out,
            "OPENCODE_API_KEY=k\nOPENCODE_BASE_URL=https://b/v1\nFLUID_LLM_MODEL=m\n"
        );
    }
}
