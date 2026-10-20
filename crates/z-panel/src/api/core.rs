//! The core's settings: outbounds, routing, DNS and the rest, as one document
//! that is checked, applied to the running core and stored together
//! ([`crate::core_settings`]); the Iran preset; and the tools the page uses
//! (share links, WARP accounts, geodata).

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use z_common::node::Applied;

use crate::audit::Audit;
use crate::auth::Access;
use crate::core_config::{self, Warning};
use crate::core_settings::{self, IranOptions};
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;

#[derive(Serialize)]
pub struct InboundName {
    tag: String,
    remark: String,
    enabled: bool,
    protocol: String,
}

#[derive(Serialize)]
pub struct CoreView {
    /// What is in force now: the stored document, or the default.
    settings: Value,
    /// What a fresh panel starts with, to go back to.
    defaults: Value,
    running: bool,
    /// The inbounds, which rules name by tag.
    inbounds: Vec<InboundName>,
    /// How fresh the geosite and geoip lists are; none while the core is not
    /// running.
    geodata: Option<Value>,
    /// The lists and mirrors geodata management starts from.
    asset_files: Value,
}

pub async fn get(State(state): State<AppState>, _: Access) -> ApiResult<Json<CoreView>> {
    let inbounds = state
        .db()
        .inbounds()
        .await?
        .into_iter()
        .map(|inbound| InboundName {
            tag: inbound.tag,
            remark: inbound.remark,
            enabled: inbound.enabled,
            protocol: inbound.protocol,
        })
        .collect();
    Ok(Json(CoreView {
        settings: core_settings::load(state.db()).await,
        defaults: core_settings::default_document(),
        running: state.node().status().await.running,
        inbounds,
        geodata: state.node().assets().await,
        asset_files: zero_config::presets::default_asset_files(),
    }))
}

#[derive(Deserialize)]
pub struct SettingsInput {
    settings: Value,
}

#[derive(Serialize)]
pub struct Checked {
    warnings: Vec<Warning>,
}

/// Whether the core would take `settings`, without applying or storing
/// them; what it accepts but does not act on comes back as warnings.
pub async fn validate(
    State(state): State<AppState>,
    _: Access,
    Json(input): Json<SettingsInput>,
) -> ApiResult<Json<Checked>> {
    let inbounds = state.db().inbounds().await?;
    let (_, warnings) = core_config::compile(&state, &inbounds, &input.settings).await?;
    Ok(Json(Checked { warnings }))
}

#[derive(Serialize)]
pub struct Saved {
    settings: Value,
    applied: Applied,
    warnings: Vec<Warning>,
}

/// Apply `settings` to the running core and, only if it takes them, store
/// them.
pub async fn save(
    State(state): State<AppState>,
    _: Access,
    Json(input): Json<SettingsInput>,
) -> ApiResult<(Audit, Json<Saved>)> {
    let _guard = state.core_lock().lock().await;
    let inbounds = state.db().inbounds().await?;
    let (applied, warnings) = core_config::apply_with(&state, &inbounds, &input.settings).await?;
    core_settings::save(state.db(), &input.settings).await?;
    tracing::info!(?applied, "the core's settings were changed");
    Ok((
        Audit::new("core.save", "settings"),
        Json(Saved {
            settings: input.settings,
            applied,
            warnings,
        }),
    ))
}

#[derive(Deserialize)]
pub struct PresetInput {
    #[serde(default)]
    options: IranOptions,
    /// The settings the preset starts from, in the editor now (unsaved
    /// changes included); the stored ones when absent.
    current: Option<Value>,
}

#[derive(Serialize)]
pub struct Proposed {
    settings: Value,
    warnings: Vec<Warning>,
}

/// The settings the Iran preset makes, for the admin to look at and apply.
/// Nothing is stored or applied here; what it proposes is checked, so the
/// admin sees any warning before applying.
pub async fn iran_preset(
    State(state): State<AppState>,
    _: Access,
    Json(input): Json<PresetInput>,
) -> ApiResult<Json<Proposed>> {
    let current = match input.current {
        Some(current) => current,
        None => core_settings::load(state.db()).await,
    };
    let settings = core_settings::iran_preset(&input.options, &current)
        .map_err(|message| ApiError::bad_request("invalid_preset", message))?;
    let inbounds = state.db().inbounds().await?;
    let (_, warnings) = core_config::compile(&state, &inbounds, &settings).await?;
    Ok(Json(Proposed { settings, warnings }))
}

#[derive(Deserialize)]
pub struct LinkInput {
    link: String,
}

