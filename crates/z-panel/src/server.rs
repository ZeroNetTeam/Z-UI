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
    /// Wrong API keys, counted by address.
    key_limiter: LoginLimiter,
    /// While no admin exists: the one-time token that may create the first,
    /// printed to the log at start.
    setup_token: Mutex<Option<String>>,
    /// 2FA secrets shown to an admin but not yet confirmed with a code.
    pending_totp: Mutex<HashMap<i64, (String, Instant)>>,
    /// Present when the panel itself terminates TLS: the certificate it
    /// presents, which can be replaced while it runs ([`crate::tls`]).
    tls: Option<Arc<crate::tls::Resolver>>,
    /// Woken to stop serving and start again ([`AppState::restart_soon`]).
    restart: tokio::sync::Notify,
    /// The id of the stored certificate the panel serves HTTPS with.
    panel_certificate: Mutex<Option<i64>>,
    /// Held while the core's configuration changes, one change at a time.
    core_lock: tokio::sync::Mutex<()>,
    /// What the last look for a newer version found ([`crate::update`]).
    update: Mutex<crate::update::Status>,
    /// Held while a certificate is got or renewed: one at a time, so two do
    /// not fight over the challenge port or the authority's limits.
    cert_lock: tokio::sync::Mutex<()>,
    /// Users' addresses against their IP limits ([`crate::jobs`]).
    address_book: Mutex<crate::jobs::AddressBook>,
    /// The subscription options, read on every fetch ([`crate::subscription`]).
    subscription: std::sync::RwLock<crate::subscription::Settings>,
    /// The host's figures and the live log ([`crate::monitor`]).
    monitor: Arc<crate::monitor::Monitor>,
    /// What admins decided about notifications, the bot and backups.
    notify_settings: std::sync::RwLock<crate::notify::Settings>,
    /// Tells people what happens ([`crate::notify`]).
    notifier: Arc<crate::notify::Notifier>,
    /// Codes an admin sends the bot to tie a Telegram chat to themselves.
    link_codes: Mutex<HashMap<String, (i64, Instant)>>,
    /// Held while another panel's data is imported: one import at a time.
    import_lock: tokio::sync::Mutex<()>,
    /// What a takeover asks of the machine (the services, the ports).
    host: std::sync::RwLock<Arc<dyn crate::takeover::host::Host>>,
    /// Where the machine's files are, for a takeover (`/` but in tests).
    takeover_root: std::sync::RwLock<std::path::PathBuf>,
    /// What lightens the load of an overloaded server ([`crate::relief`]).
    relief: Mutex<crate::relief::Relief>,
    /// When a subscription fetch (by a user, from a device) was last written
    /// down, so a busy subscription is not a database write per request.
    fetch_noted: Mutex<HashMap<(i64, String), i64>>,
}

impl AppState {
    pub fn new(
        config: PanelConfig,
        db: z_db::Database,
        node: Arc<dyn Node>,
        session_keys: SessionKeys,
        tls: Option<Arc<crate::tls::Resolver>>,
        monitor: Arc<crate::monitor::Monitor>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                db,
                node,
                session_keys,
                limiter: LoginLimiter::default(),
                key_limiter: LoginLimiter::default(),
                setup_token: Mutex::new(None),
                pending_totp: Mutex::new(HashMap::new()),
                tls,
                restart: tokio::sync::Notify::new(),
                panel_certificate: Mutex::new(None),
                core_lock: tokio::sync::Mutex::new(()),
                update: Mutex::new(Default::default()),
                cert_lock: tokio::sync::Mutex::new(()),
                address_book: Mutex::new(Default::default()),
                subscription: std::sync::RwLock::new(Default::default()),
                monitor,
                notify_settings: std::sync::RwLock::new(Default::default()),
                notifier: crate::notify::Notifier::new(),
                link_codes: Mutex::new(HashMap::new()),
                import_lock: tokio::sync::Mutex::new(()),
                host: std::sync::RwLock::new(crate::takeover::host::from_env()),
                takeover_root: std::sync::RwLock::new(crate::takeover::root()),
                relief: Mutex::new(Default::default()),
                fetch_noted: Mutex::new(HashMap::new()),
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

    pub fn key_limiter(&self) -> &LoginLimiter {
        &self.inner.key_limiter
    }

    pub fn tls(&self) -> bool {
        self.inner.tls.is_some()
    }

    /// What the panel's certificate is read from, when it serves HTTPS.
    pub fn tls_resolver(&self) -> Option<&Arc<crate::tls::Resolver>> {
        self.inner.tls.as_ref()
    }

    pub fn panel_certificate(&self) -> Option<i64> {
        *self
            .inner
            .panel_certificate
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    pub fn set_panel_certificate(&self, id: Option<i64>) {
        *self
            .inner
            .panel_certificate
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = id;
    }

    /// Stop serving and start again in `delay`: long enough for the answer
    /// that asked for it to reach whoever asked.
    pub fn restart_soon(&self, delay: std::time::Duration) {
        let state = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            // `notify_one` keeps the wake-up if nothing waits yet.
            state.inner.restart.notify_one();
        });
    }

