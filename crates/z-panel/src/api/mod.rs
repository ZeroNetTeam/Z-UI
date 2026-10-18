//! The panel's JSON API. Everything the dashboard does goes through here,
//! so scripts and bots can do the same.

mod auth;
mod certificates;
mod hosts;
mod inbounds;
mod keys;
mod tools;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post, put};
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
        .route("/inbounds", get(inbounds::list).post(inbounds::create))
        .route(
            "/inbounds/{id}",
            get(inbounds::get)
                .put(inbounds::update)
                .delete(inbounds::delete),
        )
        .route("/inbounds/{id}/enabled", put(inbounds::set_enabled))
        .route("/hosts", get(hosts::list).post(hosts::create))
        .route("/hosts/{id}", put(hosts::update).delete(hosts::delete))
        .route(
            "/certificates",
            get(certificates::list).post(certificates::create),
        )
        .route(
            "/certificates/{id}",
            axum::routing::delete(certificates::delete),
        )
        .route("/keys/x25519", get(keys::x25519))
        .route("/keys/wireguard", get(keys::wireguard))
        .route("/keys/mldsa65", get(keys::mldsa65))
        .route("/keys/uuid", get(keys::uuid))
        .route("/keys/short-id", get(keys::short_id))
        .route("/keys/password", get(keys::password))
        .route("/keys/vless-encryption", get(keys::vless_encryption))
        .route("/tools/reality-check", get(tools::reality_check))
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
