//! `z-ui` — the Z-UI panel.

mod accounts;
mod api;
mod assets;
mod audit;
mod auth;
mod backup;
mod certs;
mod cli;
mod config;
mod core_config;
mod core_settings;
mod error;
mod jobs;
mod monitor;
mod notify;
mod outbound;
mod panel_settings;
mod quota;
mod server;
mod subscription;
mod tls;
mod update;

use std::sync::Arc;

use anyhow::Context;
use z_node::Node;

use crate::auth::session::SessionKeys;
use crate::config::PanelConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // The same lines go to the terminal and to the dashboard's live log.
    // The filter can be swapped while the process runs (the Logs page does).
    let logs = monitor::LogBuffer::default();
    {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        let from_env = std::env::var("ZUI_LOG")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let filter = match &from_env {
            Some(text) => tracing_subscriber::EnvFilter::try_new(text).unwrap_or_else(|_| {
                monitor::logs::directive("info")
                    .parse()
                    .expect("a valid directive")
            }),
            None => monitor::logs::directive("info")
                .parse()
                .expect("a valid directive"),
        };
        let (filter, reload) = tracing_subscriber::reload::Layer::new(filter);
        logs.install_level(
            from_env.as_deref().unwrap_or("info"),
            from_env.is_some(),
            move |directive| {
                let filter =
                    tracing_subscriber::EnvFilter::try_new(directive).map_err(|e| e.to_string())?;
                reload.reload(filter).map_err(|e| e.to_string())
            },
        );
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer())
            .with(logs.layer())
            .init();
    }
    let mut config = PanelConfig::from_args();
    std::fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("creating {}", config.data_dir.display()))?;

    let db = z_db::Database::open(&config.database_url())
        .await
        .context("opening the database")?;

    let stored = panel_settings::load(&db).await;
    config.apply_stored(&stored);
    let config = config.resolved()?;
    if let Some(command) = &config.command {
        return cli::run(&config, &db, &stored, command).await;
    }

    let session_keys = SessionKeys::new(&session_secret(&db).await?);
    // The certificate files given as flags win over the store's.
    let resolver = match (&config.cert, &config.key) {
        (Some(cert), Some(key)) => {
            let resolver = tls::Resolver::from_files(cert, key)?;
            tls::watch_files(resolver.clone(), cert.clone(), key.clone());
            Some(resolver)
        }
        _ => match stored.certificate_id {
            Some(id) => Some(tls::Resolver::new(
                tls::stored_certificate(&db, id)
                    .await
                    .context("the panel's certificate; `z-ui setting set --certificate none` goes back to HTTP")?,
            )),
            None => None,
        },
    };
    let tls = resolver.clone().map(tls::acceptor).transpose()?;
    let node = Arc::new(z_node::LocalNode::new());

    let state = server::AppState::new(
        config.clone(),
        db.clone(),
        node.clone(),
        session_keys,
        resolver,
        monitor::Monitor::new(logs, config.data_dir.clone()),
    );
    if tls.is_some() && config.cert.is_none() {
        state.set_panel_certificate(stored.certificate_id);
    }
    state.monitor().spawn();
    state.set_subscription(subscription::load(&db).await);
    state.set_notify_settings(notify::load(&db).await);
    state.notifier().start(state.clone());
    // The level the admin chose last time, unless ZUI_LOG says otherwise.
    if let Ok(Some(level)) = db.setting(api::LOG_LEVEL_SETTING).await {
        let _ = state.monitor().logs.set_level(&level);
    }
    let scheme = if tls.is_some() { "https" } else { "http" };
    let address = format!("{scheme}://{}{}", config.listen, config.under_base("/"));
    if db.admin_count().await? == 0 {
        first_admin_from_environment(&db).await;
    }
    if db.admin_count().await? == 0 {
        // Whoever reaches a fresh panel first would otherwise own it: only
        // someone who can read this log can create the first admin.
        let token = hex::encode(rand::random::<[u8; 16]>());
        *state.setup_token() = Some(token.clone());
        tracing::warn!("no admin yet; create one at {address}setup?token={token}");
    }
    match core_config::sync(&state).await {
        Ok(applied) => tracing::info!(?applied, "core configured from the database"),
        Err(error) => tracing::error!(error = %error.message, "the core did not start"),
    }
    jobs::spawn(state.clone());
    notify::watch::spawn(state.clone());
    notify::bot::spawn(state.clone());
    backup::spawn(state.clone());
    certs::spawn(state.clone());
    update::spawn(state.clone());
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("listening on {}", config.listen))?;
    tracing::info!("{} {} on {address}", z_common::PRODUCT, z_common::VERSION);
    if let Some(sub_listen) = config.sub_listen {
        // Subscriptions on a port of their own, for a domain apart from the
        // panel's: nothing else is answered there.
        let listener = tokio::net::TcpListener::bind(sub_listen)
            .await
            .with_context(|| format!("listening on {sub_listen}"))?;
        tracing::info!("subscriptions on {scheme}://{sub_listen}");
        let app = subscription::router(state.clone());
        let tls = tls.clone();
        tokio::spawn(async move {
            match tls {
                Some(acceptor) => tls::serve(listener, acceptor, app, std::future::pending()).await,
                None => {
                    let _ = axum::serve(
                        listener,
                        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
                    )
                    .await;
                }
            }
        });
    }
    let restarting = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let shutdown = {
        let (state, restarting) = (state.clone(), restarting.clone());
        async move {
            tokio::select! {
                () = shutdown_signal() => {}
                () = state.restart_requested() => {
                    restarting.store(true, std::sync::atomic::Ordering::SeqCst);
                    tracing::info!("restarting");
                }
            }
        }
    };
    let app = server::router(state);
    match tls {
        Some(acceptor) => tls::serve(listener, acceptor, app, shutdown).await,
        None => axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown)
        .await
        .context("serving the panel")?,
    }
    node.stop().await;
    if restarting.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(start_again());
    }
    Ok(())
}

