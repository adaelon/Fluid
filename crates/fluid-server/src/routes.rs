//! HTTP routes.
//!
//! - `GET /api/project/tree`        -> { files: FileNode[] }
//! - `GET /api/file?path=<rel>`     -> { source: string }
//! - `GET /api/project/graph`       -> KnowledgeGraph | null   (S2, optional)
//!
//! All handlers share an `Arc<AppState>` as axum state.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::cache_store::{Capsule, CacheStore, CapsuleEntry, LineAnnotation};
use crate::context_assembler::{assemble_gen_context, build_gen_prompt, slice_span, FunctionSpan, SharedContext};
use crate::graph_loader::{GraphLoader, KnowledgeGraph};
use crate::llm_proxy::{parse_generation, LlmProxy};
use crate::project_reader::{FileNode, ProjectReader, ReadErr};

/// Shared server state: file reader + optional knowledge graph + bypass cache +
/// optional LLM proxy (None when no API key is configured — S1–S5 paths still work).
pub struct AppState {
    pub reader: ProjectReader,
    pub graph: GraphLoader,
    /// On-disk capsule cache (`.fluid/`), consumed by `/api/generate` (S6).
    pub cache: CacheStore,
    /// LLM proxy; `None` when `OPENCODE_API_KEY` is unset (generate → 503).
    pub llm: Option<LlmProxy>,
}

type Shared = Arc<AppState>;

pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/api/project/tree", get(tree))
        .route("/api/file", get(file))
        .route("/api/project/graph", get(graph))
        .route("/api/generate", post(generate))
        .with_state(state)
}

#[derive(Serialize)]
struct TreeResponse {
    files: Vec<FileNode>,
}

async fn tree(State(state): State<Shared>) -> Json<TreeResponse> {
    Json(TreeResponse {
        files: state.reader.list_files(),
    })
}

#[derive(Deserialize)]
struct FileQuery {
    path: String,
}

#[derive(Serialize)]
struct FileResponse {
    source: String,
}

async fn file(State(state): State<Shared>, Query(q): Query<FileQuery>) -> impl IntoResponse {
    match state.reader.read_file(&q.path) {
        Ok(source) => (StatusCode::OK, Json(FileResponse { source })).into_response(),
        Err(ReadErr::NotFound) => (StatusCode::NOT_FOUND, "file not found").into_response(),
        Err(ReadErr::Forbidden) => {
            (StatusCode::FORBIDDEN, "path outside project root").into_response()
        }
    }
}

/// Returns the knowledge graph, or `null` when no `.understand-anything/` is
/// present (ADR-0011: optional enhancement, never required).
async fn graph(State(state): State<Shared>) -> Json<Option<KnowledgeGraph>> {
    Json(state.graph.graph().cloned())
}

/// `POST /api/generate` — per-function semantic generation (S6, non-streaming).
///
/// Flow (技术方案 §5): slice the function source span → cache key → on hit return
/// the stored entry with `cacheHit: true` and **no LLM call** (the zero-token
/// contract); on miss assemble context, call the LLM once, parse, cache, return.
#[derive(Deserialize)]
struct GenerateRequest {
    #[serde(rename = "filePath")]
    file_path: String,
    #[serde(rename = "fn")]
    func: FunctionSpan,
    #[serde(default)]
    roster: Vec<String>,
    #[serde(rename = "keyLines", default)]
    key_lines: Vec<u32>,
    #[serde(default)]
    shared: SharedContext,
}

#[derive(Serialize)]
struct GenerateResponse {
    #[serde(rename = "cacheHit")]
    cache_hit: bool,
    capsule: Capsule,
    lines: Vec<LineAnnotation>,
}

async fn generate(State(state): State<Shared>, Json(req): Json<GenerateRequest>) -> impl IntoResponse {
    // 1. Resolve the function source span (deterministic; cache key derives from it).
    let source = match state.reader.read_file(&req.file_path) {
        Ok(s) => s,
        Err(ReadErr::NotFound) => return (StatusCode::NOT_FOUND, "file not found").into_response(),
        Err(ReadErr::Forbidden) => {
            return (StatusCode::FORBIDDEN, "path outside project root").into_response()
        }
    };
    let Some(fn_source) = slice_span(&source, req.func.line_range) else {
        return (StatusCode::BAD_REQUEST, "invalid lineRange for file").into_response();
    };

    // 2. Cache: a hit returns the stored entry with zero token, no LLM (核心律).
    if let Some(entry) = state.cache.get(&fn_source) {
        eprintln!("[generate] cache HIT {}#{} — zero token", req.file_path, req.func.name);
        return Json(GenerateResponse {
            cache_hit: true,
            capsule: entry.capsule,
            lines: entry.lines,
        })
        .into_response();
    }

    // 3. Miss → need the LLM.
    let Some(llm) = state.llm.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "LLM not configured: set OPENCODE_API_KEY",
        )
            .into_response();
    };

    let ctx = assemble_gen_context(state.graph.graph(), &req.file_path, &req.roster, &req.shared);
    let (system, user) = build_gen_prompt(&req.func, &fn_source, &req.key_lines, &ctx);

    eprintln!("[generate] cache MISS {}#{} — calling LLM ({})", req.file_path, req.func.name, llm.model);
    let content = match llm.complete(&system, &user).await {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("LLM error: {e}")).into_response(),
    };
    let (capsule, lines) = match parse_generation(&content, &req.func.id) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("LLM parse error: {e}")).into_response(),
    };

    // 4. Persist for the next open (best-effort; a write failure must not fail the response).
    let entry = CapsuleEntry {
        capsule: capsule.clone(),
        lines: lines.clone(),
    };
    if let Err(e) = state.cache.put(&fn_source, &entry) {
        eprintln!("[generate] warning: cache put failed: {e}");
    }

    Json(GenerateResponse {
        cache_hit: false,
        capsule,
        lines,
    })
    .into_response()
}
