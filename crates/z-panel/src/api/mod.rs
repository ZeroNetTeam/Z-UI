//! The panel's JSON API. Everything the dashboard does goes through here,
//! so scripts and bots can do the same.

mod auth;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use z_common::node::NodeStatus;

use crate::auth::CurrentAdmin;
use crate::server::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/auth/state", get(auth::state))
        .route("/setup", post(auth::setup))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/auth/password", post(auth::change_password))
        .route("/auth/2fa/begin", post(auth::totp_begin))
        .route("/auth/2fa/enable", post(auth::totp_enable))
        .route("/auth/2fa/disable", post(auth::totp_disable))
        .fallback(|| async { StatusCode::NOT_FOUND })
}

#[derive(Serialize)]
struct Health {
    product: &'static str,
    version: &'static str,
    core: NodeStatus,
}

/// Whether the panel and its core are up.
async fn health(State(state): State<AppState>, _: CurrentAdmin) -> Json<Health> {
    Json(Health {
        product: z_common::PRODUCT,
        version: z_common::VERSION,
        core: state.node().status().await,
    })
}