#[derive(Serialize)]
pub struct LinkInfo {
    /// The name in the link.
    remark: String,
    /// `vless`, `warp`…, from the link's scheme.
    scheme: String,
}

/// Whether `link` is a share link the core can use as an outbound.
pub async fn check_link(_: Access, Json(input): Json<LinkInput>) -> ApiResult<Json<LinkInfo>> {
    let link = input.link.trim();
    let parsed = zero_config::parse_link(link)
        .map_err(|message| ApiError::bad_request("invalid_link", message))?;
    Ok(Json(LinkInfo {
        remark: parsed.remark,
        scheme: link
            .split_once("://")
            .map_or("", |(scheme, _)| scheme)
            .to_ascii_lowercase(),
    }))
}

#[derive(Serialize)]
pub struct WarpAccount {
    /// A `warp://` link the core takes as an outbound.
    link: String,
}

/// Make a Cloudflare WARP account: the keys are made here and only their
/// public halves are sent. This asks Cloudflare's API from the server, so it
/// can fail where that is filtered.
pub async fn create_warp(_: Access) -> ApiResult<(Audit, Json<WarpAccount>)> {
    let api = zero_discovery::warp::Api::direct();
    let link = zero_discovery::warp::register_with(&api, &|_| {})
        .await
        .map_err(|message| ApiError::bad_request("warp_failed", message))?;
    tracing::info!("a WARP account was made");
    Ok((Audit::new("core.warp", ""), Json(WarpAccount { link })))
}

pub async fn geodata(State(state): State<AppState>, _: Access) -> Json<Option<Value>> {
    Json(state.node().assets().await)
}

