//! HTTP / WS routes.
//!
//! - `GET /api/project/tree`        -> { files: FileNode[] }
//! - `GET /api/file?path=<rel>`     -> { source: string }
//! - `GET /api/project/graph`       -> KnowledgeGraph | null   (S2, optional)
//! - `WS  /api/generate`            -> per-function streaming generation (S7)
//!
//! All handlers share an `Arc<AppState>` as axum state.

use std::collections::BTreeMap;
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
use crate::context_assembler::{
    assemble_gen_context, build_explain_line_prompt, build_gen_prompt, build_query_planning_prompt,
    build_query_prompt, cross_file_targets, query_degraded_names, slice_cross_file_sources,
    slice_requested_sources, slice_span, CrossFileTarget, FunctionSpan, GenContext, QueryFocus,
    SharedContext, QUERY_FETCH_BUDGET_CHARS,
};
use crate::graph_loader::{GraphLoader, KnowledgeGraph};
use crate::llm_proxy::{parse_fetch_plan, parse_generation, parse_line_annotation, LlmProxy, SseDecoder};
use crate::project_reader::{FileNode, ProjectReader, ReadErr};
use crate::settings::{mask_key, rewrite_env, LlmConfig};
use futures_util::StreamExt;

/// The root-bound trio: file reader + optional knowledge graph + bypass cache.
/// All three are rebuilt together when the project root changes (U3 Open Folder),
/// so they live behind one lock and swap atomically.
struct ProjectCtx {
    reader: ProjectReader,
    graph: GraphLoader,
    cache: CacheStore,
}

/// Shared server state. The root-bound `project` swaps on Open Folder (U3); the
/// LLM backend swaps on a settings change (U5a, ADR-0018). `prompt_version` is a
/// build constant feeding the cache key.
pub struct AppState {
    /// Swappable per-project context (reader + graph + cache).
    project: RwLock<ProjectCtx>,
    /// Runtime-editable LLM backend (U5a, ADR-0018): config (source of truth,
    /// holds the secret key in memory) + the derived proxy (`None` when no key).
    /// Behind a lock so the settings panel can hot-swap it; the proxy is `Arc`'d
    /// so handlers clone it out and use it across `.await` without holding the lock.
    llm: RwLock<LlmState>,
    /// Resolved `.env` path — where a settings change is persisted (U5a).
    env_path: PathBuf,
    /// Prompt template version — feeds the cache key (ADR-0003).
    prompt_version: &'static str,
}

/// The runtime LLM state behind `AppState.llm`. `config.model` feeds the cache
/// key, so it is kept in lock-step with `proxy`'s model on every swap.
struct LlmState {
    config: LlmConfig,
    proxy: Option<Arc<LlmProxy>>,
}

impl AppState {
    pub fn new(
        reader: ProjectReader,
        graph: GraphLoader,
        cache: CacheStore,
        llm_config: LlmConfig,
        env_path: PathBuf,
        prompt_version: &'static str,
    ) -> Self {
        let proxy = LlmProxy::from_config(&llm_config).map(Arc::new);
        Self {
            project: RwLock::new(ProjectCtx { reader, graph, cache }),
            llm: RwLock::new(LlmState { config: llm_config, proxy }),
            env_path,
            prompt_version,
        }
    }

    /// Snapshot the current proxy (cheap `Arc` clone), releasing the lock at once
    /// so it can be used across `.await` without blocking a settings swap.
    fn llm_proxy(&self) -> Option<Arc<LlmProxy>> {
        self.llm.read().unwrap().proxy.clone()
    }

    /// The model id that feeds the cache key (kept in lock-step with the proxy).
    fn model(&self) -> String {
        self.llm.read().unwrap().config.model.clone()
    }
}

type Shared = Arc<AppState>;

pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/api/project/tree", get(tree))
        .route("/api/file", get(file))
        .route("/api/project/graph", get(graph))
        .route("/api/project/open", post(open_folder))
        .route("/api/project/pick", post(pick_folder))
        .route("/api/settings/llm", get(get_llm_settings).post(put_llm_settings))
        .route("/api/settings/llm/test", post(test_llm_settings))
        .route("/api/explain-line", post(explain_line))
        .route("/api/generate", get(generate_ws))
        .route("/api/query", get(query_ws))
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
    let cache = CacheStore::new(reader.root(), state.model(), state.prompt_version);
    let root = reader.root().display().to_string();
    *state.project.write().unwrap() = ProjectCtx { reader, graph, cache };
    eprintln!("[open] switched project root to {root}");
    (StatusCode::OK, Json(OpenResponse { root })).into_response()
}

#[derive(Serialize)]
struct PickResponse {
    /// Chosen absolute path, or null when the user cancelled the dialog.
    path: Option<String>,
}

/// `POST /api/project/pick` — pop a native OS folder picker and return the chosen
/// absolute path (or null on cancel). The browser sandbox can't hand a
/// server-side absolute path to the backend, so the *backend* — which runs on the
/// user's own machine (ADR-0010 local topology) — opens the dialog; the frontend
/// then feeds the returned path to `/api/project/open`. The dialog is blocking, so
/// it runs on a dedicated thread to keep the async runtime free.
async fn pick_folder() -> impl IntoResponse {
    let picked = tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("选择项目文件夹")
            .pick_folder()
            .map(|p| p.display().to_string())
    })
    .await
    .unwrap_or(None);
    Json(PickResponse { path: picked })
}

