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

use std::sync::OnceLock;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::Json;
use axum::Router;
use serde::Serialize;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use z_common::node::NodeStatus;

use crate::auth::access::{guard, Need};
use crate::auth::permissions::{Level, Section};
use crate::auth::Access;
use crate::error::ErrorBody;
use crate::server::AppState;

/// The setting the admin's choice of log level is kept under.
pub const LOG_LEVEL_SETTING: &str = "log.level";

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Z-UI API",
        description = "The panel's JSON API: everything the dashboard does, so scripts and bots can do the same.\n\n\
**Signing in.** Either sign in (`POST /auth/login`), which sets a session cookie, or send an API key made under *API → Keys* as `Authorization: Bearer zk_…` (or `X-API-Key`). A key acts as its admin, with at most the permissions it was given and only from the addresses it names.\n\n\
**Permissions.** Each operation says what it needs (`x-permission`): `<section>.read` for reading, `<section>.write` for changing. `sudo` operations are for sudo admins only; `session` ones need a sign-in, not a key.\n\n\
**Errors.** A failed request answers with an HTTP status and `{ \"error\": code, \"message\": text }`; the code is stable.",
        license(name = "GPL-3.0-or-later"),
    ),
    modifiers(&Security),
    security(("session" = []), ("bearer" = []), ("api_key" = [])),
    tags(
        (name = "auth", description = "Signing in, and the admin's own account"),
        (name = "users", description = "Users, their traffic, subscriptions and devices"),
        (name = "groups", description = "Access groups: which inbounds users reach"),
        (name = "templates", description = "Starting points for new users"),
        (name = "inbounds", description = "Inbounds, which the core takes first"),
        (name = "hosts", description = "The addresses users' links point to"),
        (name = "certificates", description = "TLS certificates"),
        (name = "core", description = "The core's settings: outbounds, routing, DNS"),
        (name = "monitor", description = "The server and what users moved"),
        (name = "logs", description = "The panel's and the core's log"),
        (name = "settings", description = "Panel settings"),
        (name = "tools", description = "Keys and checks for the inbound forms"),
        (name = "admins", description = "Admins, roles and quotas (sudo only)"),
        (name = "api-keys", description = "My API keys"),
        (name = "audit", description = "What admins did"),
        (name = "panel", description = "The panel itself"),
    ),
)]
struct ApiDoc;

struct Security;

impl Modify for Security {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "session",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new(
                crate::auth::session::COOKIE,
            ))),
        );
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("zk_…")
                    .description(Some("An API key from the API page"))
                    .build(),
            ),
        );
        components.add_security_scheme(
            "api_key",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("X-API-Key"))),
        );
    }
}

