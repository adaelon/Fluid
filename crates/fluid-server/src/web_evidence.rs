//! Supplier-independent web evidence types and Responses API parsing.
//!
//! The provider-specific HTTP call remains in `LlmProxy`, the only network
//! boundary. This module owns the stable result/error contract that later
//! selection and query flows can share without depending on an API schema.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::context_assembler::build_web_search_planning_prompt;
use crate::llm_proxy::LlmProxy;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLink {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchResult {
    pub text: String,
    pub sources: Vec<SourceLink>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchPlan {
    Local,
    Search { query: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchPlanParseError {
    InvalidJson(String),
    EmptyQuery,
}

impl fmt::Display for SearchPlanParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(message) => write!(f, "invalid search plan JSON: {message}"),
            Self::EmptyQuery => write!(f, "search plan query is empty"),
        }
    }
}

impl std::error::Error for SearchPlanParseError {}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
enum RawSearchPlan {
    Local,
    Search {
        #[serde(default)]
        query: String,
    },
}

/// Parse the planning model's reply without performing IO. Markdown fences and
/// surrounding prose are tolerated, while an empty public query remains a hard
/// failure so the search adapter is never called with an ambiguous input.
pub fn parse_search_plan(content: &str) -> Result<SearchPlan, SearchPlanParseError> {
    let raw = serde_json::from_str::<RawSearchPlan>(extract_json_object(content))
        .map_err(|error| SearchPlanParseError::InvalidJson(error.to_string()))?;
    match raw {
        RawSearchPlan::Local => Ok(SearchPlan::Local),
        RawSearchPlan::Search { query } => {
            let query = query.trim();
            if query.is_empty() {
                return Err(SearchPlanParseError::EmptyQuery);
            }
            Ok(SearchPlan::Search {
                query: query.to_string(),
            })
        }
    }
}

fn extract_json_object(content: &str) -> &str {
    let content = content.trim();
    if let Some(rest) = content.strip_prefix("```") {
        let rest = rest.trim_start_matches("json").trim_start();
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim();
        }
    }
    if let (Some(start), Some(end)) = (content.find('{'), content.rfind('}')) {
        if end >= start {
            return content[start..=end].trim();
        }
    }
    content
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceStatus {
    ProjectSource,
    WebCited,
    WebUncited,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceOutcome {
    pub status: EvidenceStatus,
    pub text: Option<String>,
    pub sources: Vec<SourceLink>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct EvidenceRequest<'a> {
    /// Private selection/question/code context visible only to the non-web plan.
    pub private_context: &'a str,
    /// Already-bounded common manifest/lockfile snippets.
    pub dependency_hints: &'a str,
    /// A caller-resolved project source that is already sufficient, if any.
    pub project_evidence: Option<&'a str>,
    /// User-level pre-authorization switch; false means no planning or search.
    pub allow_web: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WebEvidenceService;

impl WebEvidenceService {
    pub const fn new() -> Self {
        Self
    }

    /// Resolve one evidence request through `local -> plan -> search -> fallback`.
    /// The caller passes one `Arc<LlmProxy>` snapshot, and this method retains that
    /// exact snapshot for both LLM calls so a concurrent settings swap cannot mix
    /// providers or models halfway through the request.
    pub async fn resolve(
        &self,
        llm: Arc<LlmProxy>,
        request: EvidenceRequest<'_>,
    ) -> EvidenceOutcome {
        if let Some(project_evidence) = request
            .project_evidence
            .map(str::trim)
            .filter(|evidence| !evidence.is_empty())
        {
            return EvidenceOutcome::project_source(project_evidence);
        }
        if !request.allow_web {
            return EvidenceOutcome::unverified(None);
        }

        let (system, user) = build_web_search_planning_prompt(
            request.private_context,
            request.dependency_hints,
        );
        let plan = match llm.complete(&system, &user).await {
            Ok(content) => match parse_search_plan(&content) {
                Ok(plan) => plan,
                Err(_) => {
                    return EvidenceOutcome::unverified(Some(
                        "未核验：联网失败（检索意图规划返回无效结果）".to_string(),
                    ));
                }
            },
            Err(_) => {
                return EvidenceOutcome::unverified(Some(
                    "未核验：联网失败（检索意图规划调用失败）".to_string(),
                ));
            }
        };

        let SearchPlan::Search { query } = plan else {
            return EvidenceOutcome::unverified(None);
        };

        match llm.responses_web_search(&query).await {
            Ok(result) if result.sources.is_empty() => EvidenceOutcome {
                status: EvidenceStatus::WebUncited,
                text: Some(result.text),
                sources: Vec::new(),
                warning: None,
            },
            Ok(result) => EvidenceOutcome {
                status: EvidenceStatus::WebCited,
                text: Some(result.text),
                sources: result.sources,
                warning: None,
            },
            Err(error) => EvidenceOutcome::unverified(Some(web_search_warning(&error))),
        }
    }
}

impl EvidenceOutcome {
    fn project_source(text: &str) -> Self {
        Self {
            status: EvidenceStatus::ProjectSource,
            text: Some(text.to_string()),
            sources: Vec::new(),
            warning: None,
        }
    }

    fn unverified(warning: Option<String>) -> Self {
        Self {
            status: EvidenceStatus::Unverified,
            text: None,
            sources: Vec::new(),
            warning,
        }
    }
}

/// Function-shaped entry point for selection/query consumers that do not need to
/// retain a service value.
#[allow(dead_code)] // staged shared entry point; route consumers arrive after S-WEB-2
pub async fn resolve_web_evidence(
    llm: Arc<LlmProxy>,
    request: EvidenceRequest<'_>,
) -> EvidenceOutcome {
    WebEvidenceService::new().resolve(llm, request).await
}

/// Stable failure categories consumed by the future evidence orchestrator.
/// Provider response bodies are retained only as diagnostic text; callers must
/// still turn every variant into a visible local-answer fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSearchError {
    Authentication { status: u16, message: String },
    Unsupported { status: u16, message: String },
    RateLimited { status: u16, message: String },
    Provider { status: u16, message: String },
    Timeout(String),
    Transport(String),
    InvalidResponse(String),
    MissingOutputText,
}

impl fmt::Display for WebSearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authentication { status, message } => {
                write!(
                    f,
                    "web search authentication failed (HTTP {status}): {message}"
                )
            }
            Self::Unsupported { status, message } => {
                write!(f, "web search unsupported (HTTP {status}): {message}")
            }
            Self::RateLimited { status, message } => {
                write!(f, "web search rate limited (HTTP {status}): {message}")
            }
            Self::Provider { status, message } => {
                write!(f, "web search provider failed (HTTP {status}): {message}")
            }
            Self::Timeout(message) => write!(f, "web search timed out: {message}"),
            Self::Transport(message) => write!(f, "web search transport failed: {message}"),
            Self::InvalidResponse(message) => {
                write!(f, "web search returned an invalid response: {message}")
            }
            Self::MissingOutputText => write!(f, "web search returned no output text"),
        }
    }
}