// — Settings: runtime LLM backend config (U5a, ADR-0018) —
//
// GET  returns the non-secret config + a *masked* key status (write-only key: the
//      full key never leaves the backend).
// POST applies new values: hot-rebuilds the in-memory proxy, rebuilds the cache
//      pointer if the model changed (model feeds the cache key, ADR-0003), and
//      writes the three lines back to `.env` so the change survives a restart. An
//      omitted/empty `apiKey` keeps the existing key (so the UI never has to echo
//      it back to overwrite the rest).

#[derive(Serialize)]
struct LlmSettingsResponse {
    #[serde(rename = "baseUrl")]
    base_url: String,
    model: String,
    /// "set" | "unset" — whether a key is configured.
    #[serde(rename = "keyStatus")]
    key_status: &'static str,
    /// Masked tail (`···last4`) or null — the only key derivative sent to the UI.
    #[serde(rename = "keyHint")]
    key_hint: Option<String>,
}

impl LlmSettingsResponse {
    fn of(cfg: &LlmConfig) -> Self {
        Self {
            base_url: cfg.base_url.clone(),
            model: cfg.model.clone(),
            key_status: if cfg.key_set() { "set" } else { "unset" },
            key_hint: mask_key(&cfg.api_key),
        }
    }
}

#[derive(Deserialize)]
struct LlmSettingsRequest {
    #[serde(rename = "baseUrl")]
    base_url: String,
    model: String,
    /// Omitted or empty → keep the current key (write-only).
    #[serde(rename = "apiKey", default)]
    api_key: Option<String>,
}

async fn get_llm_settings(State(state): State<Shared>) -> Json<LlmSettingsResponse> {
    let s = state.llm.read().unwrap();
    Json(LlmSettingsResponse::of(&s.config))
}

async fn put_llm_settings(
    State(state): State<Shared>,
    Json(req): Json<LlmSettingsRequest>,
) -> impl IntoResponse {
    if req.base_url.trim().is_empty() || req.model.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "baseUrl and model are required").into_response();
    }
    let cfg = apply_llm_settings(&state, req.base_url, req.model, req.api_key);

    // Persist to `.env` (best-effort: a write failure must not fail the request —
    // the change is already live in memory). Reads the current file so unrelated
    // lines/comments survive; absent file → write just the three lines.
    let existing = std::fs::read_to_string(&state.env_path).unwrap_or_default();
    if let Err(e) = std::fs::write(&state.env_path, rewrite_env(&existing, &cfg)) {
        eprintln!("[settings] warning: .env write-back failed ({}): {e}", state.env_path.display());
    }

    (StatusCode::OK, Json(LlmSettingsResponse::of(&cfg))).into_response()
}

/// Core of `put_llm_settings`, factored out for deterministic testing (no axum /
/// no file IO): swap the in-memory config + proxy, and rebuild the cache pointer
/// when the model changed (model feeds the cache key, ADR-0003 — mirrors the root
/// swap in `open_folder`). An empty/omitted `api_key` keeps the existing key.
/// Returns the applied config (with the resolved key) for the response/write-back.
fn apply_llm_settings(
    state: &AppState,
    base_url: String,
    model: String,
    api_key: Option<String>,
) -> LlmConfig {
    let (cfg, model_changed) = {
        let mut s = state.llm.write().unwrap();
        let old_model = s.config.model.clone();
        let key = match api_key {
            Some(k) if !k.trim().is_empty() => k,
            _ => s.config.api_key.clone(), // keep existing (write-only)
        };
        s.config = LlmConfig { base_url, model, api_key: key };
        s.proxy = LlmProxy::from_config(&s.config).map(Arc::new);
        (s.config.clone(), s.config.model != old_model)
    }; // llm lock dropped before touching the project lock (no nested ordering).

    // Model change → re-point the cache so new generations key under the new model
    // (old-model entries simply miss and regenerate, ADR-0003).
    if model_changed {
        let root = state.project.read().unwrap().reader.root().to_path_buf();
        let cache = CacheStore::new(&root, &cfg.model, state.prompt_version);
        state.project.write().unwrap().cache = cache;
    }
    cfg
}

// — Settings: test the LLM connection (U5c, ADR-0018) —
//
// POST /api/settings/llm/test makes one minimal completion with the *given*
// values so the user can verify a backend before saving. It is purely a probe:
// it never writes `.env`, never touches the runtime proxy, never retries. An
// omitted/empty `apiKey` reuses the currently-stored key (write-only — the UI
// need not echo the secret to test the other fields).

#[derive(Deserialize)]
struct LlmTestRequest {
    #[serde(rename = "baseUrl")]
    base_url: String,
    model: String,
    #[serde(rename = "apiKey", default)]
    api_key: Option<String>,
}