/// The routes, grouped by what they ask of whoever calls them. A group with
/// no `Need` is open to all. The guard and the documentation both come from
/// this, so the two cannot disagree.
fn groups() -> Vec<(Option<Need>, OpenApiRouter<AppState>)> {
    let router = OpenApiRouter::new;

    // Signing in: open to anyone, answered by the handlers themselves.
    let open = router()
        .routes(routes!(auth::state))
        .routes(routes!(auth::setup))
        .routes(routes!(auth::login))
        .routes(routes!(auth::logout));

    // Anyone signed in: their own account, and tools that touch nothing.
    let signed = router()
        .routes(routes!(health))
        .routes(routes!(openapi_json))
        .routes(routes!(auth::me))
        .routes(routes!(keys::x25519))
        .routes(routes!(keys::wireguard))
        .routes(routes!(keys::mldsa65))
        .routes(routes!(keys::uuid))
        .routes(routes!(keys::short_id))
        .routes(routes!(keys::password))
        .routes(routes!(keys::vless_encryption))
        .routes(routes!(tools::qr));

    // What is done to oneself needs a sign-in, not a key.
    let own = router()
        .routes(routes!(auth::change_password))
        .routes(routes!(auth::totp_begin))
        .routes(routes!(auth::totp_enable))
        .routes(routes!(auth::totp_disable));

    let users = router()
        .routes(routes!(users::list, users::create))
        .routes(routes!(users::counts))
        .routes(routes!(users::bulk))
        .routes(routes!(users::bulk_create))
        .routes(routes!(users::get, users::update, users::delete))
        .routes(routes!(users::reset))
        .routes(routes!(users::revoke))
        .routes(routes!(users::end_sessions))
        .routes(routes!(users::usage))
        .routes(routes!(users::online))
        .routes(routes!(subscription::user_subscription))
        .routes(routes!(subscription::devices, subscription::delete_devices))
        .routes(routes!(subscription::delete_device))
        // What users moved is the users' business, not the server's.
        .routes(routes!(monitor::traffic));

    let groups = router()
        .routes(routes!(groups::list, groups::create))
        .routes(routes!(groups::update, groups::delete));
    let templates = router()
        .routes(routes!(groups::templates, groups::create_template))
        .routes(routes!(groups::update_template, groups::delete_template));
    let inbounds = router()
        .routes(routes!(inbounds::list, inbounds::create))
        .routes(routes!(inbounds::get, inbounds::update, inbounds::delete))
        .routes(routes!(inbounds::set_enabled));
    // It dials a stranger's address on the panel's behalf.
    let probes = router().routes(routes!(tools::reality_check));
    let hosts = router()
        .routes(routes!(hosts::list, hosts::create))
        .routes(routes!(hosts::update, hosts::delete));
    let certificates = router()
        .routes(routes!(certificates::list, certificates::create))
        .routes(routes!(certificates::delete));
    let core = router()
        .routes(routes!(core::get, core::save))
        .routes(routes!(core::create_warp))
        .routes(routes!(core::geodata))
        .routes(routes!(core::refresh_geodata));
    // Questions about a configuration, which change nothing.
    let core_questions = router()
        .routes(routes!(core::validate))
        .routes(routes!(core::iran_preset))
        .routes(routes!(core::check_link));
    let monitor = router().routes(routes!(monitor::system));
    let logs = router()
        .routes(routes!(monitor::logs))
        .routes(routes!(monitor::level, monitor::set_level))
        .routes(routes!(monitor::stream));
    let settings = router().routes(routes!(subscription::settings, subscription::save_settings));
    let audit = router().routes(routes!(audit::list));
    let api_keys = router()
        .routes(routes!(api_keys::list, api_keys::create))
        .routes(routes!(api_keys::delete));
    let admins = router()
        .routes(routes!(admins::list, admins::create))
        .routes(routes!(admins::update, admins::delete))
        .routes(routes!(admins::reset_totp))
        .routes(routes!(admins::sign_out))
        .routes(routes!(roles::list, roles::create))
        .routes(routes!(roles::update, roles::delete));

    vec![
        (None, open),
        (Some(Need::Signed), signed),
        (Some(Need::Session), own),
        (Some(Need::Section(Section::Users)), users),
        (Some(Need::Section(Section::Groups)), groups),
        (Some(Need::Section(Section::Templates)), templates),
        (Some(Need::Section(Section::Inbounds)), inbounds),
        (Some(Need::Exactly(Section::Inbounds, Level::Write)), probes),
        (Some(Need::Section(Section::Hosts)), hosts),
        (Some(Need::Section(Section::Certificates)), certificates),
        (Some(Need::Section(Section::Core)), core),
        (
            Some(Need::Exactly(Section::Core, Level::Read)),
            core_questions,
        ),
        (Some(Need::Section(Section::Monitor)), monitor),
        (Some(Need::Section(Section::Logs)), logs),
        (Some(Need::Section(Section::Settings)), settings),
        (Some(Need::Section(Section::Audit)), audit),
        (Some(Need::SessionSection(Section::Api)), api_keys),
        (Some(Need::Sudo), admins),
    ]
}

/// The API's routes, each behind the guard its group asks for.
pub fn router(state: &AppState) -> Router<AppState> {
    let mut app = Router::new();
    for (need, routes) in groups() {
        let (routes, _) = routes.split_for_parts();
        app = app.merge(match need {
            Some(need) => routes.route_layer(from_fn_with_state((state.clone(), need), guard)),
            None => routes,
        });
    }
    app.fallback(|| async { StatusCode::NOT_FOUND })
        .layer(from_fn_with_state(state.clone(), crate::audit::record))
}

/// The OpenAPI document of the API, built from the same groups as the routes.
pub fn spec() -> &'static utoipa::openapi::OpenApi {
    static SPEC: OnceLock<utoipa::openapi::OpenApi> = OnceLock::new();
    SPEC.get_or_init(|| {
        let mut api = ApiDoc::openapi();
        for (need, routes) in groups() {
            let (_, mut doc) = routes.split_for_parts();
            if let Some(need) = need {
                need.document(&mut doc);
            }
            api.merge(doc);
        }
        api
    })
}

#[derive(Serialize, utoipa::ToSchema)]
struct Health {
    product: &'static str,
    version: &'static str,
    /// Whether the core is running, and how.
    #[schema(value_type = Object)]
    core: NodeStatus,
}

/// Whether the panel and its core are up.
#[utoipa::path(
    get,
    path = "/health",
    operation_id = "health",
    tag = "panel",
    summary = "Whether the panel and its core are up",
    responses((status = 200, description = "The panel and the core's state", body = Health), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
async fn health(State(state): State<AppState>, _: Access) -> Json<Health> {
    Json(Health {
        product: z_common::PRODUCT,
        version: z_common::VERSION,
        core: state.node().status().await,
    })
}

/// This document, with the address the panel was reached at as its server.
#[utoipa::path(
    get,
    path = "/openapi.json",
    operation_id = "openapi_json",
    tag = "panel",
    summary = "This API as an OpenAPI document",
    responses((status = 200, description = "OpenAPI 3.1", body = Object), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
async fn openapi_json(
    State(state): State<AppState>,
    _: Access,
    headers: HeaderMap,
) -> Json<utoipa::openapi::OpenApi> {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost");
    let scheme = if state.tls() { "https" } else { "http" };
    let mut api = spec().clone();
    api.servers = Some(vec![utoipa::openapi::ServerBuilder::new()
        .url(format!(
            "{scheme}://{host}{}",
            state.config().under_base("/api")
        ))
        .description(Some("This panel"))
        .build()]);
    Json(api)
}
