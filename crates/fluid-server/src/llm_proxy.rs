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

use std::time::Duration;

use serde::Deserialize;

use crate::cache_store::{Capsule, LineAnnotation, SelectionExplanation, SelectionKind};
use crate::orientation::{
    CodeEvidenceRef, FileOrientationCard, FunctionRole, OrientationActor, OrientationCoverage,
    OrientationFlow, OrientationInvariant, OrientationType, OrientationWalkthrough,
    SupportingCapability, ORIENTATION_SCHEMA_VERSION,
};
use crate::web_evidence::{
    parse_web_search_response, EvidenceStatus, SourceLink, WebSearchError, WebSearchResult,
};

pub const DEFAULT_BASE_URL: &str = "https://opencode.ai/zen/go/v1";
pub const DEFAULT_MODEL: &str = "glm-5.1";
const WEB_SEARCH_TIMEOUT: Duration = Duration::from_secs(60);

pub struct LlmProxy {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    pub model: String,
    // S-WEB-2 will make the S-WEB-1 search adapter reachable from a route.
    #[allow(dead_code)]
    web_search_timeout: Duration,
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
            web_search_timeout: WEB_SEARCH_TIMEOUT,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_config_with_web_search_timeout(
        cfg: &crate::settings::LlmConfig,
        web_search_timeout: Duration,
    ) -> Option<Self> {
        let mut proxy = Self::from_config(cfg)?;
        proxy.web_search_timeout = web_search_timeout;
        Some(proxy)
    }

    /// Run one non-streaming supplier-hosted web search through the Responses
    /// API. `query` is the only business input: no source code, file path, or
    /// private context is accepted by this boundary.
    pub async fn responses_web_search(
        &self,
        query: &str,
    ) -> Result<WebSearchResult, WebSearchError> {
        let url = format!("{}/responses", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.model,
            "input": query,
            "tools": [{ "type": "web_search" }],
            "tool_choice": { "type": "web_search" },
        });

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .timeout(self.web_search_timeout)
            .send()
            .await
            .map_err(classify_transport_error)?;

        let status = response.status();
        let text = response.text().await.map_err(classify_transport_error)?;
        if !status.is_success() {
            return Err(classify_web_search_status(status.as_u16(), &text));
        }

        parse_web_search_response(&text)
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
    pub async fn open_chat_stream(
        &self,
        system: &str,
        user: &str,
    ) -> anyhow::Result<reqwest::Response> {
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

fn classify_transport_error(error: reqwest::Error) -> WebSearchError {
    if error.is_timeout() {
        WebSearchError::Timeout(error.to_string())
    } else {
        WebSearchError::Transport(error.to_string())
    }
}

fn classify_web_search_status(status: u16, body: &str) -> WebSearchError {
    let message = provider_error_message(body);
    match status {
        401 | 403 => WebSearchError::Authentication { status, message },
        404 | 405 | 501 => WebSearchError::Unsupported { status, message },
        429 => WebSearchError::RateLimited { status, message },
        400 | 422 if indicates_unsupported_tool(body) || indicates_unsupported_tool(&message) => {
            WebSearchError::Unsupported { status, message }
        }
        _ => WebSearchError::Provider { status, message },
    }
}

fn provider_error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| {
            let body = body.trim();
            if body.is_empty() {
                "empty response body".to_string()
            } else {
                body.to_string()
            }
        })
}

