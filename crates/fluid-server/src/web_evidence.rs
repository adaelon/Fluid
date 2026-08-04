//! Supplier-independent web evidence types and Responses API parsing.
//!
//! The provider-specific HTTP call remains in `LlmProxy`, the only network
//! boundary. This module owns the stable result/error contract that later
//! selection and query flows can share without depending on an API schema.

use std::fmt;

use serde::{Deserialize, Serialize};

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
