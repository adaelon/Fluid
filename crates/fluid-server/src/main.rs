//! Fluid local backend entry point.
//!
//! S1: `fluid <project>` starts an axum server exposing the L0 file tree and
//! single-file source reads. No graph, no LLM, no cache yet.

mod graph_loader;
mod project_reader;
mod routes;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use graph_loader::GraphLoader;
use project_reader::ProjectReader;
use routes::AppState;

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

    let app = routes::router(Arc::new(AppState { reader, graph }));

    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("Listening on http://{addr}");

    axum::serve(listener, app).await?;
    Ok(())
}
