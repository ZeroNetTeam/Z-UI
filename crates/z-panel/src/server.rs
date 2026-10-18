//! The panel's HTTP server: the API under `{base}/api`, the dashboard under
//! `{base}/`, and nothing anywhere else.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use tower_http::set_header::SetResponseHeaderLayer;
use z_node::Node;

use crate::auth::rate_limit::LoginLimiter;
use crate::auth::session::SessionKeys;
use crate::config::PanelConfig;

/// What every request handler can reach.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    config: PanelConfig,
    db: z_db::Database,
    node: Arc<dyn Node>,
    session_keys: SessionKeys,
    limiter: LoginLimiter,
    /// While no admin exists: the one-time token that may create the first,
    /// printed to the log at start.
    setup_token: Mutex<Option<String>>,
    /// 2FA secrets shown to an admin but not yet confirmed with a code.
    pending_totp: Mutex<HashMap<i64, (String, Instant)>>,
    /// The panel itself terminates TLS.
    tls: bool,
}

impl AppState {
    pub fn new(
        config: PanelConfig,
        db: z_db::Database,
        node: Arc<dyn Node>,
        session_keys: SessionKeys,
        tls: bool,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                db,
                node,
                session_keys,
                limiter: LoginLimiter::default(),
                setup_token: Mutex::new(None),
                pending_totp: Mutex::new(HashMap::new()),
                tls,
            }),
        }
    }

    pub fn config(&self) -> &PanelConfig {
        &self.inner.config
    }

    pub fn db(&self) -> &z_db::Database {
        &self.inner.db
    }

    pub fn node(&self) -> &Arc<dyn Node> {
        &self.inner.node
    }

    pub fn session_keys(&self) -> &SessionKeys {
        &self.inner.session_keys
    }

    pub fn limiter(&self) -> &LoginLimiter {
        &self.inner.limiter
    }

    pub fn tls(&self) -> bool {
        self.inner.tls
    }

    pub fn setup_token(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.inner
            .setup_token
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    pub fn pending_totp(&self) -> std::sync::MutexGuard<'_, HashMap<i64, (String, Instant)>> {
        self.inner
            .pending_totp
            .lock()
            .unwrap_or_else(|p| p.into_inner())
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
    app.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        crate::auth::same_origin,
    ))
    .with_state(state)
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
pub mod testing {
    //! A panel on a fresh database, driven in-process.

    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use clap::Parser;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    pub struct Panel {
        pub app: Router,
        dir: std::path::PathBuf,
    }

    impl Drop for Panel {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    pub struct Reply {
        pub status: StatusCode,
        pub body: String,
        pub headers: axum::http::HeaderMap,
    }

    impl Reply {
        pub fn json(&self) -> serde_json::Value {
            serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null)
        }

        /// The session cookie this reply set, as a `Cookie` header value.
        pub fn session(&self) -> Option<String> {
            let set = self.headers.get(header::SET_COOKIE)?.to_str().ok()?;
            let pair = set.split(';').next()?.trim().to_string();
            // `z_ui_session=` with nothing after it is a sign-out.
            (!pair.ends_with('=')).then_some(pair)
        }
    }

    pub async fn panel(base: &str) -> Panel {
        use std::hash::{BuildHasher, Hasher};
        let unique = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        let dir = std::env::temp_dir().join(format!("z-panel-test-{unique:x}"));
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
        let state = AppState::new(
            config,
            db,
            Arc::new(z_node::LocalNode::new()),
            SessionKeys::new(b"test-session-key-test-session-key"),
            false,
        );
        *state.setup_token() = Some("setup-token".into());
        Panel {
            app: router(state),
            dir,
        }
    }

    impl Panel {
        pub async fn send(
            &self,
            method: &str,
            path: &str,
            cookie: Option<&str>,
            body: Option<serde_json::Value>,
        ) -> Reply {
            let mut request = Request::builder().method(method).uri(path);
            if let Some(cookie) = cookie {
                request = request.header(header::COOKIE, cookie);
            }
            let request = match body {
                Some(body) => request
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string())),
                None => request.body(Body::empty()),
            }
            .unwrap();
            let response = self.app.clone().oneshot(request).await.unwrap();
            let status = response.status();
            let headers = response.headers().clone();
            let body = response.into_body().collect().await.unwrap().to_bytes();
            Reply {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
                headers,
            }
        }

        pub async fn get(&self, path: &str, cookie: Option<&str>) -> Reply {
            self.send("GET", path, cookie, None).await
        }

        /// Create the first admin and return their session cookie.
        pub async fn set_up(&self, prefix: &str) -> String {
            let reply = self
                .send(
                    "POST",
                    &format!("{prefix}/api/setup"),
                    None,
                    Some(serde_json::json!({
                        "token": "setup-token", "username": "owner", "password": "a-long-password",
                    })),
                )
                .await;
            assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
            reply.session().expect("setup signs in")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::panel;
    use super::*;

    #[tokio::test]
    async fn the_panel_lives_only_under_its_base_path() {
        let panel = panel("/s3cret").await;
        let session = panel.set_up("/s3cret").await;
        let reply = panel.get("/s3cret/api/health", Some(&session)).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(reply.json()["product"], "Z-UI");
        assert_eq!(reply.json()["core"]["running"], false);

        for path in ["/", "/api/health", "/index.html", "/s3cretx/"] {
            assert_eq!(
                panel.get(path, None).await.status,
                StatusCode::NOT_FOUND,
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn dashboard_routes_get_the_page_with_its_base() {
        let panel = panel("/s3cret").await;
        for path in ["/s3cret", "/s3cret/", "/s3cret/users", "/s3cret/users/42"] {
            let reply = panel.get(path, None).await;
            assert_eq!(reply.status, StatusCode::OK, "{path}");
            let content_type = reply.headers.get(header::CONTENT_TYPE).unwrap();
            assert!(
                content_type.to_str().unwrap().starts_with("text/html"),
                "{path}"
            );
            assert!(
                reply.body.contains("<base href=\"/s3cret/\">"),
                "{path}: {}",
                reply.body
            );
        }
        // A missing asset is a 404, not the page.
        assert_eq!(
            panel.get("/s3cret/assets/missing.js", None).await.status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            panel.get("/s3cret/api/nope", None).await.status,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn the_root_base_serves_everything() {
        let panel = panel("/").await;
        assert_eq!(
            panel.get("/api/health", None).await.status,
            StatusCode::UNAUTHORIZED
        );
        let reply = panel.get("/settings", None).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert!(reply.body.contains("<base href=\"/\">"), "{}", reply.body);
    }
}