#[derive(Serialize)]
struct LlmTestResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Which key a connection test should use: an explicit non-empty `req_key`, else
/// the currently-stored key. Mirrors the write-only resolution in
/// `apply_llm_settings` so a test reflects what a save would actually use.
fn resolve_test_key(req_key: Option<String>, current: &str) -> String {
    match req_key {
        Some(k) if !k.trim().is_empty() => k,
        _ => current.to_string(),
    }
}

async fn test_llm_settings(
    State(state): State<Shared>,
    Json(req): Json<LlmTestRequest>,
) -> Json<LlmTestResponse> {
    if req.base_url.trim().is_empty() || req.model.trim().is_empty() {
        return Json(LlmTestResponse {
            ok: false,
            error: Some("baseUrl 和 model 不能为空".to_string()),
        });
    }
    // Snapshot the stored key under the read lock, then drop it before the await.
    let key = {
        let s = state.llm.read().unwrap();
        resolve_test_key(req.api_key, &s.config.api_key)
    };
    let cfg = LlmConfig {
        base_url: req.base_url,
        model: req.model,
        api_key: key,
    };
    let Some(proxy) = LlmProxy::from_config(&cfg) else {
        return Json(LlmTestResponse {
            ok: false,
            error: Some("未配置 API Key".to_string()),
        });
    };
    // Minimal probe: a one-token reply is enough to prove the endpoint + key +
    // model are reachable. We discard the content; only success/failure matters.
    match proxy.complete("你是连接测试助手，只回复 ok。", "ping").await {
        Ok(_) => Json(LlmTestResponse { ok: true, error: None }),
        Err(e) => Json(LlmTestResponse {
            ok: false,
            error: Some(e.to_string()),
        }),
    }
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
    // Snapshot the proxy once (Arc clone, lock released) so it survives the project
    // lock drop and the await below, and a concurrent settings swap can't tear it.
    let llm = state.llm_proxy();
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
        } else if llm.is_none() {
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

    let llm = llm.as_ref().expect("NeedLlm implies llm is Some");
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

// — POST /api/explain-line — manual single-line fill (S9) —
//
// The long-tail companion to /api/generate: a function's capsule + key lines are
// generated on open, but NON-key lines stay bare by design (CONTEXT 重点行 vs
// 手动补行). This endpoint explains one such line on demand, returning a single
// `LineAnnotation`. Unlike generate it's a plain request/response (one line, no
// streaming). A cache hit returns before the LLM is consulted (zero-token, like
// run_generation); the line entry is keyed by `line_key` so it never aliases the
// function's capsule entry.

#[derive(Deserialize)]
struct ExplainLineRequest {
    #[serde(rename = "filePath")]
    file_path: String,
    #[serde(rename = "fn")]
    func: FunctionSpan,
    #[serde(rename = "lineNumber")]
    line_number: u32,
    #[serde(default)]
    roster: Vec<String>,
    #[serde(default)]
    shared: SharedContext,
}

/// Resolve one manual-line annotation to either a finished line (cache hit / the
/// LLM result) or an error mapped to an HTTP status. Mirrors `run_generation`'s
/// lock discipline: the project lock is held only for the synchronous
/// read/slice/cache/assemble phase and dropped before the LLM await.
async fn run_explain_line(
    state: &AppState,
    req: ExplainLineRequest,
) -> Result<LineAnnotation, (StatusCode, String)> {
    // Snapshot the proxy once (see run_generation) so it survives the lock drop.
    let llm = state.llm_proxy();
    enum Step {
        Ready(LineAnnotation),
        NeedLlm {
            system: String,
            user: String,
            fn_source: String,
        },
    }

    let step = {
        let proj = state.project.read().unwrap();

        let source = match proj.reader.read_file(&req.file_path) {
            Ok(s) => s,
            Err(ReadErr::NotFound) => return Err((StatusCode::NOT_FOUND, "file not found".into())),
            Err(ReadErr::Forbidden) => {
                return Err((StatusCode::FORBIDDEN, "path outside project root".into()))
            }
        };
        let Some(fn_source) = slice_span(&source, req.func.line_range) else {
            return Err((StatusCode::BAD_REQUEST, "invalid lineRange for file".into()));
        };
        // The target line must sit inside the enclosing function span.
        let [start, end] = req.func.line_range;
        if req.line_number < start || req.line_number > end {
            return Err((StatusCode::BAD_REQUEST, "lineNumber outside function".into()));
        }

        if let Some(line) = proj.cache.get_line(&fn_source, req.line_number) {
            eprintln!(
                "[explain-line] cache HIT {}#{} L{} — zero token",
                req.file_path, req.func.name, req.line_number
            );
            Step::Ready(line)
        } else if llm.is_none() {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "LLM not configured: set OPENCODE_API_KEY".into(),
            ));
        } else {
            let ctx =
                assemble_gen_context(proj.graph.graph(), &req.file_path, &req.roster, &req.shared);
            let (system, user) =
                build_explain_line_prompt(&req.func, &fn_source, req.line_number, &ctx);
            Step::NeedLlm { system, user, fn_source }
        }
    }; // project lock dropped here — before any await.

    let (system, user, fn_source) = match step {
        Step::Ready(line) => return Ok(line),
        Step::NeedLlm { system, user, fn_source } => (system, user, fn_source),
    };

    let llm = llm.as_ref().expect("NeedLlm implies llm is Some");
    eprintln!(
        "[explain-line] cache MISS {}#{} L{} — calling LLM ({})",
        req.file_path, req.func.name, req.line_number, llm.model
    );
    let content = match llm.complete(&system, &user).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[explain-line] LLM error {}#{} L{}: {e}",
                req.file_path, req.func.name, req.line_number
            );
            return Err((StatusCode::BAD_GATEWAY, format!("LLM error: {e}")));
        }
    };
    let line = match parse_line_annotation(&content, &req.func.id, req.line_number) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "[explain-line] LLM parse error {}#{} L{}: {e}",
                req.file_path, req.func.name, req.line_number
            );
            return Err((StatusCode::BAD_GATEWAY, format!("LLM parse error: {e}")));
        }
    };

    // Persist for the next open (best-effort; a write failure must not fail the
    // response). Re-acquire the lock briefly for the cache write.
    if let Err(e) = state
        .project
        .read()
        .unwrap()
        .cache
        .put_line(&fn_source, req.line_number, &line)
    {
        eprintln!("[explain-line] warning: cache put failed: {e}");
    }

    Ok(line)
}