impl std::error::Error for WebSearchError {}

fn web_search_warning(error: &WebSearchError) -> String {
    let reason = match error {
        WebSearchError::Authentication { .. } => "联网检索认证失败",
        WebSearchError::Unsupported { .. } => "当前供应商或模型不支持联网检索",
        WebSearchError::RateLimited { .. } => "联网检索受到限流",
        WebSearchError::Provider { .. } => "联网检索供应商服务异常",
        WebSearchError::Timeout(_) => "联网检索超时",
        WebSearchError::Transport(_) => "联网检索网络传输失败",
        WebSearchError::InvalidResponse(_) | WebSearchError::MissingOutputText => {
            "联网检索返回无效结果"
        }
    };
    format!("未核验：联网失败（{reason}）")
}

/// Parse both OpenAI's Responses shape and compatible supplier variants.
///
/// OpenAI citations live in `message.content[].annotations`; the optional full
/// source list lives in `web_search_call.action.sources`. Some compatible
/// suppliers return only output text, so an empty source list is a valid result.
pub(crate) fn parse_web_search_response(body: &str) -> Result<WebSearchResult, WebSearchError> {
    let root: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| WebSearchError::InvalidResponse(error.to_string()))?;

    let mut text_parts = Vec::new();
    let mut sources = Vec::new();

    if let Some(output) = root.get("output").and_then(serde_json::Value::as_array) {
        for item in output {
            collect_action_sources(item, &mut sources);
            let Some(content) = item.get("content").and_then(serde_json::Value::as_array) else {
                continue;
            };
            for part in content {
                if let Some(text) = part.get("text").and_then(serde_json::Value::as_str) {
                    if !text.trim().is_empty() {
                        text_parts.push(text.trim().to_string());
                    }
                }
                collect_annotations(part, &mut sources);
            }
        }
    }

    // A few OpenAI-compatible providers expose the SDK-style convenience field
    // in raw JSON. Accept it without requiring it from conforming providers.
    if text_parts.is_empty() {
        if let Some(text) = root.get("output_text").and_then(serde_json::Value::as_str) {
            if !text.trim().is_empty() {
                text_parts.push(text.trim().to_string());
            }
        }
    }

    if text_parts.is_empty() {
        return Err(WebSearchError::MissingOutputText);
    }

    Ok(WebSearchResult {
        text: text_parts.join("\n"),
        sources,
    })
}

