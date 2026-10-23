//! `z-ui` — the Z-UI panel.

mod accounts;
mod api;
mod assets;
mod audit;
mod auth;
mod backup;
mod config;
mod core_config;
mod core_settings;
mod error;
mod jobs;
mod monitor;
mod notify;
mod quota;
mod server;
mod subscription;
mod tls;

use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use z_node::Node;

use crate::auth::session::SessionKeys;
use crate::config::{AdminCommand, Command, PanelConfig};

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
    let config = PanelConfig::parse().resolved()?;
    std::fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("creating {}", config.data_dir.display()))?;

    let db = z_db::Database::open(&config.database_url())
        .await
        .context("opening the database")?;

    if let Some(Command::Admin(command)) = &config.command {
        return admin_command(&db, command).await;
    }

    let session_keys = SessionKeys::new(&session_secret(&db).await?);
    let tls = match (&config.cert, &config.key) {
        (Some(cert), Some(key)) => Some(tls::acceptor(cert, key)?),
        _ => None,
    };
    let node = Arc::new(z_node::LocalNode::new());

    let state = server::AppState::new(
        config.clone(),
        db.clone(),
        node.clone(),
        session_keys,
        tls.is_some(),
        monitor::Monitor::new(logs, config.data_dir.clone()),
    );
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
    let app = server::router(state);
    match tls {
        Some(acceptor) => tls::serve(listener, acceptor, app, shutdown_signal()).await,
        None => axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving the panel")?,
    }
    node.stop().await;
    Ok(())
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

async fn admin_command(db: &z_db::Database, command: &AdminCommand) -> anyhow::Result<()> {
    match command {
        AdminCommand::ResetPassword {
            username,
            password,
            disable_2fa,
        } => {
            let admin = db
                .admin_by_username(username)
                .await?
                .with_context(|| format!("there is no admin {username:?}"))?;
            let (password, generated) = match password {
                Some(password) => (password.clone(), false),
                None => (hex::encode(rand::random::<[u8; 9]>()), true),
            };
            auth::password::check_strength(&password).map_err(|e| anyhow::anyhow!(e.message))?;
            let hash = auth::password::hash(&password)
                .await
                .map_err(|e| anyhow::anyhow!(e.message))?;
            db.set_admin_password(admin.id, &hash).await?;
            if *disable_2fa {
                db.set_admin_totp(admin.id, None).await?;
            }
            println!("the password of {username:?} was reset; their sessions have ended");
            if generated {
                println!("new password: {password}");
            }
            if *disable_2fa {
                println!("two-factor sign-in is off");
            }
            Ok(())
        }
    }
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