    /// Resolves once a restart was asked for.
    pub async fn restart_requested(&self) {
        self.inner.restart.notified().await;
    }

    pub fn host(&self) -> Arc<dyn crate::takeover::host::Host> {
        self.inner.host.read().expect("host lock").clone()
    }

    /// Whether a fetch of `user`'s subscription from `device` (empty for none)
    /// is to be written down now: the first one, then one a minute. What is
    /// kept is only when it was last fetched and by what, which a minute
    /// either way does not change; what must not wait (a device that is new
    /// to a user, which a device limit counts) has a key of its own, so it is
    /// always the first.
    pub fn fetch_due(&self, user: i64, device: &str, now: i64) -> bool {
        const EVERY: i64 = 60;
        let mut noted = self
            .inner
            .fetch_noted
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if noted.len() > 100_000 {
            noted.retain(|_, at| now - *at < EVERY);
        }
        match noted.get_mut(&(user, device.to_string())) {
            Some(at) if now - *at < EVERY => false,
            Some(at) => {
                *at = now;
                true
            }
            None => {
                noted.insert((user, device.to_string()), now);
                true
            }
        }
    }

    pub fn relief(&self) -> std::sync::MutexGuard<'_, crate::relief::Relief> {
        self.inner.relief.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn takeover_root(&self) -> std::path::PathBuf {
        self.inner.takeover_root.read().expect("root lock").clone()
    }

    #[cfg(test)]
    pub fn set_host(&self, host: Arc<dyn crate::takeover::host::Host>, root: std::path::PathBuf) {
        *self.inner.host.write().expect("host lock") = host;
        *self.inner.takeover_root.write().expect("root lock") = root;
    }