fn indicates_unsupported_tool(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    let names_web_search = message.contains("web_search") || message.contains("web search");
    let names_tool = names_web_search || message.contains("tool");
    names_tool
        && (message.contains("unsupported")
            || message.contains("not supported")
            || message.contains("does not support"))
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
pub fn parse_generation(
    content: &str,
    fn_id: &str,
) -> anyhow::Result<(Capsule, Vec<LineAnnotation>)> {
    let json = extract_json(content);
    let raw: RawGeneration = serde_json::from_str(json).map_err(|e| {
        anyhow::anyhow!("LLM did not return the expected JSON: {e}; content: {content}")
    })?;

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
#[serde(rename_all = "camelCase")]
struct RawOrientationCard {
    purpose: String,
    actors: Vec<OrientationActor>,
    #[serde(default)]
    types: Vec<OrientationType>,
    core_flows: Vec<OrientationFlow>,
    #[serde(default)]
    supporting_capabilities: Vec<SupportingCapability>,
    #[serde(default)]
    function_roles: Vec<FunctionRole>,
    walkthrough: OrientationWalkthrough,
    #[serde(default)]
    invariants: Vec<OrientationInvariant>,
    evidence: Vec<CodeEvidenceRef>,
}

/// Parse the model-authored semantic portion of an orientation card. Cache,
/// schema, file identity, and full/bounded coverage are backend facts, so the
/// model never gets to choose or echo them into the trusted artifact.
pub fn parse_orientation_card(
    content: &str,
    orientation_id: &str,
    file_path: &str,
    coverage: OrientationCoverage,
) -> anyhow::Result<FileOrientationCard> {
    let raw: RawOrientationCard = serde_json::from_str(extract_json(content)).map_err(|error| {
        anyhow::anyhow!(
            "LLM did not return the expected orientation JSON: {error}; content: {content}"
        )
    })?;

    Ok(FileOrientationCard {
        schema_version: ORIENTATION_SCHEMA_VERSION,
        orientation_id: orientation_id.to_string(),
        file_path: file_path.to_string(),
        purpose: raw.purpose,
        actors: raw.actors,
        types: raw.types,
        core_flows: raw.core_flows,
        supporting_capabilities: raw.supporting_capabilities,
        function_roles: raw.function_roles,
        walkthrough: raw.walkthrough,
        invariants: raw.invariants,
        evidence: raw.evidence,
        coverage,
    })
}

#[derive(Deserialize)]
struct RawLineAnnotation {
    #[serde(default)]
    text: String,
    #[serde(default)]
    color: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSelectionExplanation {
    #[serde(default)]
    subject: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    meaning: String,
    #[serde(default)]
    role_here: String,
    #[serde(default)]
    origin: Option<String>,
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
    let raw: RawLineAnnotation = serde_json::from_str(json).map_err(|e| {
        anyhow::anyhow!("LLM did not return the expected JSON: {e}; content: {content}")
    })?;
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

/// Parse the model-authored semantic fields for one arbitrary selection. The
/// selected source text and every evidence field are injected from deterministic
/// backend state, so a model reply cannot forge citations or upgrade its status.
pub fn parse_selection_explanation(
    content: &str,
    selected_text: &str,
    evidence_status: EvidenceStatus,
    sources: Vec<SourceLink>,
    warning: Option<String>,
) -> anyhow::Result<SelectionExplanation> {
    let raw: RawSelectionExplanation =
        serde_json::from_str(extract_json(content)).map_err(|e| {
            anyhow::anyhow!(
                "LLM did not return the expected selection JSON: {e}; content: {content}"
            )
        })?;
    if raw.subject != selected_text {
        anyhow::bail!(
            "LLM selection subject mismatch: expected {selected_text:?}, got {:?}",
            raw.subject
        );
    }
    let meaning = raw.meaning.trim();
    let role_here = raw.role_here.trim();
    if meaning.is_empty() || role_here.is_empty() {
        anyhow::bail!("LLM returned an incomplete selection explanation; content: {content}");
    }

    Ok(SelectionExplanation {
        selected_text: selected_text.to_string(),
        kind: parse_selection_kind(&raw.kind),
        meaning: meaning.to_string(),
        role_here: role_here.to_string(),
        origin: raw.origin.and_then(|origin| {
            let origin = origin.trim();
            (!origin.is_empty()).then(|| origin.to_string())
        }),
        evidence_status,
        sources,
        warning,
    })
}

fn parse_selection_kind(kind: &str) -> SelectionKind {
    match kind.trim().to_ascii_lowercase().as_str() {
        "模块" | "module" => SelectionKind::Module,
        "类型" | "type" => SelectionKind::Type,
        "函数" | "function" => SelectionKind::Function,
        "方法" | "method" => SelectionKind::Method,
        "变量" | "variable" => SelectionKind::Variable,
        "表达式" | "expression" => SelectionKind::Expression,
        _ => SelectionKind::Unknown,
    }
}

#[derive(Deserialize)]
struct RawFetchPlan {
    #[serde(default)]
    need: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOrientationSourcePlan {
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

/// Parse the stricter S-ORI-3 planning contract. Unlike query source planning,
/// malformed output is a visible orientation failure: silently treating a bad
/// plan as an empty one would make an oversized file look successfully covered.
/// Unknown/duplicate fnIds remain deterministic slicing concerns for the caller.
pub fn parse_orientation_source_plan(content: &str) -> anyhow::Result<Vec<String>> {
    serde_json::from_str::<RawOrientationSourcePlan>(extract_json(content))
        .map(|plan| plan.need)
        .map_err(|error| {
            anyhow::anyhow!(
                "LLM did not return the expected orientation source plan: {error}; content: {content}"
            )
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
    use std::sync::{Arc, Mutex};

    use axum::{extract::State, http::StatusCode, routing::post, Json, Router};

    #[derive(Clone)]
    struct MockResponse {
        status: StatusCode,
        body: &'static str,
        delay: Duration,
        requests: Arc<Mutex<Vec<serde_json::Value>>>,
    }

    struct MockServer {
        base_url: String,
        requests: Arc<Mutex<Vec<serde_json::Value>>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn mock_responses(
        State(state): State<MockResponse>,
        Json(request): Json<serde_json::Value>,
    ) -> (StatusCode, String) {
        state.requests.lock().unwrap().push(request);
        if !state.delay.is_zero() {
            tokio::time::sleep(state.delay).await;
        }
        (state.status, state.body.to_string())
    }

    async fn start_mock(status: StatusCode, body: &'static str, delay: Duration) -> MockServer {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = MockResponse {
            status,
            body,
            delay,
            requests: Arc::clone(&requests),
        };
        let app = Router::new()
            .route("/responses", post(mock_responses))
            .with_state(state);
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        MockServer {
            base_url: format!("http://{address}"),
            requests,
            task,
        }
    }

    fn test_proxy(base_url: &str, timeout: Duration) -> LlmProxy {
        LlmProxy {
            client: reqwest::Client::new(),
            base_url: base_url.to_string(),
            api_key: "fixture-key".to_string(),
            model: "fixture-model".to_string(),
            web_search_timeout: timeout,
        }
    }

    #[tokio::test]
    async fn responses_web_search_sends_only_public_query_and_protocol_fields() {
        let server = start_mock(
            StatusCode::OK,
            include_str!("../tests/fixtures/web_search/openai_cited.json"),
            Duration::ZERO,
        )
        .await;
        let proxy = test_proxy(&server.base_url, Duration::from_secs(1));

        let result = proxy
            .responses_web_search("Rust 1.80 release notes")
            .await
            .unwrap();

        assert_eq!(result.sources.len(), 2);
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.as_object().unwrap().len(), 4);
        assert_eq!(request["model"], "fixture-model");
        assert_eq!(request["input"], "Rust 1.80 release notes");
        assert_eq!(
            request["tools"],
            serde_json::json!([{ "type": "web_search" }])
        );
        assert_eq!(
            request["tool_choice"],
            serde_json::json!({ "type": "web_search" })
        );
    }

    #[tokio::test]
    async fn responses_web_search_accepts_uncited_supplier_output() {
        let server = start_mock(
            StatusCode::OK,
            include_str!("../tests/fixtures/web_search/deepseek_uncited.json"),
            Duration::ZERO,
        )
        .await;
        let proxy = test_proxy(&server.base_url, Duration::from_secs(1));

        let result = proxy.responses_web_search("DeepSeek docs").await.unwrap();

        assert!(result.sources.is_empty());
        assert_eq!(result.text, "DeepSeek returned a web-grounded summary.");
    }

    #[tokio::test]
    async fn responses_web_search_classifies_authentication_failure() {
        let server = start_mock(
            StatusCode::UNAUTHORIZED,
            include_str!("../tests/fixtures/web_search/error.json"),
            Duration::ZERO,
        )
        .await;
        let proxy = test_proxy(&server.base_url, Duration::from_secs(1));

        let error = proxy.responses_web_search("query").await.unwrap_err();

        assert!(matches!(
            error,
            WebSearchError::Authentication { status: 401, .. }
        ));
    }

    #[tokio::test]
    async fn responses_web_search_classifies_missing_endpoint_as_unsupported() {
        let server = start_mock(
            StatusCode::NOT_FOUND,
            include_str!("../tests/fixtures/web_search/error.json"),
            Duration::ZERO,
        )
        .await;
        let proxy = test_proxy(&server.base_url, Duration::from_secs(1));

        let error = proxy.responses_web_search("query").await.unwrap_err();

        assert!(matches!(
            error,
            WebSearchError::Unsupported { status: 404, .. }
        ));
    }

    #[tokio::test]
    async fn responses_web_search_classifies_rejected_tool_as_unsupported() {
        let server = start_mock(
            StatusCode::BAD_REQUEST,
            include_str!("../tests/fixtures/web_search/error.json"),
            Duration::ZERO,
        )
        .await;
        let proxy = test_proxy(&server.base_url, Duration::from_secs(1));

        let error = proxy.responses_web_search("query").await.unwrap_err();

        assert!(matches!(
            error,
            WebSearchError::Unsupported { status: 400, .. }
        ));
    }

    #[tokio::test]
    async fn responses_web_search_classifies_rate_limit_and_provider_failure() {
        for (status, expected_rate_limit) in [
            (StatusCode::TOO_MANY_REQUESTS, true),
            (StatusCode::BAD_GATEWAY, false),
        ] {
            let server = start_mock(
                status,
                include_str!("../tests/fixtures/web_search/error.json"),
                Duration::ZERO,
            )
            .await;
            let proxy = test_proxy(&server.base_url, Duration::from_secs(1));

            let error = proxy.responses_web_search("query").await.unwrap_err();

            if expected_rate_limit {
                assert!(matches!(
                    error,
                    WebSearchError::RateLimited { status: 429, .. }
                ));
            } else {
                assert!(matches!(
                    error,
                    WebSearchError::Provider { status: 502, .. }
                ));
            }
        }
    }

    #[tokio::test]
    async fn responses_web_search_classifies_timeout() {
        let server = start_mock(
            StatusCode::OK,
            include_str!("../tests/fixtures/web_search/openai_cited.json"),
            Duration::from_millis(200),
        )
        .await;
        let proxy = test_proxy(&server.base_url, Duration::from_millis(20));

        let error = proxy.responses_web_search("query").await.unwrap_err();

        assert!(matches!(error, WebSearchError::Timeout(_)));
    }

    #[tokio::test]
    #[ignore = "requires a configured provider/model with Responses Web Search and external network"]
    async fn responses_web_search_real_provider_smoke() {
        let _ = dotenvy::dotenv();
        let config = crate::settings::LlmConfig::from_env();
        let proxy = LlmProxy::from_config(&config).expect("OPENCODE_API_KEY must be configured");

        let result = proxy
            .responses_web_search("Rust programming language official website")
            .await
            .expect("configured provider/model must support Responses Web Search");

        assert!(!result.text.trim().is_empty());
    }

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
    fn selection_parser_injects_backend_evidence_and_constrains_kind() {
        let content = r#"{
            "subject":"from_str",
            "kind":"函数",
            "meaning":"把文本解析成 JSON 值",
            "roleHere":"读取当前配置输入",
            "origin":"serde_json",
            "evidenceStatus":"unverified",
            "sources":[{"title":"forged","url":"https://forged.invalid"}]
        }"#;
        let trusted_sources = vec![SourceLink {
            title: "serde_json docs".into(),
            url: "https://docs.rs/serde_json".into(),
        }];

        let explanation = parse_selection_explanation(
            content,
            "from_str",
            EvidenceStatus::WebCited,
            trusted_sources.clone(),
            None,
        )
        .unwrap();

        assert_eq!(explanation.selected_text, "from_str");
        assert_eq!(explanation.kind, SelectionKind::Function);
        assert_eq!(explanation.evidence_status, EvidenceStatus::WebCited);
        assert_eq!(explanation.sources, trusted_sources);
        assert_eq!(explanation.origin.as_deref(), Some("serde_json"));
    }

    #[test]
    fn selection_parser_maps_unknown_kind_and_rejects_incomplete_text() {
        let explanation = parse_selection_explanation(
            r#"{"subject":"x","kind":"mystery","meaning":"值","roleHere":"参与计算"}"#,
            "x",
            EvidenceStatus::Unverified,
            vec![],
            None,
        )
        .unwrap();
        assert_eq!(explanation.kind, SelectionKind::Unknown);

        assert!(parse_selection_explanation(
            r#"{"subject":"x","kind":"变量","meaning":"","roleHere":"参与计算"}"#,
            "x",
            EvidenceStatus::Unverified,
            vec![],
            None,
        )
        .is_err());
    }

    #[test]
    fn selection_parser_rejects_reply_for_a_different_subject() {
        let error = parse_selection_explanation(
            r#"{
                "subject":"output_address",
                "kind":"变量",
                "meaning":"Bridge 的输出地址字段",
                "roleHere":"作为 PushSocket 的连接目标"
            }"#,
            "tokio",
            EvidenceStatus::Unverified,
            vec![],
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("selection subject mismatch"));
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
        let out =
            d.push(": keep-alive\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n");
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

    #[test]
    fn orientation_source_plan_distinguishes_valid_empty_from_protocol_failure() {
        assert_eq!(
            parse_orientation_source_plan("```json\n{\"need\":[]}\n```").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            parse_orientation_source_plan("{\"need\":[\"fetch#5\",\"helper#8\"]}").unwrap(),
            vec!["fetch#5".to_string(), "helper#8".to_string()]
        );

        assert!(parse_orientation_source_plan("not-json").is_err());
        assert!(parse_orientation_source_plan("{\"other\":[]}").is_err());
        assert!(
            parse_orientation_source_plan("{\"need\":[\"fetch#5\"],\"source\":\"invented\"}")
                .is_err()
        );
    }
}