/// `POST /api/explain-line { filePath, fn, lineNumber, roster?, shared? }` →
/// `LineAnnotation` (200), or a status + message on error (S9).
async fn explain_line(
    State(state): State<Shared>,
    Json(req): Json<ExplainLineRequest>,
) -> impl IntoResponse {
    match run_explain_line(&state, req).await {
        Ok(line) => (StatusCode::OK, Json(line)).into_response(),
        Err((code, msg)) => (code, msg).into_response(),
    }
}

// — WS /api/query — streaming follow-up Q&A over the current file (S10a) —
//
// Unlike /api/generate (structured capsule/line frames from a single non-streaming
// call), a query answer is free-form markdown streamed token-by-token. Protocol:
//
//   ok   : delta×N → done
//   fail : error                  (terminal, no done)
//
// Context is assembled per ADR-0006 *default tier*: the whole file at summary
// granularity (file summary + every function's capsule summary + edges + cross-file
// one-liners) plus the focused function at source granularity. Over-window
// degradation (S10a-降级) and cross-file ephemeral fetch (S10c) are NOT wired here.

#[derive(Deserialize)]
struct CapsuleSummary {
    #[serde(default)]
    name: String,
    #[serde(default)]
    summary: String,
}

#[derive(Deserialize)]
struct QueryRequest {
    #[serde(rename = "reqId", default)]
    req_id: String,
    #[serde(rename = "filePath")]
    file_path: String,
    question: String,
    /// The function the user is focused on (its source is zoomed in); None = file-level.
    #[serde(default)]
    focus: Option<FunctionSpan>,
    #[serde(default)]
    roster: Vec<String>,
    /// Per-function line ranges so the backend can slice a function's source by name
    /// for on-demand fetch (S10a-追源, ADR-0017). Optional; absent (older client) →
    /// fetch is skipped and a degraded query just answers over the trimmed context.
    #[serde(rename = "rosterSpans", default)]
    roster_spans: Vec<FunctionSpan>,
    /// Per-function capsule summaries the frontend already holds (ghost store).
    #[serde(default)]
    capsules: Vec<CapsuleSummary>,
    #[serde(default)]
    shared: SharedContext,
}

/// One outbound frame on the `/api/query` socket (kebab `kind`: `delta` / `done` /
/// `error`); `reqId` injected by the sender.
#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum QueryFrame {
    Delta { text: String },
    Done,
    Error { message: String },
}

/// The synchronous (locked) phase of a query. Either an early terminal error; a
/// `Direct` single-call prompt (nothing degraded); or a `Degraded` two-phase plan
/// (S10a-追源, ADR-0017) carrying the planning prompt plus everything needed to
/// re-assemble the answer prompt after the model names the sources it wants — all
/// owned so it survives the lock drop and the planning await.
enum QueryPlan {
    Err(String),
    Direct {
        system: String,
        user: String,
    },
    /// Boxed so the (rare) two-phase variant doesn't bloat every `QueryPlan` value
    /// (`clippy::large_enum_variant`) — the common path is `Direct`.
    Degraded(Box<DegradedPlan>),
}

/// Everything `run_query` needs to run the two-phase fetch after the lock drops —
/// all owned so it survives the lock drop and the planning await.
struct DegradedPlan {
    planning_system: String,
    planning_user: String,
    file_source: String,
    ctx: GenContext,
    capsules: Vec<(String, String)>,
    focus: Option<(String, u32, String)>,
    /// Same-file name-only functions the model may fetch (S10a-追源).
    fetchable: Vec<String>,
    /// Cross-file callees the model may fetch (S10c, ADR-0007 修订).
    cross_targets: Vec<CrossFileTarget>,
    /// `file_path` → full source for every distinct cross-file target file,
    /// read under the lock so `run_query` can slice after the lock drops.
    cross_sources: BTreeMap<String, String>,
}

