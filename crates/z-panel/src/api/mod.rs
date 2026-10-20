//! The panel's JSON API. Everything the dashboard does goes through here,
//! so scripts and bots can do the same.

#[cfg(test)]
mod access_tests;
mod admins;
mod api_keys;
mod audit;
mod auth;
mod certificates;
mod core;
mod groups;
mod hosts;
mod inbounds;
mod keys;
mod monitor;
mod roles;
mod subscription;
mod tools;
mod users;

use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Serialize;
use z_common::node::NodeStatus;

use crate::auth::access::{guard, Need};
use crate::auth::permissions::{Level, Section};
use crate::auth::Access;
use crate::server::AppState;

/// The setting the admin's choice of log level is kept under.
pub const LOG_LEVEL_SETTING: &str = "log.level";

/// `routes`, open to whoever `need` lets in.
fn guarded(state: &AppState, need: Need, routes: Router<AppState>) -> Router<AppState> {
    routes.route_layer(from_fn_with_state((state.clone(), need), guard))
}

pub fn router(state: &AppState) -> Router<AppState> {
    // Signing in: open to anyone, answered by the handlers themselves.
    let open = Router::new()
        .route("/auth/state", get(auth::state))
        .route("/setup", post(auth::setup))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout));

    // Anyone signed in: their own account, and tools that touch nothing.
    let signed = Router::new()
        .route("/health", get(health))
        .route("/auth/me", get(auth::me))
        .route("/keys/x25519", get(keys::x25519))
        .route("/keys/wireguard", get(keys::wireguard))
        .route("/keys/mldsa65", get(keys::mldsa65))
        .route("/keys/uuid", get(keys::uuid))
        .route("/keys/short-id", get(keys::short_id))
        .route("/keys/password", get(keys::password))
        .route("/keys/vless-encryption", get(keys::vless_encryption))
        .route("/tools/qr", get(tools::qr));

    // What is done to oneself needs a sign-in, not a key.
    let own = Router::new()
        .route("/auth/password", post(auth::change_password))
        .route("/auth/2fa/begin", post(auth::totp_begin))
        .route("/auth/2fa/enable", post(auth::totp_enable))
        .route("/auth/2fa/disable", post(auth::totp_disable));

    let users = Router::new()
        .route("/users", get(users::list).post(users::create))
        .route("/users/counts", get(users::counts))
        .route("/users/bulk", post(users::bulk))
        .route("/users/bulk-create", post(users::bulk_create))
        .route(
            "/users/{id}",
            get(users::get).put(users::update).delete(users::delete),
        )
        .route("/users/{id}/reset", post(users::reset))
        .route("/users/{id}/revoke", post(users::revoke))
        .route("/users/{id}/end-sessions", post(users::end_sessions))
        .route("/users/{id}/usage", get(users::usage))
        .route("/users/{id}/online", get(users::online))
        .route(
            "/users/{id}/subscription",
            get(subscription::user_subscription),
        )
        .route(
            "/users/{id}/devices",
            get(subscription::devices).delete(subscription::delete_devices),
        )
        .route(
            "/users/{id}/devices/{device}",
            delete(subscription::delete_device),
        )
        // What users moved is the users' business, not the server's.
        .route("/monitor/traffic", get(monitor::traffic));

    let groups = Router::new()
        .route("/groups", get(groups::list).post(groups::create))
        .route("/groups/{id}", put(groups::update).delete(groups::delete));
    let templates = Router::new()
        .route(
            "/templates",
            get(groups::templates).post(groups::create_template),
        )
        .route(
            "/templates/{id}",
            put(groups::update_template).delete(groups::delete_template),
        );
    let inbounds = Router::new()
        .route("/inbounds", get(inbounds::list).post(inbounds::create))
        .route(
            "/inbounds/{id}",
            get(inbounds::get)
                .put(inbounds::update)
                .delete(inbounds::delete),
        )
        .route("/inbounds/{id}/enabled", put(inbounds::set_enabled));
    // It dials a stranger's address on the panel's behalf.
    let probes = Router::new().route("/tools/reality-check", get(tools::reality_check));
    let hosts = Router::new()
        .route("/hosts", get(hosts::list).post(hosts::create))
        .route("/hosts/{id}", put(hosts::update).delete(hosts::delete));
    let certificates = Router::new()
        .route(
            "/certificates",
            get(certificates::list).post(certificates::create),
        )
        .route("/certificates/{id}", delete(certificates::delete));
    let core = Router::new()
        .route("/core", get(core::get).put(core::save))
        .route("/core/warp", post(core::create_warp))
        .route("/core/geodata", get(core::geodata))
        .route("/core/geodata/refresh", post(core::refresh_geodata));
    // Questions about a configuration, which change nothing.
    let core_questions = Router::new()
        .route("/core/validate", post(core::validate))
        .route("/core/presets/iran", post(core::iran_preset))
        .route("/core/check-link", post(core::check_link));
    let monitor = Router::new().route("/monitor/system", get(monitor::system));
    let logs = Router::new()
        .route("/logs", get(monitor::logs))
        .route("/logs/level", get(monitor::level).put(monitor::set_level))
        .route("/logs/stream", get(monitor::stream));
    let settings = Router::new().route(
        "/settings/subscription",
        get(subscription::settings).put(subscription::save_settings),
    );
    let audit = Router::new().route("/audit", get(audit::list));
    let api_keys = Router::new()
        .route("/api-keys", get(api_keys::list).post(api_keys::create))
        .route("/api-keys/{id}", delete(api_keys::delete));
    let admins = Router::new()
        .route("/admins", get(admins::list).post(admins::create))
        .route("/admins/{id}", put(admins::update).delete(admins::delete))
        .route("/admins/{id}/2fa", delete(admins::reset_totp))
        .route("/admins/{id}/sign-out", post(admins::sign_out))
        .route("/roles", get(roles::list).post(roles::create))
        .route("/roles/{id}", put(roles::update).delete(roles::delete));

    open.merge(guarded(state, Need::Signed, signed))
        .merge(guarded(state, Need::Session, own))
        .merge(guarded(state, Need::Section(Section::Users), users))
        .merge(guarded(state, Need::Section(Section::Groups), groups))
        .merge(guarded(state, Need::Section(Section::Templates), templates))
        .merge(guarded(state, Need::Section(Section::Inbounds), inbounds))
        .merge(guarded(
            state,
            Need::Exactly(Section::Inbounds, Level::Write),
            probes,
        ))
        .merge(guarded(state, Need::Section(Section::Hosts), hosts))
        .merge(guarded(
            state,
            Need::Section(Section::Certificates),
            certificates,
        ))
        .merge(guarded(state, Need::Section(Section::Core), core))
        .merge(guarded(
            state,
            Need::Exactly(Section::Core, Level::Read),
            core_questions,
        ))
        .merge(guarded(state, Need::Section(Section::Monitor), monitor))
        .merge(guarded(state, Need::Section(Section::Logs), logs))
        .merge(guarded(state, Need::Section(Section::Settings), settings))
        .merge(guarded(state, Need::Section(Section::Audit), audit))
        .merge(guarded(state, Need::SessionSection(Section::Api), api_keys))
        .merge(guarded(state, Need::Sudo, admins))
        .fallback(|| async { StatusCode::NOT_FOUND })
        .layer(from_fn_with_state(state.clone(), crate::audit::record))
}

#[derive(Serialize)]
struct Health {
    product: &'static str,
    version: &'static str,
    core: NodeStatus,
}

/// Whether the panel and its core are up.
async fn health(State(state): State<AppState>, _: Access) -> Json<Health> {
    Json(Health {
        product: z_common::PRODUCT,
        version: z_common::VERSION,
        core: state.node().status().await,
    })
}
