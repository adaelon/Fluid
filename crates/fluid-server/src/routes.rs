//! HTTP routes for the L0 skeleton (S1).
//!
//! - `GET /api/project/tree`        -> { files: FileNode[] }
//! - `GET /api/file?path=<rel>`     -> { source: string }
//!
//! Both share an `Arc<ProjectReader>` as axum state.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::project_reader::{FileNode, ProjectReader, ReadErr};

type Shared = Arc<ProjectReader>;

pub fn router(reader: Shared) -> Router {
    Router::new()
        .route("/api/project/tree", get(tree))
        .route("/api/file", get(file))
        .with_state(reader)
}

#[derive(Serialize)]
struct TreeResponse {
    files: Vec<FileNode>,
}

async fn tree(State(reader): State<Shared>) -> Json<TreeResponse> {
    Json(TreeResponse {
        files: reader.list_files(),
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

async fn file(State(reader): State<Shared>, Query(q): Query<FileQuery>) -> impl IntoResponse {
    match reader.read_file(&q.path) {
        Ok(source) => (StatusCode::OK, Json(FileResponse { source })).into_response(),
        Err(ReadErr::NotFound) => (StatusCode::NOT_FOUND, "file not found").into_response(),
        Err(ReadErr::Forbidden) => {
            (StatusCode::FORBIDDEN, "path outside project root").into_response()
        }
    }
}