fn collect_annotations(part: &serde_json::Value, sources: &mut Vec<SourceLink>) {
    let Some(annotations) = part
        .get("annotations")
        .and_then(serde_json::Value::as_array)
    else {
        return;
    };
    for annotation in annotations {
        if annotation.get("type").and_then(serde_json::Value::as_str) != Some("url_citation") {
            continue;
        }
        // The canonical Responses shape is flat. The nested fallback tolerates
        // compatible gateways that wrap citation fields in `url_citation`.
        let citation = annotation.get("url_citation").unwrap_or(annotation);
        push_source(
            sources,
            citation.get("title").and_then(serde_json::Value::as_str),
            citation.get("url").and_then(serde_json::Value::as_str),
        );
    }
}

fn collect_action_sources(item: &serde_json::Value, sources: &mut Vec<SourceLink>) {
    let Some(action_sources) = item
        .get("action")
        .and_then(|action| action.get("sources"))
        .and_then(serde_json::Value::as_array)
    else {
        return;
    };
    for source in action_sources {
        push_source(
            sources,
            source.get("title").and_then(serde_json::Value::as_str),
            source.get("url").and_then(serde_json::Value::as_str),
        );
    }
}

fn push_source(sources: &mut Vec<SourceLink>, title: Option<&str>, url: Option<&str>) {
    let Some(url) = url.map(str::trim).filter(|url| !url.is_empty()) else {
        return;
    };
    let title = title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(url);

    if let Some(existing) = sources.iter_mut().find(|source| source.url == url) {
        if existing.title == existing.url && title != url {
            existing.title = title.to_string();
        }
        return;
    }
    sources.push(SourceLink {
        title: title.to_string(),
        url: url.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    use axum::{extract::State, http::StatusCode, routing::post, Json, Router};

    use crate::settings::LlmConfig;

    #[derive(Debug, Clone)]
    struct RecordedRequest {
        path: &'static str,
        body: serde_json::Value,
    }

    #[derive(Clone)]
    struct MockBackend {
        plan: String,
        web_status: StatusCode,
        web_body: String,
        web_delay: Duration,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
    }

    struct MockServer {
        base_url: String,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn mock_plan(
        State(state): State<MockBackend>,
        Json(request): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        state.requests.lock().unwrap().push(RecordedRequest {
            path: "/chat/completions",
            body: request,
        });
        Json(serde_json::json!({
            "choices": [{ "message": { "content": state.plan } }]
        }))
    }

    async fn mock_web_search(
        State(state): State<MockBackend>,
        Json(request): Json<serde_json::Value>,
    ) -> (StatusCode, String) {
        state.requests.lock().unwrap().push(RecordedRequest {
            path: "/responses",
            body: request,
        });
        if !state.web_delay.is_zero() {
            tokio::time::sleep(state.web_delay).await;
        }
        (state.web_status, state.web_body)
    }

    async fn start_mock(
        plan: &str,
        web_status: StatusCode,
        web_body: &str,
        web_delay: Duration,
    ) -> MockServer {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = MockBackend {
            plan: plan.to_string(),
            web_status,
            web_body: web_body.to_string(),
            web_delay,
            requests: Arc::clone(&requests),
        };
        let app = Router::new()
            .route("/chat/completions", post(mock_plan))
            .route("/responses", post(mock_web_search))
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

    fn test_proxy(server: &MockServer, timeout: Duration) -> Arc<LlmProxy> {
        let config = LlmConfig {
            base_url: server.base_url.clone(),
            model: "fixture-model".to_string(),
            api_key: "fixture-key".to_string(),
        };
        Arc::new(
            LlmProxy::from_config_with_web_search_timeout(&config, timeout)
                .expect("fixture key enables proxy"),
        )
    }

    fn request() -> EvidenceRequest<'static> {
        EvidenceRequest {
            private_context: "secret_project::PrivateType calls serde_json::from_str",
            dependency_hints: "【依赖文件: Cargo.toml】\nserde_json = \"1\"",
            project_evidence: None,
            allow_web: true,
        }
    }

    #[test]
    fn parses_local_and_search_plans() {
        assert_eq!(
            parse_search_plan("model: {\"action\":\"local\"}").unwrap(),
            SearchPlan::Local
        );
        assert_eq!(
            parse_search_plan(
                "```json\n{\"action\":\"search\",\"query\":\"  serde_json Value docs  \"}\n```"
            )
            .unwrap(),
            SearchPlan::Search {
                query: "serde_json Value docs".to_string(),
            }
        );
    }

    #[test]
    fn rejects_bad_plan_json_and_empty_search_query() {
        assert!(matches!(
            parse_search_plan("not-json"),
            Err(SearchPlanParseError::InvalidJson(_))
        ));
        assert_eq!(
            parse_search_plan("{\"action\":\"search\",\"query\":\"   \"}")
                .unwrap_err(),
            SearchPlanParseError::EmptyQuery
        );
    }

    #[tokio::test]
    async fn project_source_short_circuits_before_planning_and_search() {
        let server = start_mock(
            "{\"action\":\"search\",\"query\":\"unused\"}",
            StatusCode::OK,
            include_str!("../tests/fixtures/web_search/openai_cited.json"),
            Duration::ZERO,
        )
        .await;
        let proxy = test_proxy(&server, Duration::from_secs(1));
        let mut req = request();
        req.project_evidence = Some("fn local_definition() {}");

        let outcome = WebEvidenceService::new().resolve(proxy, req).await;

        assert_eq!(outcome.status, EvidenceStatus::ProjectSource);
        assert_eq!(outcome.text.as_deref(), Some("fn local_definition() {}"));
        assert!(outcome.warning.is_none());
        assert!(server.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn web_disabled_short_circuits_without_llm_calls() {
        let server = start_mock(
            "{\"action\":\"search\",\"query\":\"unused\"}",
            StatusCode::OK,
            include_str!("../tests/fixtures/web_search/openai_cited.json"),
            Duration::ZERO,
        )
        .await;
        let proxy = test_proxy(&server, Duration::from_secs(1));
        let mut req = request();
        req.allow_web = false;

        let outcome = WebEvidenceService::new().resolve(proxy, req).await;

        assert_eq!(outcome.status, EvidenceStatus::Unverified);
        assert!(outcome.warning.is_none());
        assert!(server.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn local_plan_skips_supplier_search() {
        let server = start_mock(
            "{\"action\":\"local\"}",
            StatusCode::OK,
            include_str!("../tests/fixtures/web_search/openai_cited.json"),
            Duration::ZERO,
        )
        .await;
        let proxy = test_proxy(&server, Duration::from_secs(1));

        let outcome = WebEvidenceService::new().resolve(proxy, request()).await;

        assert_eq!(outcome.status, EvidenceStatus::Unverified);
        assert!(outcome.warning.is_none());
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/chat/completions");
    }

    #[tokio::test]
    async fn malformed_plan_becomes_visible_local_fallback() {
        let server = start_mock(
            "not-json",
            StatusCode::OK,
            include_str!("../tests/fixtures/web_search/openai_cited.json"),
            Duration::ZERO,
        )
        .await;
        let proxy = test_proxy(&server, Duration::from_secs(1));

        let outcome = WebEvidenceService::new().resolve(proxy, request()).await;

        assert_eq!(outcome.status, EvidenceStatus::Unverified);
        assert!(outcome
            .warning
            .as_deref()
            .unwrap()
            .contains("检索意图规划返回无效结果"));
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cited_search_uses_one_proxy_snapshot_and_sends_only_the_plan_query() {
        let server = start_mock(
            "{\"action\":\"search\",\"query\":\"serde_json Value public docs\"}",
            StatusCode::OK,
            include_str!("../tests/fixtures/web_search/openai_cited.json"),
            Duration::ZERO,
        )
        .await;
        let proxy = test_proxy(&server, Duration::from_secs(1));

        let outcome = WebEvidenceService::new().resolve(proxy, request()).await;

        assert_eq!(outcome.status, EvidenceStatus::WebCited);
        assert_eq!(outcome.sources.len(), 2);
        assert!(outcome.warning.is_none());

        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].path, "/chat/completions");
        assert_eq!(requests[1].path, "/responses");
        assert_eq!(requests[0].body["model"], "fixture-model");
        assert_eq!(requests[1].body["model"], "fixture-model");
        assert!(requests[0].body.to_string().contains("secret_project"));
        assert_eq!(requests[1].body.as_object().unwrap().len(), 4);
        assert_eq!(
            requests[1].body["input"],
            "serde_json Value public docs"
        );
        assert!(!requests[1].body.to_string().contains("secret_project"));
        assert!(!requests[1].body.to_string().contains("PrivateType"));
    }

    #[tokio::test]
    async fn uncited_supplier_output_remains_a_successful_evidence_state() {
        let server = start_mock(
            "{\"action\":\"search\",\"query\":\"DeepSeek Responses docs\"}",
            StatusCode::OK,
            include_str!("../tests/fixtures/web_search/deepseek_uncited.json"),
            Duration::ZERO,
        )
        .await;
        let proxy = test_proxy(&server, Duration::from_secs(1));

        let outcome = WebEvidenceService::new().resolve(proxy, request()).await;

        assert_eq!(outcome.status, EvidenceStatus::WebUncited);
        assert_eq!(
            outcome.text.as_deref(),
            Some("DeepSeek returned a web-grounded summary.")
        );
        assert!(outcome.sources.is_empty());
        assert!(outcome.warning.is_none());
    }

    #[tokio::test]
    async fn unsupported_provider_becomes_visible_local_fallback() {
        let server = start_mock(
            "{\"action\":\"search\",\"query\":\"public docs\"}",
            StatusCode::NOT_FOUND,
            include_str!("../tests/fixtures/web_search/error.json"),
            Duration::ZERO,
        )
        .await;
        let proxy = test_proxy(&server, Duration::from_secs(1));

        let outcome = WebEvidenceService::new().resolve(proxy, request()).await;

        assert_eq!(outcome.status, EvidenceStatus::Unverified);
        assert!(outcome.warning.as_deref().unwrap().contains("不支持联网检索"));
        assert!(outcome.text.is_none());
    }

    #[tokio::test]
    async fn rate_limit_becomes_visible_local_fallback() {
        let server = start_mock(
            "{\"action\":\"search\",\"query\":\"public docs\"}",
            StatusCode::TOO_MANY_REQUESTS,
            include_str!("../tests/fixtures/web_search/error.json"),
            Duration::ZERO,
        )
        .await;
        let proxy = test_proxy(&server, Duration::from_secs(1));

        let outcome = WebEvidenceService::new().resolve(proxy, request()).await;

        assert_eq!(outcome.status, EvidenceStatus::Unverified);
        assert!(outcome.warning.as_deref().unwrap().contains("限流"));
    }

    #[tokio::test]
    async fn timeout_becomes_visible_local_fallback() {
        let server = start_mock(
            "{\"action\":\"search\",\"query\":\"public docs\"}",
            StatusCode::OK,
            include_str!("../tests/fixtures/web_search/openai_cited.json"),
            Duration::from_millis(100),
        )
        .await;
        let proxy = test_proxy(&server, Duration::from_millis(10));

        let outcome = WebEvidenceService::new().resolve(proxy, request()).await;

        assert_eq!(outcome.status, EvidenceStatus::Unverified);
        assert!(outcome.warning.as_deref().unwrap().contains("超时"));
    }

    #[test]
    fn parses_openai_citations_and_action_sources_without_duplicates() {
        let result = parse_web_search_response(include_str!(
            "../tests/fixtures/web_search/openai_cited.json"
        ))
        .unwrap();

        assert_eq!(result.text, "Rust 1.80 was released with cited details.");
        assert_eq!(
            result.sources,
            vec![
                SourceLink {
                    title: "Rust Blog".into(),
                    url: "https://blog.rust-lang.org/release.html".into(),
                },
                SourceLink {
                    title: "Release Notes".into(),
                    url: "https://doc.rust-lang.org/releases.html".into(),
                },
            ]
        );
    }

    #[test]
    fn accepts_deepseek_output_without_sources() {
        let result = parse_web_search_response(include_str!(
            "../tests/fixtures/web_search/deepseek_uncited.json"
        ))
        .unwrap();

        assert_eq!(result.text, "DeepSeek returned a web-grounded summary.");
        assert!(result.sources.is_empty());
    }

    #[test]
    fn rejects_a_response_without_output_text() {
        let error = parse_web_search_response(include_str!(
            "../tests/fixtures/web_search/missing_output_text.json"
        ))
        .unwrap_err();

        assert_eq!(error, WebSearchError::MissingOutputText);
    }

    #[test]
    fn rejects_malformed_json() {
        let error = parse_web_search_response("{not-json").unwrap_err();
        assert!(matches!(error, WebSearchError::InvalidResponse(_)));
    }
}