/// Replace this process with a new one running the same program with the
/// same arguments, so a restart needs no service manager (and keeps the
/// process id, which one expects of a container's first process). Only
/// returns if that failed.
fn start_again() -> anyhow::Error {
    let exe = update::own_executable();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = std::process::Command::new(&exe)
            .args(std::env::args_os().skip(1))
            .exec();
        anyhow::anyhow!("starting {} again: {error}", exe.display())
    }
    #[cfg(not(unix))]
    {
        anyhow::anyhow!("stopped for a restart; start {} again", exe.display())
    }
}

/// A fresh panel started with `ZUI_ADMIN_USER` and `ZUI_ADMIN_PASSWORD`
/// (a container, which has no one at a terminal) gets that admin, a sudo
/// one, at once. A name or password that will not do is said and ignored: the
/// setup link in the log is still there.
async fn first_admin_from_environment(db: &z_db::Database) {
    let (Some(username), Some(password)) = (
        std::env::var("ZUI_ADMIN_USER")
            .ok()
            .filter(|v| !v.trim().is_empty()),
        std::env::var("ZUI_ADMIN_PASSWORD")
            .ok()
            .filter(|v| !v.is_empty()),
    ) else {
        return;
    };
    let made = async {
        auth::password::check_username(&username).map_err(|e| e.message)?;
        auth::password::check_strength(&password).map_err(|e| e.message)?;
        let hash = auth::password::hash(&password)
            .await
            .map_err(|e| e.message)?;
        db.create_admin(&username, &hash, true)
            .await
            .map_err(|e| e.to_string())
    }
    .await;
    match made {
        Ok(_) => {
            tracing::info!(%username, "the first admin was made from ZUI_ADMIN_USER and ZUI_ADMIN_PASSWORD")
        }
        Err(error) => {
            tracing::warn!(%error, "ZUI_ADMIN_USER and ZUI_ADMIN_PASSWORD could not make an admin")
        }
    }
}

/// The key sessions are signed with, made on first start and kept.
async fn session_secret(db: &z_db::Database) -> anyhow::Result<Vec<u8>> {
    const KEY: &str = "auth.session_secret";
    if let Some(secret) = db.setting(KEY).await? {
        if let Ok(bytes) = hex::decode(&secret) {
            if bytes.len() >= 32 {
                return Ok(bytes);
            }
        }
    }
    let secret = rand::random::<[u8; 32]>();
    db.set_setting(KEY, &hex::encode(secret)).await?;
    Ok(secret.to_vec())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutting down");
}