/// Assemble the query plan while holding the project lock, then hand it back so the
/// caller can run the LLM call(s) *after* the lock is dropped (no lock across await,
/// mirroring `run_generation`). When the degradation ladder reduced same-file
/// functions to name-only AND we have spans to slice them, returns a two-phase plan;
/// otherwise a direct single-call prompt.
fn prepare_query(state: &AppState, req: &QueryRequest) -> QueryPlan {
    let proj = state.project.read().unwrap();

    let source = match proj.reader.read_file(&req.file_path) {
        Ok(s) => s,
        Err(ReadErr::NotFound) => return QueryPlan::Err("file not found".into()),
        Err(ReadErr::Forbidden) => return QueryPlan::Err("path outside project root".into()),
    };

    // The focused function zoomed to source granularity (owned so it survives the
    // lock drop / planning await). Its name rides along for prioritization + fetch.
    let focus: Option<(String, u32, String)> = match &req.focus {
        Some(f) => match slice_span(&source, f.line_range) {
            Some(src) => Some((src, f.line_range[0], f.name.clone())),
            None => return QueryPlan::Err("invalid lineRange for focus".into()),
        },
        None => None,
    };

    if state.llm_proxy().is_none() {
        return QueryPlan::Err("LLM not configured: set OPENCODE_API_KEY".into());
    }

    let ctx = assemble_gen_context(proj.graph.graph(), &req.file_path, &req.roster, &req.shared);
    let capsules: Vec<(String, String)> = req
        .capsules
        .iter()
        .map(|c| (c.name.clone(), c.summary.clone()))
        .collect();
    let focus_ref = focus.as_ref().map(|(s, n, name)| QueryFocus {
        source: s.as_str(),
        start_line: *n,
        name: name.as_str(),
    });

    // Same-file functions degraded to name-only that we can actually slice (have a
    // span) form the same-file fetchable set (S10a-追源).
    let degraded = query_degraded_names(&req.question, &capsules, focus_ref.as_ref(), &ctx);
    let same_file_fetchable: Vec<String> = degraded
        .into_iter()
        .filter(|name| req.roster_spans.iter().any(|s| &s.name == name))
        .collect();

    // Cross-file callees the graph can locate (S10c, ADR-0007 修订). Read each
    // distinct target file's source now, under the lock, so run_query can slice
    // after the lock drops (mirroring `file_source` — no lock across await). A
    // target whose file can't be read is dropped (never offer a name we can't
    // honor). Pure read: no cache write, no activation — 目标文件事后仍真空.
    let cross_all = cross_file_targets(proj.graph.graph(), &req.file_path, &req.roster);
    let mut cross_sources: BTreeMap<String, String> = BTreeMap::new();
    let mut cross_targets: Vec<CrossFileTarget> = Vec::new();
    for t in cross_all {
        let have = cross_sources.contains_key(&t.file_path)
            || match proj.reader.read_file(&t.file_path) {
                Ok(s) => {
                    cross_sources.insert(t.file_path.clone(), s);
                    true
                }
                Err(_) => false,
            };
        if have {
            cross_targets.push(t);
        }
    }

    // fetchable for the planning prompt = same-file degraded ∪ cross-file callees.
    // The two name pools are disjoint (cross excludes roster names), so the model's
    // `{"need":[...]}` resolves each name to exactly one pool in run_query.
    // Non-empty (either pool) → two-phase fetch; empty → single streaming call
    // (ADR-0017 修订: 门控由「仅降级时」扩为「fetchable 非空即触发」).
    let mut fetchable = same_file_fetchable.clone();
    fetchable.extend(cross_targets.iter().map(|t| t.name.clone()));

    if fetchable.is_empty() {
        let (system, user) = build_query_prompt(&req.question, &capsules, focus_ref, &ctx, &[]);
        return QueryPlan::Direct { system, user };
    }

    let (planning_system, planning_user) =
        build_query_planning_prompt(&req.question, &capsules, focus_ref, &ctx, &fetchable);
    QueryPlan::Degraded(Box::new(DegradedPlan {
        planning_system,
        planning_user,
        file_source: source,
        ctx,
        capsules,
        focus,
        fetchable: same_file_fetchable,
        cross_targets,
        cross_sources,
    }))
}

