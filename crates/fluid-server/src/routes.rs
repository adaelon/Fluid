//! HTTP / WS routes.
//!
//! - `GET /api/project/tree`        -> { files: FileNode[] }
//! - `GET /api/file?path=<rel>`     -> { source: string }
//! - `GET /api/project/graph`       -> KnowledgeGraph | null   (S2, optional)
//! - `WS  /api/generate`            -> per-function streaming generation (S7)
//!
//! All handlers share an `Arc<AppState>` as axum state.

use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::get,
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
    /// LLM proxy; `None` when `OPENCODE_API_KEY` is unset (generate → error frame).
    pub llm: Option<LlmProxy>,
}

type Shared = Arc<AppState>;

pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/api/project/tree", get(tree))
        .route("/api/file", get(file))
        .route("/api/project/graph", get(graph))
        .route("/api/generate", get(generate_ws))
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

// — WS /api/generate — per-function streaming generation (S7a) —
//
// Protocol (技术方案 §4). The client sends one or more request frames on the
// socket (each tagged with its own `reqId`); the server processes them
// sequentially (scheduling/concurrency is S8) and answers each with a sequence
// of frames carrying the same `reqId`:
//
//   miss : capsule → line×N → done
//   hit  : cache-hit → capsule → line×N → done   (zero token, no LLM call)
//   fail : error                                   (terminal, no done)
//
// "Streaming" here is semantic framing (Option B): the LLM is still a single
// non-streaming call, but its product is emitted frame-by-frame so the frontend
// renders the capsule first and then each key line as it arrives. The cache-hit
// path emits the same frame sequence (prefixed with `cache-hit`) so the client
// renders identically whether or not a token was spent.

#[derive(Deserialize)]
struct GenerateRequest {
    #[serde(rename = "reqId", default)]
    req_id: String,
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

/// One outbound frame on the `/api/generate` socket. Serialized with a `kind`
/// tag (kebab-case: `cache-hit` / `capsule` / `line` / `done` / `error`); the
/// `reqId` is injected by the sender so a frame stays independent of any one
/// request.
#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum GenFrame {
    CacheHit,
    Capsule { capsule: Capsule },
    Line { line: LineAnnotation },
    Done,
    Error { message: String },
}

/// Build the frame sequence for a produced (capsule, lines): an optional leading
/// `cache-hit`, then the capsule, then each line in order, then `done`. This is
/// the deterministic core of the protocol — unit-tested directly.
fn build_frames(cache_hit: bool, capsule: Capsule, lines: Vec<LineAnnotation>) -> Vec<GenFrame> {
    let mut frames = Vec::with_capacity(lines.len() + 3);
    if cache_hit {
        frames.push(GenFrame::CacheHit);
    }
    frames.push(GenFrame::Capsule { capsule });
    for line in lines {
        frames.push(GenFrame::Line { line });
    }
    frames.push(GenFrame::Done);
    frames
}

/// Run one generation request to a complete frame sequence. A cache hit returns
/// before the LLM is ever consulted (the zero-token contract). On any failure a
/// single terminal `error` frame is returned.
async fn run_generation(state: &AppState, req: GenerateRequest) -> Vec<GenFrame> {
    // 1. Resolve the function source span (deterministic; the cache key derives from it).
    let source = match state.reader.read_file(&req.file_path) {
        Ok(s) => s,
        Err(ReadErr::NotFound) => return vec![err("file not found")],
        Err(ReadErr::Forbidden) => return vec![err("path outside project root")],
    };
    let Some(fn_source) = slice_span(&source, req.func.line_range) else {
        return vec![err("invalid lineRange for file")];
    };

    // 2. Cache: a hit returns the stored entry with zero token, no LLM (核心律).
    if let Some(entry) = state.cache.get(&fn_source) {
        eprintln!("[generate] cache HIT {}#{} — zero token", req.file_path, req.func.name);
        return build_frames(true, entry.capsule, entry.lines);
    }

    // 3. Miss → need the LLM.
    let Some(llm) = state.llm.as_ref() else {
        return vec![err("LLM not configured: set OPENCODE_API_KEY")];
    };

    let ctx = assemble_gen_context(state.graph.graph(), &req.file_path, &req.roster, &req.shared);
    let (system, user) = build_gen_prompt(&req.func, &fn_source, &req.key_lines, &ctx);

    eprintln!("[generate] cache MISS {}#{} — calling LLM ({})", req.file_path, req.func.name, llm.model);
    let content = match llm.complete(&system, &user).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[generate] LLM error {}#{}: {e}", req.file_path, req.func.name);
            return vec![err(format!("LLM error: {e}"))];
        }
    };
    let (capsule, lines) = match parse_generation(&content, &req.func.id) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[generate] LLM parse error {}#{}: {e}", req.file_path, req.func.name);
            return vec![err(format!("LLM parse error: {e}"))];
        }
    };

    // 4. Persist for the next open (best-effort; a write failure must not fail the response).
    let entry = CapsuleEntry {
        capsule: capsule.clone(),
        lines: lines.clone(),
    };
    if let Err(e) = state.cache.put(&fn_source, &entry) {
        eprintln!("[generate] warning: cache put failed: {e}");
    }

    build_frames(false, capsule, lines)
}

fn err(message: impl Into<String>) -> GenFrame {
    GenFrame::Error {
        message: message.into(),
    }
}

async fn generate_ws(ws: WebSocketUpgrade, State(state): State<Shared>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_generate_socket(socket, state))
}