    pub fn import_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.inner.import_lock
    }

    pub fn core_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.inner.core_lock
    }

    pub fn update_status(&self) -> crate::update::Status {
        self.inner
            .update
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn set_update_status(&self, status: crate::update::Status) {
        *self.inner.update.lock().unwrap_or_else(|p| p.into_inner()) = status;
    }

    pub fn cert_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.inner.cert_lock
    }

    pub fn address_book(&self) -> std::sync::MutexGuard<'_, crate::jobs::AddressBook> {
        self.inner
            .address_book
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    pub fn monitor(&self) -> &Arc<crate::monitor::Monitor> {
        &self.inner.monitor
    }

    pub fn link_codes(&self) -> std::sync::MutexGuard<'_, HashMap<String, (i64, Instant)>> {
        self.inner
            .link_codes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    pub fn notifier(&self) -> &Arc<crate::notify::Notifier> {
        &self.inner.notifier
    }

    pub fn notify_settings(&self) -> crate::notify::Settings {
        self.inner
            .notify_settings
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn set_notify_settings(&self, settings: crate::notify::Settings) {
        *self
            .inner
            .notify_settings
            .write()
            .unwrap_or_else(|p| p.into_inner()) = settings;
    }

    /// Tell people something that happened, without waiting.
    pub fn notify(&self, event: crate::notify::Event) {
        self.inner.notifier.emit(event);
    }

    pub fn subscription(&self) -> crate::subscription::Settings {
        self.inner
            .subscription
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn set_subscription(&self, settings: crate::subscription::Settings) {
        *self
            .inner
            .subscription
            .write()
            .unwrap_or_else(|p| p.into_inner()) = settings;
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
        .nest("/api", crate::api::router(&state))
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
    // Subscriptions live outside the panel's path, and are answered first.
    .layer(axum::middleware::from_fn_with_state(
        state.clone(),
        crate::subscription::intercept,
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
    use http_body_util::BodyExt;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tower::ServiceExt;

    pub struct Panel {
        pub app: Router,
        pub state: AppState,
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

    pub fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    pub async fn echo_sink() -> u16 {
        let sink = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = sink.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = sink.accept().await {
                tokio::spawn(async move {
                    let (mut read, mut write) = socket.split();
                    let _ = tokio::io::copy(&mut read, &mut write).await;
                });
            }
        });
        port
    }

    /// A session through `socks` to the sink, carrying an echo, or None.
    pub async fn connect(socks: u16, sink: u16, payload: &[u8]) -> Option<TcpStream> {
        let attempt = async {
            let mut stream = TcpStream::connect(("127.0.0.1", socks)).await.ok()?;
            stream.write_all(&[5, 1, 0]).await.ok()?;
            let mut method = [0u8; 2];
            stream.read_exact(&mut method).await.ok()?;
            let mut request = vec![5, 1, 0, 1, 127, 0, 0, 1];
            request.extend_from_slice(&sink.to_be_bytes());
            stream.write_all(&request).await.ok()?;
            let mut reply = [0u8; 10];
            stream.read_exact(&mut reply).await.ok()?;
            stream.write_all(payload).await.ok()?;
            let mut back = vec![0u8; payload.len()];
            stream.read_exact(&mut back).await.ok()?;
            (back == payload).then_some(stream)
        };
        tokio::time::timeout(Duration::from_secs(3), attempt)
            .await
            .ok()
            .flatten()
    }

    pub async fn ended(stream: &mut TcpStream) -> bool {
        let _ = stream.write_all(b"still?").await;
        let mut buf = [0u8; 8];
        matches!(
            tokio::time::timeout(Duration::from_secs(3), stream.read(&mut buf)).await,
            Ok(Ok(0) | Err(_))
        )
    }

    /// A zray client whose SOCKS5 port goes to `port` as `uuid` over VLESS.
    pub async fn client(port: u16, uuid: &str) -> u16 {
        let socks = free_port();
        let config = serde_json::json!({
            "inbounds": [{"tag": "socks", "listen": "127.0.0.1", "port": socks, "protocol": "socks"}],
            "outbounds": [{"tag": "out", "protocol": "vless", "settings": {"vnext": [{
                "address": "127.0.0.1", "port": port, "users": [{"id": uuid, "encryption": "none"}],
            }]}}],
        });
        let (generation, _) =
            zero_config::compile_config(&config, zero_core::GenerationId(1)).unwrap();
        let server = std::sync::Arc::new(zero_runtime::Server::new(zero_runtime::ServerConfig {
            config: generation.config,
            generation: generation.id,
        }));
        let running = std::sync::Arc::clone(&server);
        tokio::spawn(async move { running.run().await });
        server.wait_until_listening().await;
        socks
    }

    pub async fn panel(base: &str) -> Panel {
        // The default path is not given as a flag: a flag would fix it.
        if base == "/" {
            panel_with(&[], None).await
        } else {
            panel_with(&["--base-path", base], None).await
        }
    }

    /// A panel started with these flags, serving HTTPS when it has a
    /// certificate resolver.
    pub async fn panel_with(args: &[&str], tls: Option<Arc<crate::tls::Resolver>>) -> Panel {
        use std::hash::{BuildHasher, Hasher};
        let unique = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        let dir = std::env::temp_dir().join(format!("z-panel-test-{unique:x}"));
        std::fs::create_dir_all(&dir).unwrap();
        let mut argv = vec!["z-ui", "--data-dir", dir.to_str().unwrap()];
        argv.extend_from_slice(args);
        let config = PanelConfig::from_args_iter(argv).resolved().unwrap();
        let db = z_db::Database::open(&config.database_url()).await.unwrap();
        let state = AppState::new(
            config,
            db,
            Arc::new(z_node::LocalNode::new()),
            SessionKeys::new(b"test-session-key-test-session-key"),
            tls,
            crate::monitor::Monitor::new(Default::default(), dir.clone()),
        );
        *state.setup_token() = Some("setup-token".into());
        state.notifier().start(state.clone());
        Panel {
            app: router(state.clone()),
            state,
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
            let headers: Vec<(&str, &str)> = cookie
                .map(|cookie| (header::COOKIE.as_str(), cookie))
                .into_iter()
                .collect();
            self.request(method, path, &headers, body).await
        }

        /// Any request, with any headers: a session cookie, an API key.
        pub async fn request(
            &self,
            method: &str,
            path: &str,
            headers: &[(&str, &str)],
            body: Option<serde_json::Value>,
        ) -> Reply {
            let mut request = Request::builder().method(method).uri(path);
            for (name, value) in headers {
                request = request.header(*name, *value);
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

        /// A request whose body is these raw bytes (a file upload).
        pub async fn upload(&self, path: &str, headers: &[(&str, &str)], bytes: Vec<u8>) -> Reply {
            self.upload_as("POST", path, headers, bytes).await
        }

        pub async fn upload_as(
            &self,
            method: &str,
            path: &str,
            headers: &[(&str, &str)],
            bytes: Vec<u8>,
        ) -> Reply {
            let mut request = Request::builder()
                .method(method)
                .uri(path)
                .header(header::CONTENT_TYPE, "application/octet-stream");
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            let response = self
                .app
                .clone()
                .oneshot(request.body(Body::from(bytes)).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let headers = response.headers().clone();
            let body = response.into_body().collect().await.unwrap().to_bytes();
            Reply {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
                headers,
            }
        }

        /// Sign in as `username` and return the session cookie.
        pub async fn sign_in(&self, username: &str, password: &str) -> String {
            let reply = self
                .send(
                    "POST",
                    "/api/auth/login",
                    None,
                    Some(serde_json::json!({"username": username, "password": password})),
                )
                .await;
            assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
            reply.session().expect("signing in sets a cookie")
        }

        /// A GET with `headers`, as a client app or browser sends it.
        pub async fn fetch(&self, path: &str, headers: &[(&str, &str)]) -> Reply {
            let mut request = Request::builder().method("GET").uri(path);
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            let response = self
                .app
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
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
