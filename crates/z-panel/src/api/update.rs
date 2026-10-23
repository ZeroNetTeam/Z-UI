//! Looking for a newer version of the panel, and putting it in place.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::Audit;
use crate::auth::Access;
use crate::certs::settings::redact;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::server::AppState;
use crate::update::{self, Info};

#[derive(Serialize, utoipa::ToSchema)]
pub struct UpdateView {
    /// The version running.
    current: String,
    /// The newest release found at the last look.
    latest: Option<Info>,
    /// When it looked (Unix seconds); 0 for never.
    checked_at: i64,
    /// Why the last look failed, if it did.
    error: Option<String>,
    /// Whether this panel can replace itself where it is.
    can_apply: bool,
    /// Whether it runs in a container, which is updated by a new image.
    container: bool,
    /// Whether it looks now and then and says when there is something new.
    check: bool,
    /// The proxy for the release server, with its password hidden.
    proxy: String,
}

async fn view(state: &AppState) -> UpdateView {
    let status = state.update_status();
    let settings = update::load(state.db()).await;
    let container = update::in_container();
    UpdateView {
        current: z_common::VERSION.into(),
        latest: status.info,
        checked_at: status.checked_at,
        error: status.error,
        can_apply: !container
            && update::archive_name().is_some()
            && update::can_replace(&update::own_executable()),
        container,
        check: settings.check,
        proxy: redact(&settings.proxy),
    }
}