/// Drive one `/api/generate` socket: read request frames, process each
/// sequentially, stream its frames back tagged with the request's `reqId`.
async fn handle_generate_socket(mut socket: WebSocket, state: Shared) {
    while let Some(Ok(msg)) = socket.recv().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            // ignore ping/pong/binary; axum answers pings for us.
            _ => continue,
        };

        let (req_id, frames) = match serde_json::from_str::<GenerateRequest>(&text) {
            Ok(req) => {
                let req_id = req.req_id.clone();
                (req_id, run_generation(&state, req).await)
            }
            Err(e) => (String::new(), vec![err(format!("bad request: {e}"))]),
        };

        for frame in &frames {
            if send_frame(&mut socket, &req_id, frame).await.is_err() {
                return; // peer gone
            }
        }
    }
}

/// Serialize a frame and inject `reqId` before sending it as a text message.
async fn send_frame(socket: &mut WebSocket, req_id: &str, frame: &GenFrame) -> Result<(), axum::Error> {
    let mut v = serde_json::to_value(frame).unwrap_or_else(|_| {
        serde_json::json!({ "kind": "error", "message": "frame serialize failed" })
    });
    if let serde_json::Value::Object(map) = &mut v {
        map.insert("reqId".to_string(), serde_json::Value::String(req_id.to_string()));
    }
    socket.send(Message::Text(v.to_string())).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::graph_loader::GraphLoader;

    // — minimal self-cleaning temp dir (project habit: hand-rolled, cf. S1) —
    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let mut p = std::env::temp_dir();
            p.push(format!(
                "fluid-routes-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn cap(fn_id: &str) -> Capsule {
        Capsule {
            fn_id: fn_id.to_string(),
            signature: "def f()".into(),
            summary: "做一件事".into(),
            complexity: "simple".into(),
            io: "无->无".into(),
        }
    }

    fn line(fn_id: &str, n: u32) -> LineAnnotation {
        LineAnnotation {
            fn_id: fn_id.to_string(),
            line_number: n,
            text: "一行".into(),
            color: "#7ee787".into(),
        }
    }

    fn make_state(root: &Path, llm: Option<LlmProxy>) -> AppState {
        AppState {
            reader: ProjectReader::new(root.to_path_buf()).unwrap(),
            graph: GraphLoader::load(root),
            cache: CacheStore::new(root, "test-model", "p1"),
            llm,
        }
    }

    fn req(file_path: &str, line_range: [u32; 2]) -> GenerateRequest {
        GenerateRequest {
            req_id: "r1".into(),
            file_path: file_path.into(),
            func: FunctionSpan {
                id: "f#1".into(),
                name: "f".into(),
                line_range,
            },
            roster: vec![],
            key_lines: vec![],
            shared: SharedContext::default(),
        }
    }

    #[test]
    fn build_frames_hit_orders_cache_hit_capsule_lines_done() {
        let frames = build_frames(true, cap("f#1"), vec![line("f#1", 2), line("f#1", 3)]);
        assert_eq!(frames.len(), 5);
        assert_eq!(frames[0], GenFrame::CacheHit);
        assert!(matches!(frames[1], GenFrame::Capsule { .. }));
        assert!(matches!(frames[2], GenFrame::Line { .. }));
        assert!(matches!(frames[3], GenFrame::Line { .. }));
        assert_eq!(frames[4], GenFrame::Done);
    }

    #[test]
    fn build_frames_miss_has_no_cache_hit_and_empty_lines_ok() {
        let frames = build_frames(false, cap("f#1"), vec![]);
        assert_eq!(frames.len(), 2);
        assert!(matches!(frames[0], GenFrame::Capsule { .. }));
        assert_eq!(frames[1], GenFrame::Done);
    }

    #[test]
    fn frame_serializes_with_kebab_kind() {
        let v = serde_json::to_value(GenFrame::CacheHit).unwrap();
        assert_eq!(v["kind"], "cache-hit");
        let v = serde_json::to_value(err("x")).unwrap();
        assert_eq!(v["kind"], "error");
        assert_eq!(v["message"], "x");
    }

    #[tokio::test]
    async fn cache_hit_returns_frames_with_zero_llm() {
        let tmp = TmpDir::new();
        std::fs::write(tmp.path().join("a.py"), "def f():\n    return 1\n").unwrap();
        // llm: None — yet a pre-populated cache must still succeed (zero token).
        let state = make_state(tmp.path(), None);
        let fn_source = "def f():\n    return 1";
        state
            .cache
            .put(
                fn_source,
                &CapsuleEntry {
                    capsule: cap("f#1"),
                    lines: vec![line("f#1", 2)],
                },
            )
            .unwrap();

        let frames = run_generation(&state, req("a.py", [1, 2])).await;
        assert_eq!(frames[0], GenFrame::CacheHit);
        assert!(matches!(frames.last(), Some(GenFrame::Done)));
        assert!(frames.iter().any(|f| matches!(f, GenFrame::Line { .. })));
    }

    #[tokio::test]
    async fn invalid_line_range_yields_single_error_frame() {
        let tmp = TmpDir::new();
        std::fs::write(tmp.path().join("a.py"), "def f():\n    return 1\n").unwrap();
        let state = make_state(tmp.path(), None);
        let frames = run_generation(&state, req("a.py", [5, 9])).await;
        assert_eq!(frames.len(), 1);
        assert!(matches!(frames[0], GenFrame::Error { .. }));
    }

    #[tokio::test]
    async fn cache_miss_without_llm_yields_error_frame() {
        let tmp = TmpDir::new();
        std::fs::write(tmp.path().join("a.py"), "def f():\n    return 1\n").unwrap();
        let state = make_state(tmp.path(), None);
        let frames = run_generation(&state, req("a.py", [1, 2])).await;
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            GenFrame::Error { message } => assert!(message.contains("LLM not configured")),
            other => panic!("expected error frame, got {other:?}"),
        }
    }
}
