//! HTTP / WS routes.
//!
//! - `GET /api/project/tree`        -> { files: FileNode[] }
//! - `GET /api/file?path=<rel>`     -> { source: string }
//! - `GET /api/project/graph`       -> KnowledgeGraph | null   (S2, optional)
//! - `WS  /api/generate`            -> per-function streaming generation (S7)
//!
//! All handlers share an `Arc<AppState>` as axum state.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
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

/// The root-bound trio: file reader + optional knowledge graph + bypass cache.
/// All three are rebuilt together when the project root changes (U3 Open Folder),
/// so they live behind one lock and swap atomically.
struct ProjectCtx {
    reader: ProjectReader,
    graph: GraphLoader,
    cache: CacheStore,
}

/// Shared server state. The root-bound `project` swaps on Open Folder (U3); the
/// LLM proxy and the model/prompt versions are root-independent (the latter two
/// are kept so the cache can be rebuilt for a new root with the same key inputs).
pub struct AppState {
    /// Swappable per-project context (reader + graph + cache).
    project: RwLock<ProjectCtx>,
    /// LLM proxy; `None` when `OPENCODE_API_KEY` is unset (generate → error frame).
    llm: Option<LlmProxy>,
    /// Model id — feeds the cache key; needed to rebuild the cache on root swap.
    model: String,
    /// Prompt template version — feeds the cache key (ADR-0003).
    prompt_version: &'static str,
}

impl AppState {
    pub fn new(
        reader: ProjectReader,
        graph: GraphLoader,
        cache: CacheStore,
        llm: Option<LlmProxy>,
        model: String,
        prompt_version: &'static str,
    ) -> Self {
        Self {
            project: RwLock::new(ProjectCtx { reader, graph, cache }),
            llm,
            model,
            prompt_version,
        }
    }
}

type Shared = Arc<AppState>;

pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/api/project/tree", get(tree))
        .route("/api/file", get(file))
        .route("/api/project/graph", get(graph))
        .route("/api/project/open", post(open_folder))
        .route("/api/generate", get(generate_ws))
        .with_state(state)
}

#[derive(Serialize)]
struct TreeResponse {
    files: Vec<FileNode>,
}

