//! Fluid local backend entry point.
//!
//! S1: `fluid <project>` starts an axum server exposing the L0 file tree and
//! single-file source reads. No graph, no LLM, no cache yet.

mod cache_store;
mod context_assembler;
mod graph_loader;
mod llm_proxy;
// S-ORI-1 lands the validated protocol/cache boundary before S-ORI-2 adds the
// first route/LLM consumer. Keep the staged allowance local to that module.
#[allow(dead_code)]
mod orientation;
mod project_reader;
// S-QTHREAD-1 lands the project-scoped persistence boundary before S-QAPI-1
// adds the first route consumer. Keep the staged allowance local to the module.
#[allow(dead_code)]
mod query_history;
mod routes;
mod settings;
mod startup;
mod static_assets;
mod translate;
// S-WEB-1 intentionally lands the stable protocol layer before S-WEB-2 adds
// its first business consumer. Keep this allowance local to that staged module.
#[allow(dead_code)]
mod web_evidence;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use cache_store::CacheStore;
use graph_loader::GraphCatalog;
use project_reader::ProjectReader;
use query_history::QueryThreadStore;
use routes::AppState;
#[cfg(windows)]
use settings::windows_env_path;
use settings::LlmConfig;
use startup::{select_listener, StartupSelection};

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

    /// Port to bind on 127.0.0.1. When omitted, Fluid reuses a compatible server
    /// on 7878 or selects a free port if another program owns 7878.
    #[arg(long)]
    port: Option<u16>,
}

#[cfg(windows)]
fn load_llm_config() -> anyhow::Result<(PathBuf, LlmConfig)> {
    let local_app_data = std::env::var_os("LOCALAPPDATA");
    let env_path = windows_env_path(local_app_data.as_deref())?;
    let config = match LlmConfig::from_env_file(&env_path) {
        Ok(config) => config,
        Err(error) => {
            eprintln!(
                "warning: Fluid config is unreadable ({}): {error}; using explicit environment variables/defaults",
                env_path.display()
            );
            LlmConfig::from_env()
        }
    };
    println!("Fluid config: {}", env_path.display());
    Ok((env_path, config))
}

#[cfg(not(windows))]
fn load_llm_config() -> anyhow::Result<(PathBuf, LlmConfig)> {
    // Preserve the existing non-Windows behavior: search CWD/ancestors and load
    // the first `.env`, while explicit process variables retain precedence.
    let env_path = match dotenvy::dotenv() {
        Ok(path) => {
            println!("Loaded .env: {}", path.display());
            path
        }
        Err(error) if error.not_found() => PathBuf::from(".env"),
        Err(error) => {
            eprintln!("warning: .env present but unreadable: {error}");
            PathBuf::from(".env")
        }
    };
    Ok((env_path, LlmConfig::from_env()))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let startup = select_listener(args.port).await?;
    let (listener, url) = match startup {
        StartupSelection::Reuse { url } => {
            println!("Fluid 已在运行,复用现有实例 → {url}");
            if let Some(project) = args.project.as_deref() {
                startup::handoff_project(&url, project).await?;
                println!("已将项目交给现有实例: {}", project.display());
            }
            let _ = open::that(&url);
            return Ok(());
        }
        StartupSelection::Serve(server) => {
            if let Some(port) = server.fallback_from {
                println!("默认端口 {port} 已被其他程序占用,Fluid 改用 {}", server.url);
            }
            (server.listener, server.url)
        }
    };
    let (env_path, llm_config) = load_llm_config()?;

    // The model id drives both the LLM call and the cache key, so they stay in
    // lock-step (a model switch invalidates the cache). All three values live in a
    // runtime-editable LlmConfig (U5a, ADR-0018); env-overridable, default glm-5.1
    // via the opencode zen gateway (S6 decision, see docs/代码链路.md).
    if llm_config.key_set() {
        println!("LLM proxy ready: model {}", llm_config.model);
    } else {
        println!(
            "LLM proxy disabled (OPENCODE_API_KEY unset) — configure it in the settings panel"
        );
    }

    // Project is optional: with a path we serve it immediately; without one we start
    // empty and let the user open a folder from the UI (which calls /api/project/open).
    let state = match args.project {
        Some(path) => {
            let reader = ProjectReader::new(path)
                .map_err(|e| anyhow::anyhow!("cannot open project directory: {e}"))?;
            println!("Fluid serving project: {}", reader.root().display());
            let graphs = GraphCatalog::discover(reader.root());
            debug_assert_eq!(graphs.project_root(), reader.root());
            if graphs.is_empty() {
                println!(
                    "No knowledge graph (.ua/.understand-anything absent or invalid) — running self-contained"
                );
            } else {
                let nodes: usize = graphs
                    .snapshots()
                    .iter()
                    .map(|snapshot| snapshot.graph().nodes.len())
                    .sum();
                let edges: usize = graphs
                    .snapshots()
                    .iter()
                    .map(|snapshot| snapshot.graph().edges.len())
                    .sum();
                println!(
                    "Knowledge graph catalog loaded: {} scope(s), {} nodes, {} edges, freshness {}",
                    graphs.len(),
                    nodes,
                    edges,
                    graphs.freshness_hash()
                );
                for snapshot in graphs.snapshots() {
                    println!(
                        "  graph scope {} ({}) via {} at {} [{}]: {} nodes, {} edges",
                        snapshot.scope_path(),
                        snapshot.scope_root().display(),
                        snapshot.origin().label(),
                        snapshot.path().display(),
                        snapshot.content_hash(),
                        snapshot.graph().nodes.len(),
                        snapshot.graph().edges.len()
                    );
                }
            }
            let cache = CacheStore::new(reader.root(), &llm_config.model, PROMPT_VERSION);
            let query_threads = QueryThreadStore::new(reader.root())?;
            AppState::new(
                reader,
                graphs,
                cache,
                query_threads,
                llm_config,
                env_path,
                PROMPT_VERSION,
            )
        }
        None => {
            println!("No project specified — open a folder from the UI to begin");
            AppState::new_no_project(llm_config, env_path, PROMPT_VERSION)
        }
    };

    let app = routes::router(Arc::new(state));

    println!("\n  Fluid 已启动 → {url}\n  (后端 + 前端同端口;Ctrl+C 退出)\n");

    // Best-effort: open the default browser. Ignored on headless/unsupported hosts —
    // the URL is printed above regardless.
    let _ = open::that(&url);

    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_argument_launch_uses_automatic_port_selection() {
        let args = Args::try_parse_from(["fluid"]).unwrap();
        assert_eq!(args.project, None);
        assert_eq!(args.port, None);
    }

    #[test]
    fn explicit_port_remains_distinguishable_and_strict() {
        let args = Args::try_parse_from(["fluid", "project", "--port", "7879"]).unwrap();
        assert_eq!(args.project, Some(PathBuf::from("project")));
        assert_eq!(args.port, Some(7879));
    }
}