/// Run one query request, streaming `delta` frames to the socket then `done`. On a
/// pre-LLM error or an LLM/stream failure, a single terminal `error` frame is sent
/// and the socket is left alive for the next question. `Err(())` means the peer is
/// gone (a send failed) and the caller should stop reading.
async fn run_query(
    state: &AppState,
    req: QueryRequest,
    socket: &mut WebSocket,
    req_id: &str,
) -> Result<(), ()> {
    // Snapshot the proxy once (Arc clone, lock released) and reuse it for both the
    // planning and streaming awaits, so a settings swap mid-query can't tear it.
    let llm_proxy = state.llm_proxy();
    let (system, user) = match prepare_query(state, &req) {
        QueryPlan::Err(msg) => {
            return send_query_frame(socket, req_id, &QueryFrame::Error { message: msg })
                .await
                .map_err(|_| ());
        }
        QueryPlan::Direct { system, user } => (system, user),
        QueryPlan::Degraded(plan) => {
            let DegradedPlan {
                planning_system,
                planning_user,
                file_source,
                ctx,
                capsules,
                focus,
                fetchable,
                cross_targets,
                cross_sources,
            } = *plan;
            // Phase 1: planning (non-streaming). A failed call or unparseable plan
            // degrades to answering with no extra source — the fetch must never fail
            // the query (ADR-0017). One round only: no recursion.
            let llm = llm_proxy.as_ref().expect("Degraded implies llm is Some");
            eprintln!(
                "[query] {} — planning fetch ({} same-file, {} cross-file)",
                req.file_path,
                fetchable.len(),
                cross_targets.len()
            );
            let need = match llm.complete(&planning_system, &planning_user).await {
                Ok(c) => parse_fetch_plan(&c),
                Err(e) => {
                    eprintln!("[query] planning failed {}: {e} — answering without fetch", req.file_path);
                    Vec::new()
                }
            };
            // Same-file fetch first, then cross-file (S10c) with the *remaining* shared
            // budget, so the appended sources of both kinds stay within one bound
            // (ADR-0017 修订: 共享 QUERY_FETCH_BUDGET_CHARS).
            let mut extra = slice_requested_sources(
                &file_source,
                &req.roster_spans,
                &need,
                &fetchable,
                QUERY_FETCH_BUDGET_CHARS,
            );
            let used: usize = extra
                .iter()
                .map(|(n, s)| n.chars().count() + s.chars().count() + 4)
                .sum();
            let cross = slice_cross_file_sources(
                &cross_targets,
                &cross_sources,
                &need,
                QUERY_FETCH_BUDGET_CHARS.saturating_sub(used),
            );
            extra.extend(cross);
            if !extra.is_empty() {
                let got: Vec<&str> = extra.iter().map(|(n, _)| n.as_str()).collect();
                eprintln!("[query] {} — fetched sources: {}", req.file_path, got.join(", "));
            }
            let focus_ref = focus.as_ref().map(|(s, n, name)| QueryFocus {
                source: s.as_str(),
                start_line: *n,
                name: name.as_str(),
            });
            build_query_prompt(&req.question, &capsules, focus_ref, &ctx, &extra)
        }
    };

    let llm = llm_proxy.as_ref().expect("a non-Err plan implies llm is Some");
    eprintln!("[query] {} — streaming ({})", req.file_path, llm.model);
    let resp = match llm.open_chat_stream(&system, &user).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[query] LLM error {}: {e}", req.file_path);
            return send_query_frame(
                socket,
                req_id,
                &QueryFrame::Error { message: format!("LLM error: {e}") },
            )
            .await
            .map_err(|_| ());
        }
    };

    let mut stream = resp.bytes_stream();
    let mut decoder = SseDecoder::new();
    while let Some(chunk) = stream.next().await {
        let bytes = match chunk {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[query] stream error {}: {e}", req.file_path);
                return send_query_frame(
                    socket,
                    req_id,
                    &QueryFrame::Error { message: format!("LLM stream error: {e}") },
                )
                .await
                .map_err(|_| ());
            }
        };
        for delta in decoder.push(&String::from_utf8_lossy(&bytes)) {
            send_query_frame(socket, req_id, &QueryFrame::Delta { text: delta })
                .await
                .map_err(|_| ())?;
        }
    }

    send_query_frame(socket, req_id, &QueryFrame::Done)
        .await
        .map_err(|_| ())
}

async fn query_ws(ws: WebSocketUpgrade, State(state): State<Shared>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_query_socket(socket, state))
}

/// Drive one `/api/query` socket: read question frames, stream each answer back
/// tagged with the request's `reqId`.
async fn handle_query_socket(mut socket: WebSocket, state: Shared) {
    while let Some(Ok(msg)) = socket.recv().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };

        let req: QueryRequest = match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(e) => {
                let _ = send_query_frame(
                    &mut socket,
                    "",
                    &QueryFrame::Error { message: format!("bad request: {e}") },
                )
                .await;
                continue;
            }
        };
        let req_id = req.req_id.clone();
        if run_query(&state, req, &mut socket, &req_id).await.is_err() {
            return; // peer gone
        }
    }
}

