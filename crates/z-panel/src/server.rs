//! The panel's HTTP server: the API under `{base}/api`, the dashboard under
//! `{base}/`, and nothing anywhere else.

use std::sync::Arc;

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use tower_http::set_header::SetResponseHeaderLayer;
use z_node::Node;

use crate::config::PanelConfig;

/// What every request handler can reach.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    config: PanelConfig,
    #[allow(dead_code)] // The users, inbounds and settings handlers (phase 2) read it.
    db: z_db::Database,
    node: Arc<dyn Node>,
}

impl AppState {
    pub fn new(config: PanelConfig, db: z_db::Database, node: Arc<dyn Node>) -> Self {
        Self {
            inner: Arc::new(Inner { config, db, node }),
        }
    }

    pub fn config(&self) -> &PanelConfig {
        &self.inner.config
    }

    pub fn node(&self) -> &Arc<dyn Node> {
        &self.inner.node
    }
}

pub fn router(state: AppState) -> Router {
    let base = state.config().base_path.clone();
    let panel = Router::new()
        .nest("/api", crate::api::router())
        .fallback(crate::assets::serve);
    let app = if base == "/" {
        panel
    } else {
        Router::new()
            .nest(&base, panel)
            // `nest` takes `/base` and `/base/…` but not `/base/` itself,
            // which is where a browser lands after following a link.
            .route(&format!("{base}/"), get(crate::assets::serve))
            .fallback(not_found)
    };
    app.with_state(state)
        // The panel's address is its secret path: never send it on.
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
}

/// What a request outside the base path gets: nothing that says a panel is
/// here.
async fn not_found() -> impl IntoResponse {
    StatusCode::NOT_FOUND
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use clap::Parser;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn app(base: &str) -> Router {
        let dir = std::env::temp_dir().join(format!(
            "z-panel-test-{}-{}",
            std::process::id(),
            base.replace('/', "_")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config = PanelConfig::parse_from([
            "z-ui",
            "--base-path",
            base,
            "--data-dir",
            dir.to_str().unwrap(),
        ])
        .resolved()
        .unwrap();
        let db = z_db::Database::open(&config.database_url()).await.unwrap();
        router(AppState::new(
            config,
            db,
            Arc::new(z_node::LocalNode::new()),
        ))
    }

    async fn get(app: &Router, path: &str) -> (StatusCode, String, Option<String>) {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string());
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&body).into_owned(), content_type)
    }

    #[tokio::test]
    async fn the_panel_lives_only_under_its_base_path() {
        let app = app("/s3cret").await;
        let (status, body, _) = get(&app, "/s3cret/api/health").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let health: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(health["product"], "Z-UI");
        assert_eq!(health["core"]["running"], false);

        for path in ["/", "/api/health", "/index.html", "/s3cretx/"] {
            assert_eq!(get(&app, path).await.0, StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[tokio::test]
    async fn dashboard_routes_get_the_page_with_its_base() {
        let app = app("/s3cret").await;
        for path in ["/s3cret", "/s3cret/", "/s3cret/users", "/s3cret/users/42"] {
            let (status, body, content_type) = get(&app, path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert!(content_type.unwrap().starts_with("text/html"), "{path}");
            assert!(body.contains("<base href=\"/s3cret/\">"), "{path}: {body}");
        }
        // A missing asset is a 404, not the page.
        assert_eq!(
            get(&app, "/s3cret/assets/missing.js").await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(get(&app, "/s3cret/api/nope").await.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_root_base_serves_everything() {
        let app = app("/").await;
        assert_eq!(get(&app, "/api/health").await.0, StatusCode::OK);
        let (status, body, _) = get(&app, "/settings").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("<base href=\"/\">"), "{body}");
    }
}
