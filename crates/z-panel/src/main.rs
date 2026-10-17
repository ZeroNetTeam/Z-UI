//! `z-ui` — the Z-UI panel.

mod api;
mod assets;
mod config;
mod server;

use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use z_node::Node;

use crate::config::PanelConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ZUI_LOG")
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    let config = PanelConfig::parse().resolved()?;
    std::fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("creating {}", config.data_dir.display()))?;

    let db = z_db::Database::open(&config.database_url())
        .await
        .context("opening the database")?;
    let node = Arc::new(z_node::LocalNode::new());

    // Until the panel builds the core's configuration from its database
    // (phase 3), a hand-written one can be dropped next to it.
    let core_config = config.data_dir.join("core.json");
    if core_config.exists() {
        let text = std::fs::read_to_string(&core_config)
            .with_context(|| format!("reading {}", core_config.display()))?;
        let value: serde_json::Value =
            serde_json::from_str(&text).context("core.json is not JSON")?;
        match node.apply(value).await {
            Ok(applied) => tracing::info!(?applied, "core started"),
            Err(error) => tracing::error!(%error, "the core did not start"),
        }
    }

    let state = server::AppState::new(config.clone(), db, node.clone());
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("listening on {}", config.listen))?;
    tracing::info!(
        "{} {} on http://{}{}",
        z_common::PRODUCT,
        z_common::VERSION,
        config.listen,
        config.base_path
    );
    axum::serve(listener, server::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving the panel")?;
    node.stop().await;
    Ok(())
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