/// Serialize a query frame and inject `reqId` before sending it as a text message.
async fn send_query_frame(
    socket: &mut WebSocket,
    req_id: &str,
    frame: &QueryFrame,
) -> Result<(), axum::Error> {
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

    /// Build a test state. `api_key` empty → proxy `None` (no-LLM paths); a
    /// non-empty key → proxy `Some` (without ever calling out, since tests assert
    /// cache/settings behaviour, not network).
    fn make_state(root: &Path, api_key: &str) -> AppState {
        let cfg = LlmConfig {
            base_url: "https://test/v1".into(),
            model: "test-model".into(),
            api_key: api_key.into(),
        };
        AppState::new(
            ProjectReader::new(root.to_path_buf()).unwrap(),
            GraphLoader::load(root),
            CacheStore::new(root, &cfg.model, "p1"),
            cfg,
            root.join(".env"),
            "p1",
        )
    }

    /// Swap the project root in place, the way `POST /api/project/open` does.
    fn swap_root(state: &AppState, root: &Path) {
        let reader = ProjectReader::new(root.to_path_buf()).unwrap();
        let graph = GraphLoader::load(root);
        let cache = CacheStore::new(root, state.model(), state.prompt_version);
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
        let state = make_state(tmp.path(), "");
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
        let state = make_state(tmp.path(), "");
        let frames = run_generation(&state, req("a.py", [5, 9])).await;
        assert_eq!(frames.len(), 1);
        assert!(matches!(frames[0], GenFrame::Error { .. }));
    }

    #[tokio::test]
    async fn cache_miss_without_llm_yields_error_frame() {
        let tmp = TmpDir::new();
        std::fs::write(tmp.path().join("a.py"), "def f():\n    return 1\n").unwrap();
        let state = make_state(tmp.path(), "");
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

        let state = make_state(one.path(), "");
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

        let state = make_state(one.path(), "");
        swap_root(&state, two.path());
        // Traversal / absolute paths are still refused against the new root.
        let proj = state.project.read().unwrap();
        assert!(matches!(proj.reader.read_file("../a.py"), Err(ReadErr::Forbidden)));
        assert!(matches!(
            proj.reader.read_file("b.py/../../etc"),
            Err(ReadErr::Forbidden)
        ));
    }

    // — S9 explain-line —

    fn explain_req(file_path: &str, line_range: [u32; 2], line_number: u32) -> ExplainLineRequest {
        ExplainLineRequest {
            file_path: file_path.into(),
            func: FunctionSpan {
                id: "f#1".into(),
                name: "f".into(),
                line_range,
            },
            line_number,
            roster: vec![],
            shared: SharedContext::default(),
        }
    }

    #[tokio::test]
    async fn explain_line_cache_hit_returns_line_with_zero_llm() {
        let tmp = TmpDir::new();
        std::fs::write(tmp.path().join("a.py"), "def f():\n    return 1\n").unwrap();
        // llm: None — yet a pre-populated line cache must still succeed (zero token).
        let state = make_state(tmp.path(), "");
        let fn_source = "def f():\n    return 1";
        state
            .project
            .read()
            .unwrap()
            .cache
            .put_line(fn_source, 2, &line("f#1", 2))
            .unwrap();

        let got = run_explain_line(&state, explain_req("a.py", [1, 2], 2)).await;
        assert_eq!(got.unwrap(), line("f#1", 2));
    }

    #[tokio::test]
    async fn explain_line_invalid_line_range_is_bad_request() {
        let tmp = TmpDir::new();
        std::fs::write(tmp.path().join("a.py"), "def f():\n    return 1\n").unwrap();
        let state = make_state(tmp.path(), "");
        let err = run_explain_line(&state, explain_req("a.py", [5, 9], 5)).await.unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn explain_line_outside_function_is_bad_request() {
        let tmp = TmpDir::new();
        std::fs::write(tmp.path().join("a.py"), "def f():\n    return 1\n").unwrap();
        let state = make_state(tmp.path(), "");
        // Line 9 is outside the function span [1, 2].
        let err = run_explain_line(&state, explain_req("a.py", [1, 2], 9)).await.unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("outside function"));
    }

    #[tokio::test]
    async fn explain_line_miss_without_llm_is_service_unavailable() {
        let tmp = TmpDir::new();
        std::fs::write(tmp.path().join("a.py"), "def f():\n    return 1\n").unwrap();
        let state = make_state(tmp.path(), "");
        let err = run_explain_line(&state, explain_req("a.py", [1, 2], 2)).await.unwrap_err();
        assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);
        assert!(err.1.contains("LLM not configured"));
    }

    #[tokio::test]
    async fn explain_line_missing_file_is_not_found() {
        let tmp = TmpDir::new();
        let state = make_state(tmp.path(), "");
        let err = run_explain_line(&state, explain_req("nope.py", [1, 2], 1)).await.unwrap_err();
        assert_eq!(err.0, StatusCode::NOT_FOUND);
    }

    // — S10a /api/query —

    fn query_req(file_path: &str, focus: Option<[u32; 2]>) -> QueryRequest {
        QueryRequest {
            req_id: "q1".into(),
            file_path: file_path.into(),
            question: "这个函数做什么？".into(),
            focus: focus.map(|lr| FunctionSpan {
                id: "f#1".into(),
                name: "f".into(),
                line_range: lr,
            }),
            roster: vec![],
            roster_spans: vec![],
            capsules: vec![],
            shared: SharedContext::default(),
        }
    }

    #[test]
    fn query_frame_serializes_with_kebab_kind() {
        let v = serde_json::to_value(QueryFrame::Delta { text: "你好".into() }).unwrap();
        assert_eq!(v["kind"], "delta");
        assert_eq!(v["text"], "你好");
        let v = serde_json::to_value(QueryFrame::Done).unwrap();
        assert_eq!(v["kind"], "done");
        let v = serde_json::to_value(QueryFrame::Error { message: "x".into() }).unwrap();
        assert_eq!(v["kind"], "error");
        assert_eq!(v["message"], "x");
    }

    #[test]
    fn prepare_query_without_llm_is_an_error() {
        let tmp = TmpDir::new();
        std::fs::write(tmp.path().join("a.py"), "def f():\n    return 1\n").unwrap();
        let state = make_state(tmp.path(), "");
        match prepare_query(&state, &query_req("a.py", Some([1, 2]))) {
            QueryPlan::Err(msg) => assert!(msg.contains("LLM not configured")),
            _ => panic!("expected Err without llm"),
        }
    }

    #[test]
    fn prepare_query_rejects_invalid_focus_range() {
        let tmp = TmpDir::new();
        std::fs::write(tmp.path().join("a.py"), "def f():\n    return 1\n").unwrap();
        let state = make_state(tmp.path(), "");
        // Line 9 is out of bounds for the 2-line file → focus slice fails *before* the
        // llm check, so this reports the focus error rather than "LLM not configured".
        match prepare_query(&state, &query_req("a.py", Some([1, 9]))) {
            QueryPlan::Err(msg) => assert!(msg.contains("invalid lineRange for focus")),
            _ => panic!("expected Err on bad focus"),
        }
    }

    #[test]
    fn prepare_query_missing_file_is_an_error() {
        let tmp = TmpDir::new();
        let state = make_state(tmp.path(), "");
        match prepare_query(&state, &query_req("nope.py", None)) {
            QueryPlan::Err(msg) => assert!(msg.contains("file not found")),
            _ => panic!("expected Err for missing file"),
        }
    }

    // — U5a settings (ADR-0018) —

    #[test]
    fn settings_response_masks_the_key() {
        let set = LlmConfig { base_url: "b".into(), model: "m".into(), api_key: "sk-xyz9999".into() };
        let r = LlmSettingsResponse::of(&set);
        assert_eq!(r.key_status, "set");
        assert_eq!(r.key_hint.as_deref(), Some("···9999"));

        let unset = LlmConfig { base_url: "b".into(), model: "m".into(), api_key: "".into() };
        let r2 = LlmSettingsResponse::of(&unset);
        assert_eq!(r2.key_status, "unset");
        assert_eq!(r2.key_hint, None);
    }

    // — U5c test-connection key resolution (ADR-0018) —

    #[test]
    fn resolve_test_key_uses_typed_key_or_falls_back_to_current() {
        // A typed, non-empty key wins (testing a brand-new backend).
        assert_eq!(resolve_test_key(Some("new-key".into()), "current"), "new-key");
        // Blank or whitespace-only typed key → reuse the stored one (write-only).
        assert_eq!(resolve_test_key(Some("".into()), "current"), "current");
        assert_eq!(resolve_test_key(Some("   ".into()), "current"), "current");
        // Omitted key → reuse the stored one.
        assert_eq!(resolve_test_key(None, "current"), "current");
    }

    #[test]
    fn empty_api_key_keeps_existing_key_and_updates_the_rest() {
        let tmp = TmpDir::new();
        let state = make_state(tmp.path(), "secret-key"); // key set → proxy Some
        assert!(state.llm_proxy().is_some());

        // No apiKey in the request → key preserved, base_url/model updated, proxy live.
        let cfg = apply_llm_settings(&state, "https://new/v1".into(), "m2".into(), None);
        assert_eq!(cfg.api_key, "secret-key");
        assert_eq!(cfg.base_url, "https://new/v1");
        assert_eq!(cfg.model, "m2");
        assert!(state.llm_proxy().is_some());
        assert_eq!(state.model(), "m2");
        // The masked response never reveals the kept key beyond its tail.
        assert_eq!(LlmSettingsResponse::of(&cfg).key_hint.as_deref(), Some("···-key"));
    }

    #[test]
    fn setting_a_key_enables_the_proxy() {
        let tmp = TmpDir::new();
        let state = make_state(tmp.path(), ""); // unset → proxy None
        assert!(state.llm_proxy().is_none());

        apply_llm_settings(&state, "https://x/v1".into(), "m".into(), Some("new-key".into()));
        assert!(state.llm_proxy().is_some());
    }

    #[test]
    fn changing_model_via_settings_repoints_the_cache() {
        let tmp = TmpDir::new();
        let state = make_state(tmp.path(), "key123"); // model "test-model"
        let fn_source = "def f():\n    return 1\n";
        let entry = CapsuleEntry { capsule: cap("f#1"), lines: vec![line("f#1", 2)] };
        state.project.read().unwrap().cache.put(fn_source, &entry).unwrap();
        // Hit under the original model.
        assert!(state.project.read().unwrap().cache.get(fn_source).is_some());

        apply_llm_settings(&state, "https://x/v1".into(), "different-model".into(), None);

        // Cache re-pointed under the new model → the old entry no longer matches.
        assert!(state.project.read().unwrap().cache.get(fn_source).is_none());
    }

    #[test]
    fn keeping_the_model_leaves_the_cache_intact() {
        let tmp = TmpDir::new();
        let state = make_state(tmp.path(), "key123");
        let fn_source = "def f():\n    return 1\n";
        let entry = CapsuleEntry { capsule: cap("f#1"), lines: vec![line("f#1", 2)] };
        state.project.read().unwrap().cache.put(fn_source, &entry).unwrap();

        // Same model (only base_url changes) → cache untouched, entry still hits.
        apply_llm_settings(&state, "https://x/v1".into(), "test-model".into(), None);
        assert!(state.project.read().unwrap().cache.get(fn_source).is_some());
    }
}
