//! Fluid local backend entry point.
//!
//! S1: `fluid <project>` starts an axum server exposing the L0 file tree and
//! single-file source reads. No graph, no LLM, no cache yet.

// cache_store's API is exercised only by its own tests until S6 wires it into
// `/api/generate`; suppress dead-code noise meanwhile (cf. S4 parser).
#[allow(dead_code)]
mod cache_store;
mod graph_loader;
mod project_reader;
mod routes;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use cache_store::CacheStore;
use graph_loader::GraphLoader;
use project_reader::ProjectReader;
use routes::AppState;

/// Prompt template version — bump when the generation prompt changes (invalidates
/// cache, ADR-0003). The model version is PENDING until LLMProxy lands (S6); a
/// placeholder keeps the cache keyed without pre-committing model selection.
const PROMPT_VERSION: &str = "p1";
const MODEL_VERSION_PLACEHOLDER: &str = "s5-unset";

#[derive(Parser)]
#[command(name = "fluid", about = "Fluid — read-only code understanding backend")]
struct Args {
    /// Path to the project directory to serve.
    project: PathBuf,

    /// Port to bind on 127.0.0.1.
    #[arg(long, default_value_t = 7878)]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let reader = ProjectReader::new(args.project)
        .map_err(|e| anyhow::anyhow!("cannot open project directory: {e}"))?;
    println!("Fluid serving project: {}", reader.root().display());

    let graph = GraphLoader::load(reader.root());
    match graph.graph() {
        Some(g) => println!(
            "Knowledge graph loaded: {} nodes, {} edges",
            g.nodes.len(),
            g.edges.len()
        ),
        None => println!("No knowledge graph (.understand-anything/ absent) — running self-contained"),
    }

    let cache = CacheStore::new(reader.root(), MODEL_VERSION_PLACEHOLDER, PROMPT_VERSION);

    let app = routes::router(Arc::new(AppState {
        reader,
        graph,
        cache,
    }));

    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("Listening on http://{addr}");

    axum::serve(listener, app).await?;
    Ok(())
}
