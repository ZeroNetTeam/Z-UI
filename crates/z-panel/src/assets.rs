//! The dashboard, built by Vite and embedded into the binary.
//!
//! Vite builds with relative asset URLs, and every page gets a `<base>`
//! naming the panel's base path, so one build works under any secret path
//! and at any depth of client-side route.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

use crate::server::AppState;

#[derive(rust_embed::Embed)]
#[folder = "../../dashboard/dist"]
struct Dashboard;

pub async fn serve(State(state): State<AppState>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if !path.is_empty() && path != "index.html" {
        if let Some(file) = Dashboard::get(path) {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            // Vite names built assets by their content hash.
            let cache = if path.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            return (
                [
                    (header::CONTENT_TYPE, mime.as_ref().to_string()),
                    (header::CACHE_CONTROL, cache.to_string()),
                ],
                Body::from(file.data.into_owned()),
            )
                .into_response();
        }
        // A file that should exist and does not is not a page.
        if path.starts_with("assets/") || path.contains('.') {
            return StatusCode::NOT_FOUND.into_response();
        }
    }
    index(&state.config().under_base("/"))
}

/// The dashboard's page, told where it lives.
fn index(base: &str) -> Response {
    let Some(file) = Dashboard::get("index.html") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let page = String::from_utf8_lossy(&file.data);
    let base_tag = format!("<base href=\"{}\">", escape(base));
    let page = if page.contains("<head>") {
        page.replacen("<head>", &format!("<head>{base_tag}"), 1)
    } else {
        format!("{base_tag}{page}")
    };
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        page,
    )
        .into_response()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