#[utoipa::path(
    get,
    path = "/update",
    operation_id = "update_get",
    tag = "update",
    summary = "Read what is known about newer versions",
    responses((status = 200, description = "The running version, the newest found at the last look, and whether this panel can update itself", body = UpdateView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn get(State(state): State<AppState>, _: Access) -> Json<UpdateView> {
    Json(view(&state).await)
}

#[utoipa::path(
    post,
    path = "/update/check",
    operation_id = "update_check",
    tag = "update",
    summary = "Look for a newer version now",
    responses((status = 200, description = "What the look found; `error` says if it failed", body = UpdateView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn check(State(state): State<AppState>, _: Access) -> (Audit, Json<UpdateView>) {
    update::refresh(&state).await;
    (Audit::new("update.check", ""), Json(view(&state).await))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Applied {
    /// The version now in place; the panel restarts into it in a moment.
    version: String,
}

#[utoipa::path(
    post,
    path = "/update/apply",
    operation_id = "update_apply",
    tag = "update",
    summary = "Update the panel to the newest version",
    description = "Downloads the newest release, checks the signature on its checksums and the archive against them, starts the new program once to see it is that version, puts it in place (keeping the old one), and restarts. Nothing is changed unless every check passes. Not for a panel in a container.",
    responses((status = 200, description = "Updated; the panel restarts in a moment", body = Applied), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn apply(State(state): State<AppState>, _: Access) -> ApiResult<(Audit, Json<Applied>)> {
    let info = update::apply(&state, update::SIGNING_KEY, &update::own_executable()).await?;
    Ok((
        Audit::new("update.apply", &info.version).detail(format!("from {}", info.current)),
        Json(Applied {
            version: info.version,
        }),
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SettingsInput {
    check: bool,
    /// `http://…` or `socks5://…`; empty for none. The hidden form of the
    /// stored one keeps it.
    #[serde(default)]
    proxy: String,
}

#[utoipa::path(
    put,
    path = "/update",
    operation_id = "update_settings",
    tag = "update",
    summary = "Change how the panel looks for newer versions",
    request_body = SettingsInput,
    responses((status = 200, description = "Saved", body = UpdateView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn settings(
    State(state): State<AppState>,
    _: Access,
    Json(input): Json<SettingsInput>,
) -> ApiResult<(Audit, Json<UpdateView>)> {
    let mut stored = update::load(state.db()).await;
    let proxy = input.proxy.trim();
    // The page sends back the proxy as it was shown, password hidden.
    if !(proxy == redact(&stored.proxy) && !stored.proxy.is_empty()) {
        stored.proxy = proxy.to_string();
    }
    stored
        .proxy()
        .map_err(|message| ApiError::bad_request("invalid_proxy", message))?;
    stored.check = input.check;
    update::save(state.db(), &stored).await?;
    Ok((Audit::new("update.settings", ""), Json(view(&state).await)))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::{json, Value};

    use crate::server::testing::{panel, Panel};
    use crate::update::tests::{keypair, scratch, script, serve, Release};
    use crate::update::{self, Settings};

    async fn world(release: Option<Release>) -> (Panel, String) {
        let server = serve(release).await;
        let panel = panel("/").await;
        let owner = panel.set_up("").await;
        let settings = Settings {
            api_base: server.base,
            ..Settings::default()
        };
        update::save(panel.state.db(), &settings).await.unwrap();
        (panel, owner)
    }

    #[tokio::test]
    async fn a_look_finds_the_newer_version_keeps_it_and_says_so_once() {
        let (pair, _) = keypair();
        let (panel, owner) = world(Some(Release::new("9.9.9", pair))).await;
        let before = panel.get("/api/update", Some(&owner)).await.json();
        assert_eq!(before["latest"], Value::Null);
        assert_eq!(before["checked_at"], 0);
        assert_eq!(before["current"], z_common::VERSION);

        let after = panel
            .send("POST", "/api/update/check", Some(&owner), None)
            .await;
        assert_eq!(after.status, StatusCode::OK, "{}", after.body);
        let view = after.json();
        assert_eq!(view["latest"]["version"], "9.9.9");
        assert_eq!(view["latest"]["available"], true);
        assert!(view["checked_at"].as_i64().unwrap() > 0 && view["error"].is_null());
        assert!(
            panel.state.db().was_notified("update:9.9.9").await.unwrap(),
            "told once"
        );
        // What was found is kept for the next time the page asks.
        let kept = panel.get("/api/update", Some(&owner)).await.json();
        assert_eq!(kept["latest"]["version"], "9.9.9");
        assert_eq!(kept["container"], update::in_container());
    }

    #[tokio::test]
    async fn a_look_that_fails_says_why_and_keeps_what_was_known() {
        let (panel, owner) = world(None).await;
        let view = panel
            .send("POST", "/api/update/check", Some(&owner), None)
            .await
            .json();
        assert!(
            view["error"].as_str().unwrap().contains("no release"),
            "{view}"
        );
        assert_eq!(view["latest"], Value::Null);
    }

    #[tokio::test]
    async fn the_settings_are_kept_the_proxy_stays_when_shown_hidden_and_a_bad_one_is_refused() {
        let (panel, owner) = world(None).await;
        let put = |body: Value| panel.send("PUT", "/api/update", Some(&owner), Some(body));
        let saved =
            put(json!({"check": false, "proxy": "socks5://u:hunter2@127.0.0.1:1080"})).await;
        assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
        assert!(!saved.body.contains("hunter2"), "{}", saved.body);
        let shown = saved.json();
        assert_eq!(shown["check"], false);
        // The page sends back what it was shown: the stored proxy is kept.
        put(json!({"check": true, "proxy": shown["proxy"]})).await;
        let stored = update::load(panel.state.db()).await;
        assert_eq!(stored.proxy, "socks5://u:hunter2@127.0.0.1:1080");
        assert!(stored.check);
        let bad = put(json!({"check": true, "proxy": "ftp://x"})).await;
        assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.body);
        assert_eq!(bad.json()["error"], "invalid_proxy");
        let cleared = put(json!({"check": true, "proxy": ""})).await.json();
        assert_eq!(cleared["proxy"], "");
    }

    #[tokio::test]
    async fn applying_puts_the_new_program_in_place_and_restarts_and_refuses_what_it_should() {
        let (pair, public) = keypair();
        let (panel, _) = world(Some(Release::new("9.9.9", pair))).await;
        let dir = scratch("api-apply");
        let exe = dir.join("z-ui");
        std::fs::write(&exe, script("0.1.0")).unwrap();

        // In a container the program is not replaced.
        let refused = update::apply_in(&panel.state, &public, &exe, true)
            .await
            .unwrap_err();
        assert_eq!(
            (refused.status, refused.code),
            (StatusCode::CONFLICT, "container")
        );
        assert_eq!(std::fs::read(&exe).unwrap(), script("0.1.0"));

        // Signed by someone else than the key the panel was built with: refused, untouched.
        let (_, other) = keypair();
        let wrong = update::apply_in(&panel.state, &other, &exe, false)
            .await
            .unwrap_err();
        assert_eq!(wrong.code, "update_failed");
        assert!(
            wrong.message.contains("not the maintainers'"),
            "{}",
            wrong.message
        );
        assert_eq!(std::fs::read(&exe).unwrap(), script("0.1.0"));

        let done = update::apply_in(&panel.state, &public, &exe, false)
            .await
            .unwrap();
        assert_eq!(done.version, "9.9.9");
        assert_eq!(std::fs::read(&exe).unwrap(), script("9.9.9"));
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            panel.state.restart_requested(),
        )
        .await
        .expect("it restarts into the new program");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn nothing_is_applied_when_the_newest_version_is_the_one_running() {
        let (pair, public) = keypair();
        let (panel, _) = world(Some(Release::new(z_common::VERSION, pair))).await;
        let dir = scratch("api-same");
        let exe = dir.join("z-ui");
        std::fs::write(&exe, script("0.1.0")).unwrap();
        let error = update::apply_in(&panel.state, &public, &exe, false)
            .await
            .unwrap_err();
        assert_eq!(
            (error.status, error.code),
            (StatusCode::CONFLICT, "up_to_date")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
