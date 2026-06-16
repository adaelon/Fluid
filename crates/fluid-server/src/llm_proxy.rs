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
    /// Build from an `LlmConfig` (U5a, ADR-0018). Returns `None` when the key is
    /// unset/empty — the server still runs, but `/api/generate` answers 503 on a
    /// cache miss instead of leaking a hard requirement into S1–S5 paths. This is
    /// the single construction path, used at startup and on every settings change.
    pub fn from_config(cfg: &crate::settings::LlmConfig) -> Option<Self> {
        if !cfg.key_set() {
            return None;
        }
        Some(Self {
            client: reqwest::Client::new(),
            base_url: cfg.base_url.clone(),
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
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

    /// Open a streaming chat completion (`stream: true`) and return the live
    /// `reqwest::Response` once headers are in and the status is success (S10a
    /// /api/query). The caller drives `resp.bytes_stream()` through an `SseDecoder`
    /// to pull content deltas. A non-2xx status is drained and turned into an error
    /// here, so the caller only ever streams a healthy body.
    pub async fn open_chat_stream(&self, system: &str, user: &str) -> anyhow::Result<reqwest::Response> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.model,
            "temperature": 0.2,
            "stream": true,
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
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("LLM HTTP {status}: {text}");
        }
        Ok(resp)
    }
}

/// Incremental decoder for an OpenAI-compatible SSE stream. The byte stream is
/// chunked arbitrarily (a chunk may split a line mid-way), so `push` buffers a
/// trailing partial line and only emits content deltas for *complete* `data:`
/// lines. The `[DONE]` sentinel and role-only/empty deltas yield nothing.
#[derive(Default)]
pub struct SseDecoder {
    buf: String,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a raw text chunk and return the content deltas of any lines that are
    /// now complete (ended by a newline). A trailing partial line stays buffered.
    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        self.buf.push_str(chunk);
        let mut out = Vec::new();
        while let Some(nl) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=nl).collect();
            let line = line.trim();
            let Some(data) = line.strip_prefix("data:") else {
                continue; // SSE comment / blank separator / event: line
            };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if let Some(delta) = parse_chunk_delta(data) {
                if !delta.is_empty() {
                    out.push(delta);
                }
            }
        }
        out
    }
}

/// Pull `choices[0].delta.content` out of one SSE `data:` JSON payload, if present.
fn parse_chunk_delta(data: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(data).ok()?;
    v["choices"][0]["delta"]["content"]
        .as_str()
        .map(|s| s.to_string())
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

#[derive(Deserialize)]
struct RawFetchPlan {
    #[serde(default)]
    need: Vec<String>,
}

/// Parse the phase-1 planning reply of on-demand fetch (S10a-追源, ADR-0017) into
/// the list of function names the model wants source for. Tolerates fences/prose
/// like the other parsers; **any** failure (bad JSON, missing field) yields an
/// empty list — the caller then simply answers over the degraded context, so a
/// malformed plan can never fail the query.
pub fn parse_fetch_plan(content: &str) -> Vec<String> {
    serde_json::from_str::<RawFetchPlan>(extract_json(content))
        .map(|p| p.need)
        .unwrap_or_default()
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

    #[test]
    fn sse_decoder_extracts_content_deltas_in_order() {
        let mut d = SseDecoder::new();
        let out = d.push(
            "data: {\"choices\":[{\"delta\":{\"content\":\"你\"}}]}\n\
             data: {\"choices\":[{\"delta\":{\"content\":\"好\"}}]}\n",
        );
        assert_eq!(out, vec!["你".to_string(), "好".to_string()]);
    }

    #[test]
    fn sse_decoder_buffers_a_partial_line_across_pushes() {
        let mut d = SseDecoder::new();
        // First chunk cuts the JSON line in half — nothing complete yet.
        assert!(d.push("data: {\"choices\":[{\"delta\":{\"con").is_empty());
        // Second chunk completes the line.
        let out = d.push("tent\":\"x\"}}]}\n");
        assert_eq!(out, vec!["x".to_string()]);
    }

    #[test]
    fn sse_decoder_skips_done_sentinel_and_role_only_delta() {
        let mut d = SseDecoder::new();
        let out = d.push(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\
             data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\
             data: [DONE]\n",
        );
        assert_eq!(out, vec!["hi".to_string()]);
    }

    #[test]
    fn sse_decoder_ignores_blank_lines_and_comments() {
        let mut d = SseDecoder::new();
        let out = d.push(": keep-alive\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n");
        assert_eq!(out, vec!["a".to_string()]);
    }

    // — S10a-追源 plan parsing (ADR-0017) —

    #[test]
    fn parse_fetch_plan_reads_need_list_tolerating_prose() {
        let need = parse_fetch_plan("好的：{\"need\":[\"save\",\"verify\"]} 完毕");
        assert_eq!(need, vec!["save".to_string(), "verify".to_string()]);
    }

    #[test]
    fn parse_fetch_plan_empty_when_none_needed() {
        assert!(parse_fetch_plan("{\"need\":[]}").is_empty());
    }

    #[test]
    fn parse_fetch_plan_bad_json_is_empty_not_panic() {
        assert!(parse_fetch_plan("我不需要任何源码").is_empty());
        assert!(parse_fetch_plan("{\"other\":1}").is_empty()); // missing field → default empty
    }
}
