//! The panel's JSON API. Everything the dashboard does goes through here,
//! so scripts and bots can do the same.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use z_common::node::NodeStatus;

use crate::server::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .fallback(|| async { StatusCode::NOT_FOUND })
}

#[derive(Serialize)]
struct Health {
    product: &'static str,
    version: &'static str,
    core: NodeStatus,
}

/// Whether the panel and its core are up.
async fn health(State(state): State<AppState>) -> Json<Health> {
    Json(Health {
        product: z_common::PRODUCT,
        version: z_common::VERSION,
        core: state.node().status().await,
    })
}