async fn tree(State(state): State<Shared>) -> Json<TreeResponse> {
    Json(TreeResponse {
        files: state.project.read().unwrap().reader.list_files(),
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
    let result = state.project.read().unwrap().reader.read_file(&q.path);
    match result {
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
    Json(state.project.read().unwrap().graph.graph().cloned())
}

#[derive(Deserialize)]
struct OpenRequest {
    path: String,
}

#[derive(Serialize)]
struct OpenResponse {
    root: String,
}

/// `POST /api/project/open { path }` — switch the served project root (U3, single
/// root swap). Validates the path is an existing directory, then atomically swaps
/// in a fresh reader + graph + cache built for the new root (same model/prompt so
/// the cache key inputs are unchanged). Traversal protection is per-reader, so the
/// new reader enforces containment against the new root automatically.
async fn open_folder(State(state): State<Shared>, Json(req): Json<OpenRequest>) -> impl IntoResponse {
    let reader = match ProjectReader::new(PathBuf::from(&req.path)) {
        Ok(r) => r,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("cannot open directory: {e}")).into_response()
        }
    };
    let graph = GraphLoader::load(reader.root());
    let cache = CacheStore::new(reader.root(), &state.model, state.prompt_version);
    let root = reader.root().display().to_string();
    *state.project.write().unwrap() = ProjectCtx { reader, graph, cache };
    eprintln!("[open] switched project root to {root}");
    (StatusCode::OK, Json(OpenResponse { root })).into_response()
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

/// The synchronous (locked) phase of a generation: either fully resolved frames
/// (cache hit / error), or the prompt + span needed for an LLM call (cache miss).
enum GenStep {
    Ready(Vec<GenFrame>),
    NeedLlm {
        system: String,
        user: String,
        fn_source: String,
    },
}

/// Run one generation request to a complete frame sequence. A cache hit returns
/// before the LLM is ever consulted (the zero-token contract). On any failure a
/// single terminal `error` frame is returned. The project lock is held only for
/// the synchronous read/cache/assemble phase and is dropped before the LLM await
/// (so the future stays Send and a concurrent Open Folder can't deadlock).
async fn run_generation(state: &AppState, req: GenerateRequest) -> Vec<GenFrame> {
    let step = {
        let proj = state.project.read().unwrap();

        // 1. Resolve the function source span (deterministic; the cache key derives from it).
        let source = match proj.reader.read_file(&req.file_path) {
            Ok(s) => s,
            Err(ReadErr::NotFound) => return vec![err("file not found")],
            Err(ReadErr::Forbidden) => return vec![err("path outside project root")],
        };
        let Some(fn_source) = slice_span(&source, req.func.line_range) else {
            return vec![err("invalid lineRange for file")];
        };

        // 2. Cache: a hit returns the stored entry with zero token, no LLM (核心律).
        if let Some(entry) = proj.cache.get(&fn_source) {
            eprintln!("[generate] cache HIT {}#{} — zero token", req.file_path, req.func.name);
            GenStep::Ready(build_frames(true, entry.capsule, entry.lines))
        } else if state.llm.is_none() {
            // 3a. Miss but no LLM configured.
            GenStep::Ready(vec![err("LLM not configured: set OPENCODE_API_KEY")])
        } else {
            // 3b. Miss → assemble the prompt while we still hold the project lock.
            let ctx =
                assemble_gen_context(proj.graph.graph(), &req.file_path, &req.roster, &req.shared);
            let (system, user) = build_gen_prompt(&req.func, &fn_source, &req.key_lines, &ctx);
            GenStep::NeedLlm { system, user, fn_source }
        }
    }; // project lock dropped here — before any await.

    let (system, user, fn_source) = match step {
        GenStep::Ready(frames) => return frames,
        GenStep::NeedLlm { system, user, fn_source } => (system, user, fn_source),
    };

    let llm = state.llm.as_ref().expect("NeedLlm implies llm is Some");
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

    // 4. Persist for the next open (best-effort; a write failure must not fail the
    //    response). Re-acquire the lock briefly for the cache write.
    let entry = CapsuleEntry {
        capsule: capsule.clone(),
        lines: lines.clone(),
    };
    if let Err(e) = state.project.read().unwrap().cache.put(&fn_source, &entry) {
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
        AppState::new(
            ProjectReader::new(root.to_path_buf()).unwrap(),
            GraphLoader::load(root),
            CacheStore::new(root, "test-model", "p1"),
            llm,
            "test-model".to_string(),
            "p1",
        )
    }

    /// Swap the project root in place, the way `POST /api/project/open` does.
    fn swap_root(state: &AppState, root: &Path) {
        let reader = ProjectReader::new(root.to_path_buf()).unwrap();
        let graph = GraphLoader::load(root);
        let cache = CacheStore::new(root, &state.model, state.prompt_version);
        *state.project.write().unwrap() = ProjectCtx { reader, graph, cache };
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
            .project
            .read()
            .unwrap()
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

    #[test]
    fn root_swap_switches_the_listed_tree_and_readable_files() {
        // Two distinct project roots (U3 single-root swap).
        let one = TmpDir::new();
        std::fs::write(one.path().join("a.py"), "x = 1\n").unwrap();
        let two = TmpDir::new();
        std::fs::write(two.path().join("b.py"), "y = 2\n").unwrap();

        let state = make_state(one.path(), None);
        // Before swap: tree lists a.py, b.py is unreadable (outside root).
        let names: Vec<String> = state
            .project
            .read()
            .unwrap()
            .reader
            .list_files()
            .into_iter()
            .map(|f| f.path)
            .collect();
        assert_eq!(names, vec!["a.py"]);

        swap_root(&state, two.path());
        // After swap: tree lists b.py only; a.py is now outside the (new) root.
        let proj = state.project.read().unwrap();
        let names: Vec<String> = proj.reader.list_files().into_iter().map(|f| f.path).collect();
        assert_eq!(names, vec!["b.py"]);
        assert_eq!(proj.reader.read_file("b.py").unwrap(), "y = 2\n");
        assert!(matches!(proj.reader.read_file("a.py"), Err(ReadErr::NotFound)));
    }

    #[test]
    fn root_swap_traversal_protection_holds_on_new_root() {
        let one = TmpDir::new();
        std::fs::write(one.path().join("a.py"), "x = 1\n").unwrap();
        let two = TmpDir::new();
        std::fs::write(two.path().join("b.py"), "y = 2\n").unwrap();

        let state = make_state(one.path(), None);
        swap_root(&state, two.path());
        // Traversal / absolute paths are still refused against the new root.
        let proj = state.project.read().unwrap();
        assert!(matches!(proj.reader.read_file("../a.py"), Err(ReadErr::Forbidden)));
        assert!(matches!(
            proj.reader.read_file("b.py/../../etc"),
            Err(ReadErr::Forbidden)
        ));
    }
}
