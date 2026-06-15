//! LLMProxy — the only component that goes out to the network (ADR-0010).
//!
//! Holds the API key (never sent to the frontend, never written to `.fluid/`,
//! never committed) and talks to an OpenAI-compatible chat-completions endpoint.
//! Default target is the opencode "zen" gateway serving glm-5.1; both the base
//! URL and model are env-overridable, the key is env-only (no default → no secret
//! in source).
//!
//! Config (S6 decision, recorded in docs/代码链路.md):
//! - `OPENCODE_API_KEY`   (required; absent → proxy disabled, /api/generate 503)
//! - `OPENCODE_BASE_URL`  (default `https://opencode.ai/zen/go/v1`)
//! - `FLUID_LLM_MODEL`    (default `glm-5.1`; passed in from main so the cache
//!   model_version stays in lock-step with the model actually queried)

use serde::Deserialize;

use crate::cache_store::{Capsule, LineAnnotation};

pub const DEFAULT_BASE_URL: &str = "https://opencode.ai/zen/go/v1";
pub const DEFAULT_MODEL: &str = "glm-5.1";

pub struct LlmProxy {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    pub model: String,
}

impl LlmProxy {
    /// Build from environment. Returns `None` when `OPENCODE_API_KEY` is unset/
    /// empty — the server still runs, but `/api/generate` answers 503 on a cache
    /// miss instead of leaking a hard requirement into S1–S5 paths.
    pub fn from_env(model: impl Into<String>) -> Option<Self> {
        let api_key = std::env::var("OPENCODE_API_KEY")
            .ok()
            .filter(|s| !s.is_empty())?;
        let base_url =
            std::env::var("OPENCODE_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        Some(Self {
            client: reqwest::Client::new(),
            base_url,
            api_key,
            model: model.into(),
        })
    }

    /// One non-streaming chat completion; returns the assistant message content.
    pub async fn complete(&self, system: &str, user: &str) -> anyhow::Result<String> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.model,
            "temperature": 0.2,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("LLM HTTP {status}: {text}");
        }

        let parsed: ChatResponse = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("unparseable LLM response: {e}; body: {text}"))?;
        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow::anyhow!("LLM returned no choices"))
    }
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}
#[derive(Deserialize)]
struct Choice {
    message: ChatMessage,
}
#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

// — Parsing the model's JSON into our domain types —

#[derive(Deserialize)]
struct RawCapsule {
    #[serde(default)]
    signature: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    complexity: String,
    #[serde(default)]
    io: String,
}

#[derive(Deserialize)]
struct RawLine {
    #[serde(rename = "lineNumber")]
    line_number: u32,
    #[serde(default)]
    text: String,
    #[serde(default)]
    color: String,
}

#[derive(Deserialize)]
struct RawGeneration {
    capsule: RawCapsule,
    #[serde(default)]
    lines: Vec<RawLine>,
}

const DEFAULT_LINE_COLOR: &str = "#7ee787";

/// Parse the model's content into a `(Capsule, Vec<LineAnnotation>)`. Tolerates
/// markdown code fences / surrounding prose; `fn_id` is injected by us (the model
/// is not asked to echo it). A missing line color defaults to the neutral tone.
pub fn parse_generation(content: &str, fn_id: &str) -> anyhow::Result<(Capsule, Vec<LineAnnotation>)> {
    let json = extract_json(content);
    let raw: RawGeneration = serde_json::from_str(json)
        .map_err(|e| anyhow::anyhow!("LLM did not return the expected JSON: {e}; content: {content}"))?;

    let capsule = Capsule {
        fn_id: fn_id.to_string(),
        signature: raw.capsule.signature,
        summary: raw.capsule.summary,
        complexity: raw.capsule.complexity,
        io: raw.capsule.io,
    };
    let lines = raw
        .lines
        .into_iter()
        .map(|l| LineAnnotation {
            fn_id: fn_id.to_string(),
            line_number: l.line_number,
            text: l.text,
            color: if l.color.trim().is_empty() {
                DEFAULT_LINE_COLOR.to_string()
            } else {
                l.color
            },
        })
        .collect();

    Ok((capsule, lines))
}

#[derive(Deserialize)]
struct RawLineAnnotation {
    #[serde(default)]
    text: String,
    #[serde(default)]
    color: String,
}

