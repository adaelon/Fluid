//! LLM backend settings (U5a, ADR-0018).
//!
//! Holds the runtime-editable config (base_url / model / api_key), the key
//! masking used for write-only display (the full key is never sent to the
//! frontend), and the pure `.env` write-back used to persist a change.

use crate::llm_proxy::{DEFAULT_BASE_URL, DEFAULT_MODEL};

/// The three values that define which LLM backend Fluid talks to. `api_key` is a
/// secret: it lives in memory + the gitignored `.env`, and is *never* serialized
/// out to the frontend (see `mask_key` for the only thing the UI ever sees).
#[derive(Clone)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

impl LlmConfig {
    /// Read the three values from the process env (`.env` already loaded by main).
    /// Missing/empty base_url and model fall back to the built-in defaults; an
    /// absent key yields an empty string (proxy stays disabled until configured).
    pub fn from_env() -> Self {
        let nonempty = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
        Self {
            base_url: nonempty("OPENCODE_BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            model: nonempty("FLUID_LLM_MODEL").unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            api_key: std::env::var("OPENCODE_API_KEY").ok().unwrap_or_default(),
        }
    }

    /// Whether a usable key is set (a configured proxy implies this is true).
    pub fn key_set(&self) -> bool {
        !self.api_key.trim().is_empty()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