/// Fetch the geosite and geoip lists now.
pub async fn refresh_geodata(
    State(state): State<AppState>,
    _: Access,
) -> ApiResult<(Audit, Json<Value>)> {
    match state.node().refresh_assets().await {
        Some(report) => Ok((Audit::new("core.geodata_refresh", ""), Json(report))),
        None => Err(ApiError::bad_request(
            "core_stopped",
            "the core is not running, so there is nothing to refresh",
        )),
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::{json, Value};

    use crate::server::testing::{client, connect, echo_sink, free_port, panel, Panel};

    const LINK: &str = "vless://5783a3e7-e373-51cd-8642-c83782b807c5@203.0.113.9:443?encryption=none&security=reality&sni=www.example.com&fp=chrome&pbk=6X6TqIqO7Hh4kq-OdBzc7r8xbvxz3e0iZ8LgXzn8WlU&sid=ab12cd34&type=tcp&flow=xtls-rprx-vision#out";

    /// A panel with one VLESS inbound and a user, with the private-address
    /// block as shipped, and the user's client port.
    async fn rig() -> (Panel, String, u16, u16) {
        let panel = panel("/").await;
        let session = panel.set_up("").await;
        let port = free_port();
        let made = panel
            .send(
                "POST",
                "/api/inbounds",
                Some(&session),
                Some(json!({
                    "remark": "plain", "listen": "127.0.0.1", "port": port, "protocol": "vless",
                    "settings": {"decryption": "none"}, "stream": {"network": "raw", "security": "none"},
                })),
            )
            .await;
        assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
        let group = panel.get("/api/groups", Some(&session)).await.json()[0]["id"].clone();
        let user = panel
            .send(
                "POST",
                "/api/users",
                Some(&session),
                Some(json!({"username": "alice", "group_ids": [group]})),
            )
            .await;
        let uuid = user.json()["uuid"].as_str().unwrap().to_string();
        let socks = client(port, &uuid).await;
        (panel, session, port, socks)
    }

    async fn settings(panel: &Panel, session: &str) -> Value {
        panel.get("/api/core", Some(session)).await.json()["settings"].clone()
    }

    async fn put(panel: &Panel, session: &str, settings: &Value) -> crate::server::testing::Reply {
        panel
            .send(
                "PUT",
                "/api/core",
                Some(session),
                Some(json!({"settings": settings})),
            )
            .await
    }

    #[tokio::test]
    async fn only_an_admin_reaches_the_core_settings() {
        let panel = panel("/").await;
        for (method, path) in [
            ("GET", "/api/core"),
            ("PUT", "/api/core"),
            ("POST", "/api/core/validate"),
            ("POST", "/api/core/presets/iran"),
            ("POST", "/api/core/check-link"),
            ("POST", "/api/core/warp"),
            ("GET", "/api/core/geodata"),
            ("PUT", "/api/logs/level"),
        ] {
            let reply = panel.send(method, path, None, Some(json!({}))).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method} {path}");
        }
    }

    #[tokio::test]
    async fn a_fresh_panel_has_the_defaults_and_names_its_inbounds() {
        let (panel, session, _, _) = rig().await;
        let view = panel.get("/api/core", Some(&session)).await.json();
        assert_eq!(view["settings"], view["defaults"]);
        assert_eq!(view["settings"]["blockPrivate"], true);
        assert_eq!(view["running"], true);
        assert_eq!(view["inbounds"][0]["remark"], "plain");
        assert!(view["inbounds"][0]["tag"]
            .as_str()
            .unwrap()
            .starts_with("vless-"));
    }

    #[tokio::test]
    async fn settings_are_checked_before_anything_is_applied() {
        let (panel, session, _, _) = rig().await;
        let base = settings(&panel, &session).await;
        let check = |settings: Value| {
            let (panel, session) = (&panel, &session);
            async move {
                panel
                    .send(
                        "POST",
                        "/api/core/validate",
                        Some(session),
                        Some(json!({"settings": settings})),
                    )
                    .await
            }
        };
        let mut with_inbounds = base.clone();
        with_inbounds["inbounds"] = json!([]);
        let refused = check(with_inbounds).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST);
        assert_eq!(refused.json()["error"], "invalid_core");
        assert!(refused.json()["message"]
            .as_str()
            .unwrap()
            .contains("Inbounds page"));

        let mut no_outbounds = base.clone();
        no_outbounds["outbounds"] = json!([]);
        assert_eq!(check(no_outbounds).await.json()["error"], "invalid_core");

        let mut unknown_target = base.clone();
        unknown_target["routing"]["rules"] =
            json!([{"type": "field", "domain": ["example.com"], "outboundTag": "nowhere"}]);
        let refused = check(unknown_target).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);

        // What the core accepts and does not act on comes back as a warning.
        let mut noisy = base.clone();
        noisy["api"] = json!({"tag": "api", "services": ["StatsService"]});
        let accepted = check(noisy).await;
        assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.body);
        let warnings = accepted.json()["warnings"].clone();
        assert!(
            warnings
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w["path"] == "api"),
            "{warnings}"
        );
        assert_eq!(
            settings(&panel, &session).await,
            base,
            "validating stores nothing"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn settings_that_are_applied_change_what_clients_reach() {
        let (panel, session, _, socks) = rig().await;
        let sink = echo_sink().await;
        let base = settings(&panel, &session).await;
        assert!(
            connect(socks, sink, b"x").await.is_none(),
            "private addresses are blocked as shipped"
        );

        // Switched off, the user reaches the sink on the loopback.
        let mut open = base.clone();
        open["blockPrivate"] = json!(false);
        let saved = put(&panel, &session, &open).await;
        assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
        assert_eq!(saved.json()["applied"], "reloaded");
        assert!(connect(socks, sink, b"x").await.is_some());
        assert_eq!(settings(&panel, &session).await["blockPrivate"], false);

        // A rule of the admin's, by port, sends it to the blackhole.
        let mut ruled = open.clone();
        ruled["routing"]["rules"] =
            json!([{"type": "field", "port": sink.to_string(), "outboundTag": "block"}]);
        assert_eq!(put(&panel, &session, &ruled).await.status, StatusCode::OK);
        assert!(
            connect(socks, sink, b"x").await.is_none(),
            "the rule blocks it"
        );

        // A settings change the core refuses is not stored, and changes nothing.
        let mut broken = ruled.clone();
        broken["routing"]["rules"] =
            json!([{"type": "field", "port": "1", "outboundTag": "nowhere"}]);
        let refused = put(&panel, &session, &broken).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
        assert_eq!(
            settings(&panel, &session).await,
            ruled,
            "what was in force stays"
        );
        assert!(connect(socks, sink, b"x").await.is_none());

        // Taking the rule out lets it through again; the settings survive a
        // restart of the core from the database.
        assert_eq!(put(&panel, &session, &open).await.status, StatusCode::OK);
        assert!(connect(socks, sink, b"x").await.is_some());
        crate::core_config::sync(&panel.state).await.unwrap();
        assert!(
            connect(socks, sink, b"x").await.is_some(),
            "stored settings are applied at start"
        );
    }

    #[tokio::test]
    async fn the_iran_preset_is_proposed_not_applied() {
        let (panel, session, _, _) = rig().await;
        let before = settings(&panel, &session).await;
        let proposed = panel
            .send(
                "POST",
                "/api/core/presets/iran",
                Some(&session),
                Some(json!({"options": {"proxies": [LINK], "fragment": true}})),
            )
            .await;
        assert_eq!(proposed.status, StatusCode::OK, "{}", proposed.body);
        let settings_ = proposed.json()["settings"].clone();
        assert_eq!(settings_["outbounds"][0]["tag"], "proxy");
        assert!(settings_["routing"]["rules"].as_array().unwrap().len() > 3);
        assert!(settings_.get("inbounds").is_none());
        assert_eq!(
            settings(&panel, &session).await,
            before,
            "nothing was stored"
        );
        assert!(
            proposed.json()["warnings"].as_array().unwrap().is_empty(),
            "the preset brings resolvers of its own: {}",
            proposed.body
        );

        // The proposal is applicable as it stands.
        let applied = put(&panel, &session, &settings_).await;
        assert_eq!(applied.status, StatusCode::OK, "{}", applied.body);
        let refused = panel
            .send(
                "POST",
                "/api/core/presets/iran",
                Some(&session),
                Some(json!({"options": {"proxies": []}})),
            )
            .await;
        assert_eq!(refused.json()["error"], "invalid_preset");
    }

    #[tokio::test]
    async fn a_proxy_outbound_without_resolvers_is_warned_about() {
        let (panel, session, _, _) = rig().await;
        let check = |settings: Value| {
            let (panel, session) = (&panel, &session);
            async move {
                let reply = panel
                    .send(
                        "POST",
                        "/api/core/validate",
                        Some(session),
                        Some(json!({"settings": settings})),
                    )
                    .await;
                assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
                reply.json()["warnings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|w| w["path"].as_str().unwrap().to_string())
                    .collect::<Vec<_>>()
            }
        };
        let base = settings(&panel, &session).await;
        assert!(
            check(base.clone()).await.is_empty(),
            "a server that sends traffic out directly has nothing to warn of"
        );

        // Any outbound through another server makes the core refuse the system
        // resolver, which is all the defaults ask.
        let mut proxied = base.clone();
        proxied["outbounds"]
            .as_array_mut()
            .unwrap()
            .push(json!({"tag": "p", "link": LINK}));
        let paths = check(proxied.clone()).await;
        assert_eq!(paths, ["dns.leakPolicy", "dns.servers"], "{paths:?}");

        // With a resolver by address the names resolve; only the ignored
        // setting is left to mention.
        proxied["dns"]["servers"] = json!(["1.1.1.1"]);
        assert_eq!(check(proxied.clone()).await, ["dns.leakPolicy"]);
        proxied["dns"].as_object_mut().unwrap().remove("leakPolicy");
        assert!(check(proxied).await.is_empty());
    }

    #[tokio::test]
    async fn links_are_checked() {
        let (panel, session, _, _) = rig().await;
        let ok = panel
            .send(
                "POST",
                "/api/core/check-link",
                Some(&session),
                Some(json!({"link": format!("  {LINK} ")})),
            )
            .await;
        assert_eq!(ok.json(), json!({"remark": "out", "scheme": "vless"}));
        let bad = panel
            .send(
                "POST",
                "/api/core/check-link",
                Some(&session),
                Some(json!({"link": "vless://nope"})),
            )
            .await;
        assert_eq!(bad.status, StatusCode::BAD_REQUEST);
        assert_eq!(bad.json()["error"], "invalid_link");
    }

    #[tokio::test]
    async fn the_log_level_is_changed_and_remembered() {
        let panel = panel("/").await;
        let session = panel.set_up("").await;
        let before = panel.get("/api/logs/level", Some(&session)).await.json();
        assert_eq!(before["level"], "info");
        let set = panel
            .send(
                "PUT",
                "/api/logs/level",
                Some(&session),
                Some(json!({"level": "debug"})),
            )
            .await;
        assert_eq!(set.json()["level"], "debug", "{}", set.body);
        assert_eq!(
            panel.get("/api/logs/level", Some(&session)).await.json()["level"],
            "debug"
        );
        assert_eq!(
            panel
                .state
                .db()
                .setting(crate::api::LOG_LEVEL_SETTING)
                .await
                .unwrap()
                .as_deref(),
            Some("debug")
        );
        let refused = panel
            .send(
                "PUT",
                "/api/logs/level",
                Some(&session),
                Some(json!({"level": "loud"})),
            )
            .await;
        assert_eq!(refused.json()["error"], "invalid_log_level");
    }
}
