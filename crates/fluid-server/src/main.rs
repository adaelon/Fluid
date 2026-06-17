//! Fluid local backend entry point.
//!
//! S1: `fluid <project>` starts an axum server exposing the L0 file tree and
//! single-file source reads. No graph, no LLM, no cache yet.

mod cache_store;
mod context_assembler;
mod graph_loader;
mod llm_proxy;
mod project_reader;
mod routes;
mod settings;
mod static_assets;
mod translate;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use cache_store::CacheStore;
use graph_loader::GraphLoader;
use project_reader::ProjectReader;
use routes::AppState;
use settings::LlmConfig;

/// Prompt template version — bump when the generation prompt changes (invalidates
/// cache, ADR-0003). The model version is now the real model id (S6); both feed the
/// cache key so a model/prompt change invalidates cached capsules.
const PROMPT_VERSION: &str = "p1";

#[derive(Parser)]
#[command(name = "fluid", about = "Fluid — read-only code understanding backend")]
struct Args {
    /// Path to the project directory to serve. Optional — omit it to start without a
    /// project and pick one from the UI (Open Folder).
    project: Option<PathBuf>,

    /// Port to bind on 127.0.0.1.
    #[arg(long, default_value_t = 7878)]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load `.env` (LLM config) from the CWD/ancestors into the process env before
    // any env read. Real environment variables take precedence (dotenvy default).
    // Run fluid from its own repo root, not from inside a served project that has
    // its own .env. Absent .env is fine (env vars alone still work).
    // Capture the resolved `.env` path so the settings panel (U5a) can write
    // changes back to the same file. When absent, default to `.env` in the CWD.
    let env_path = match dotenvy::dotenv() {
        Ok(path) => {
            println!("Loaded .env: {}", path.display());
            path
        }
        Err(e) if e.not_found() => PathBuf::from(".env"),
        Err(e) => {
            eprintln!("warning: .env present but unreadable: {e}");
            PathBuf::from(".env")
        }
    };

    let args = Args::parse();

    // The model id drives both the LLM call and the cache key, so they stay in
    // lock-step (a model switch invalidates the cache). All three values live in a
    // runtime-editable LlmConfig (U5a, ADR-0018); env-overridable, default glm-5.1
    // via the opencode zen gateway (S6 decision, see docs/代码链路.md).
    let llm_config = LlmConfig::from_env();
    if llm_config.key_set() {
        println!("LLM proxy ready: model {}", llm_config.model);
    } else {
        println!("LLM proxy disabled (OPENCODE_API_KEY unset) — configure it in the settings panel");
    }

    // Project is optional: with a path we serve it immediately; without one we start
    // empty and let the user open a folder from the UI (which calls /api/project/open).
    let state = match args.project {
        Some(path) => {
            let reader = ProjectReader::new(path)
                .map_err(|e| anyhow::anyhow!("cannot open project directory: {e}"))?;
            println!("Fluid serving project: {}", reader.root().display());
            let graph = GraphLoader::load(reader.root());
            match graph.graph() {
                Some(g) => println!(
                    "Knowledge graph loaded: {} nodes, {} edges",
                    g.nodes.len(),
                    g.edges.len()
                ),
                None => println!(
                    "No knowledge graph (.understand-anything/ absent) — running self-contained"
                ),
            }
            let cache = CacheStore::new(reader.root(), &llm_config.model, PROMPT_VERSION);
            AppState::new(reader, graph, cache, llm_config, env_path, PROMPT_VERSION)
        }
        None => {
            println!("No project specified — open a folder from the UI to begin");
            AppState::new_no_project(llm_config, env_path, PROMPT_VERSION)
        }
    };

    let app = routes::router(Arc::new(state));

    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let url = format!("http://{addr}");
    println!("\n  Fluid 已启动 → {url}\n  (后端 + 前端同端口;Ctrl+C 退出)\n");

    // Best-effort: open the default browser. Ignored on headless/unsupported hosts —
    // the URL is printed above regardless.
    let _ = open::that(&url);

    axum::serve(listener, app).await?;
    Ok(())
}
