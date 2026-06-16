//! Serves the embedded frontend SPA (web/dist) so a single `fluid` binary is the
//! whole app — no Vite/Node at runtime (packaging). Release builds bake the assets
//! into the binary; debug builds read them from disk. This is wired as the router's
//! `fallback`, so it only runs for paths no real route matched: a real asset is
//! returned as-is, any other (non-`/api`) path falls back to `index.html` so the
//! Vue client boots and routes it (SPA fallback).

use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct Assets;

/// Router fallback. Order: real asset → `index.html` (SPA) → 404. An `/api/*` path
/// only reaches here if no API route matched it, so it must 404 (never the SPA),
/// otherwise a typo'd endpoint would silently return HTML.
pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    if let Some(resp) = serve(path) {
        return resp;
    }
    serve("index.html").unwrap_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            "frontend not built — run `npm --prefix web run build`",
        )
            .into_response()
    })
}

/// Build a response for an embedded asset, or `None` if there's no such file.
fn serve(path: &str) -> Option<Response> {
    let file = Assets::get(path)?;
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    Some(
        Response::builder()
            .header(header::CONTENT_TYPE, mime.as_ref())
            .body(Body::from(file.data.into_owned()))
            .expect("valid static asset response"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content_type(r: &Response) -> &str {
        r.headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }

    #[test]
    fn index_html_is_embedded() {
        // build.rs guarantees this exists (real frontend in CI, placeholder otherwise).
        assert!(Assets::get("index.html").is_some());
    }

    #[tokio::test]
    async fn serves_index_html_at_root() {
        let r = static_handler(Uri::from_static("/")).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert!(content_type(&r).starts_with("text/html"));
    }

    #[tokio::test]
    async fn unknown_path_falls_back_to_index_for_spa() {
        // A client-side route with no matching asset → index.html (SPA boot).
        let r = static_handler(Uri::from_static("/some/spa/route")).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert!(content_type(&r).starts_with("text/html"));
    }

    #[tokio::test]
    async fn unmatched_api_path_is_404_not_the_spa() {
        // An /api typo that no real route caught must 404, never silently return HTML.
        let r = static_handler(Uri::from_static("/api/does-not-exist")).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        assert!(!content_type(&r).starts_with("text/html"));
    }
}