/// Parse the model's reply for a single manual-line explanation (S9) into one
/// `LineAnnotation`. The model returns only `{text, color}`; `fn_id` and
/// `line_number` are injected by us. Tolerates fences/prose like `parse_generation`;
/// an empty `text` is an error (no point caching a blank annotation), a missing
/// color defaults to the neutral tone.
pub fn parse_line_annotation(
    content: &str,
    fn_id: &str,
    line_number: u32,
) -> anyhow::Result<LineAnnotation> {
    let json = extract_json(content);
    let raw: RawLineAnnotation = serde_json::from_str(json)
        .map_err(|e| anyhow::anyhow!("LLM did not return the expected JSON: {e}; content: {content}"))?;
    if raw.text.trim().is_empty() {
        anyhow::bail!("LLM returned an empty line annotation; content: {content}");
    }
    Ok(LineAnnotation {
        fn_id: fn_id.to_string(),
        line_number,
        text: raw.text,
        color: if raw.color.trim().is_empty() {
            DEFAULT_LINE_COLOR.to_string()
        } else {
            raw.color
        },
    })
}

/// Pull the JSON object out of the model's reply: strips a leading ```/```json
/// fence if present, otherwise takes the span from the first `{` to the last `}`.
fn extract_json(content: &str) -> &str {
    let s = content.trim();
    if let Some(rest) = s.strip_prefix("```") {
        let rest = rest.trim_start_matches("json").trim_start();
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim();
        }
    }
    if let (Some(a), Some(b)) = (s.find('{'), s.rfind('}')) {
        if b >= a {
            return s[a..=b].trim();
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_json_and_injects_fn_id() {
        let content = r##"{"capsule":{"signature":"def f(x)","summary":"加一","complexity":"simple","io":"x:int->int"},"lines":[{"lineNumber":2,"text":"返回 x+1","color":"#abcdef"}]}"##;
        let (cap, lines) = parse_generation(content, "f#1").unwrap();
        assert_eq!(cap.fn_id, "f#1");
        assert_eq!(cap.summary, "加一");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].fn_id, "f#1");
        assert_eq!(lines[0].line_number, 2);
        assert_eq!(lines[0].color, "#abcdef");
    }

    #[test]
    fn strips_markdown_code_fence() {
        let content = "```json\n{\"capsule\":{\"signature\":\"\",\"summary\":\"s\",\"complexity\":\"simple\",\"io\":\"\"},\"lines\":[]}\n```";
        let (cap, lines) = parse_generation(content, "g#5").unwrap();
        assert_eq!(cap.summary, "s");
        assert!(lines.is_empty());
    }

    #[test]
    fn tolerates_surrounding_prose() {
        let content = "好的，结果如下：{\"capsule\":{\"summary\":\"x\"},\"lines\":[]} 完毕";
        let (cap, _) = parse_generation(content, "h#1").unwrap();
        assert_eq!(cap.summary, "x");
    }

    #[test]
    fn missing_line_color_defaults() {
        let content = r#"{"capsule":{"summary":"s"},"lines":[{"lineNumber":3,"text":"t"}]}"#;
        let (_, lines) = parse_generation(content, "f#1").unwrap();
        assert_eq!(lines[0].color, DEFAULT_LINE_COLOR);
    }

    #[test]
    fn non_json_is_an_error_not_a_panic() {
        assert!(parse_generation("抱歉我无法完成", "f#1").is_err());
    }

    #[test]
    fn parses_single_line_annotation_and_injects_fn_id_and_line() {
        let content = r##"{"text":"把 x+1 赋给 y","color":"#f0883e"}"##;
        let line = parse_line_annotation(content, "f#1", 12).unwrap();
        assert_eq!(line.fn_id, "f#1");
        assert_eq!(line.line_number, 12);
        assert_eq!(line.text, "把 x+1 赋给 y");
        assert_eq!(line.color, "#f0883e");
    }

    #[test]
    fn line_annotation_missing_color_defaults() {
        let content = r#"包裹一下：{"text":"返回结果"} 完"#;
        let line = parse_line_annotation(content, "g#3", 5).unwrap();
        assert_eq!(line.text, "返回结果");
        assert_eq!(line.color, DEFAULT_LINE_COLOR);
    }

    #[test]
    fn empty_line_text_is_an_error() {
        let content = r##"{"text":"   ","color":"#7ee787"}"##;
        assert!(parse_line_annotation(content, "f#1", 2).is_err());
    }

    #[test]
    fn non_json_line_is_an_error_not_a_panic() {
        assert!(parse_line_annotation("我不知道", "f#1", 2).is_err());
    }
}
