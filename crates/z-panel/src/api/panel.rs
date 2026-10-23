//! Where the panel listens, under which path, and with which certificate.
//! What is stored applies at the next start; a restart can be asked for here,
//! and swapping one certificate for another is done at once.

use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::Audit;
use crate::auth::Access;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::panel_settings::{self, Stored};
use crate::server::AppState;

/// Which settings a flag or an environment variable fixed.
#[derive(Serialize, utoipa::ToSchema)]
pub struct Locked {
    listen: bool,
    base_path: bool,
    certificate: bool,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct PanelView {
    /// The address and port the panel listens on now.
    listen: String,
    /// The path it is served under now (`/` for none).
    base_path: String,
    /// The certificate of the store it serves HTTPS with now, when it uses
    /// one (not when it was given certificate files).
    certificate_id: Option<i64>,
    /// Whether it serves HTTPS now.
    tls: bool,
    /// Which of these a flag or `ZUI_*` variable fixed: changing them here
    /// is refused.
    locked: Locked,
    /// What is stored, which is what the panel will use after a restart.
    stored_listen: String,
    stored_base_path: String,
    stored_certificate_id: Option<i64>,
    /// Whether a restart is needed for the stored settings to be in force.
    restart_required: bool,
}

/// What the panel would use after a restart, given what is stored.
struct Next {
    listen: SocketAddr,
    base_path: String,
    certificate_id: Option<i64>,
}

fn next(state: &AppState, stored: &Stored) -> Next {
    let config = state.config();
    let base_path = match (config.locked.base_path, &stored.base_path) {
        (false, Some(path)) => {
            panel_settings::normalize_base_path(path).unwrap_or_else(|_| config.base_path.clone())
        }
        _ => config.base_path.clone(),
    };
    Next {
        listen: match (config.locked.listen, stored.listen) {
            (false, Some(listen)) => listen,
            _ => config.listen,
        },
        base_path,
        certificate_id: if config.locked.certificate || config.cert.is_some() {
            None
        } else {
            stored.certificate_id
        },
    }
}

fn view(state: &AppState, stored: &Stored) -> PanelView {
    let config = state.config();
    let fixed = config.locked.certificate || config.cert.is_some();
    let wanted = next(state, stored);
    // Moving between certificates is done at once; going to or from HTTP is not.
    let tls_change = !fixed && wanted.certificate_id.is_some() != state.tls();
    PanelView {
        listen: config.listen.to_string(),
        base_path: config.base_path.clone(),
        certificate_id: state
            .panel_certificate()
            .filter(|_| state.tls() && config.cert.is_none()),
        tls: state.tls(),
        locked: Locked {
            listen: config.locked.listen,
            base_path: config.locked.base_path,
            certificate: fixed,
        },
        stored_listen: wanted.listen.to_string(),
        stored_base_path: wanted.base_path.clone(),
        stored_certificate_id: wanted.certificate_id,
        restart_required: wanted.listen != config.listen
            || wanted.base_path != config.base_path
            || tls_change,
    }
}

#[utoipa::path(
    get,
    path = "/panel",
    operation_id = "panel_get",
    tag = "panel",
    summary = "Read where the panel listens",
    responses((status = 200, description = "What the panel uses now and what it will after a restart", body = PanelView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn get(State(state): State<AppState>, _: Access) -> Json<PanelView> {
    let stored = panel_settings::load(state.db()).await;
    Json(view(&state, &stored))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct PanelInput {
    /// `ip:port`, `0.0.0.0:2053`.
    listen: String,
    /// `/` or `/secret`.
    base_path: String,
    /// A certificate of the store to serve HTTPS with; empty for plain HTTP.
    certificate_id: Option<i64>,
}

#[utoipa::path(
    put,
    path = "/panel",
    operation_id = "panel_save",
    tag = "panel",
    summary = "Change where the panel listens",
    description = "The address and path apply after a restart (`POST /panel/restart`). A certificate can be swapped for another at once; going between HTTP and HTTPS needs the restart. A setting that a flag or `ZUI_*` variable fixed cannot be changed.",
    request_body = PanelInput,
    responses((status = 200, description = "Saved", body = PanelView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn save(
    State(state): State<AppState>,
    _: Access,
    Json(input): Json<PanelInput>,
) -> ApiResult<(Audit, Json<PanelView>)> {
    let config = state.config();
    let mut stored = panel_settings::load(state.db()).await;

    let listen: SocketAddr = input.listen.trim().parse().map_err(|_| {
        ApiError::bad_request(
            "invalid_listen",
            "the address is `ip:port`, like 0.0.0.0:2053",
        )
    })?;
    if listen.port() == 0 {
        return Err(ApiError::bad_request(
            "invalid_listen",
            "port 0 is not a port to listen on",
        ));
    }
    let base_path = panel_settings::normalize_base_path(&input.base_path)
        .map_err(|message| ApiError::bad_request("invalid_base_path", message))?;

    if config.locked.listen && listen != config.listen {
        return Err(locked("listen address"));
    }
    if config.locked.base_path && base_path != config.base_path {
        return Err(locked("path"));
    }
    if (config.locked.certificate || config.cert.is_some())
        && input.certificate_id != stored.certificate_id
    {
        return Err(locked("certificate"));
    }

    if listen != config.listen {
        if Some(listen) == config.sub_listen {
            return Err(ApiError::bad_request(
                "port_conflict",
                "that is the subscription listener's address",
            ));
        }
        crate::core_config::check_ports(&state.db().inbounds().await?, listen.port())?;
        // Listening is the only way to know it is free.
        if let Err(error) = std::net::TcpListener::bind(listen) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "port_in_use",
                format!("{listen} cannot be listened on: {error}"),
            ));
        }
    }
    if base_path != config.base_path {
        state
            .subscription()
            .checked(&base_path)
            .map_err(|message| ApiError::bad_request("path_conflict", message))?;
    }
    let swapped = match input.certificate_id {
        Some(id) => {
            let key = crate::tls::stored_certificate(state.db(), id)
                .await
                .map_err(|error| ApiError::bad_request("invalid_certificate", error.to_string()))?;
            Some((id, key))
        }
        None => None,
    };

    if !config.locked.listen {
        stored.listen = (listen != config.listen || stored.listen.is_some()).then_some(listen);
    }
    if !config.locked.base_path {
        stored.base_path = (base_path != "/" || stored.base_path.is_some()).then_some(base_path);
    }
    if !(config.locked.certificate || config.cert.is_some()) {
        stored.certificate_id = input.certificate_id;
        // Between two certificates there is nothing to restart.
        if let (Some((id, key)), Some(resolver)) = (swapped, state.tls_resolver()) {
            resolver.set(key);
            state.set_panel_certificate(Some(id));
        }
    }
    panel_settings::save(state.db(), &stored).await?;
    let audit = Audit::new("panel.save", "").detail(format!(
        "listen {listen}, path {}, certificate {}",
        stored.base_path.as_deref().unwrap_or("/"),
        stored
            .certificate_id
            .map_or_else(|| "none".to_string(), |id| format!("#{id}"))
    ));
    Ok((audit, Json(view(&state, &stored))))
}

fn locked(what: &str) -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "locked",
        format!("the {what} was fixed by a flag or an environment variable, which this page cannot override"),
    )
}

#[utoipa::path(
    post,
    path = "/panel/restart",
    operation_id = "panel_restart",
    tag = "panel",
    summary = "Restart the panel",
    description = "The panel answers, stops serving, and starts again with the stored settings, in about a second. Sessions are not kept in memory, so they survive; a different path, or going from HTTPS to HTTP, needs a new sign-in because the cookie is scoped to the path and, over HTTPS, to HTTPS.",
    responses((status = 202, description = "Restarting"), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn restart(State(state): State<AppState>, _: Access) -> (StatusCode, Audit, ()) {
    state.restart_soon(Duration::from_millis(800));
    (StatusCode::ACCEPTED, Audit::new("panel.restart", ""), ())
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::{json, Value};

    use crate::server::testing::{panel, panel_with, Panel, Reply};
    use crate::tls::testing::self_signed;
    use crate::tls::Resolver;

    async fn world() -> (Panel, String) {
        let panel = panel("/").await;
        let owner = panel.set_up("").await;
        (panel, owner)
    }

    async fn error(reply: Reply, status: StatusCode, code: &str) {
        assert_eq!(reply.status, status, "{}", reply.body);
        assert_eq!(reply.json()["error"], code, "{}", reply.body);
    }

    fn input(listen: &str, path: &str, certificate: Value) -> Value {
        json!({"listen": listen, "base_path": path, "certificate_id": certificate})
    }

    async fn store(panel: &Panel, name: &str, days: i64) -> (i64, String) {
        let (cert, key) = self_signed(&[name], days);
        let saved = panel
            .state
            .db()
            .create_certificate(name, name, &cert, &key, "upload", 0, i64::MAX)
            .await
            .unwrap();
        (saved.id, cert)
    }

    #[tokio::test]
    async fn a_fresh_panel_reports_its_defaults_and_nothing_to_restart() {
        let (panel, owner) = world().await;
        let view = panel.get("/api/panel", Some(&owner)).await.json();
        assert_eq!(view["listen"], "0.0.0.0:2053");
        assert_eq!(view["base_path"], "/");
        assert_eq!(view["tls"], false);
        assert_eq!(view["certificate_id"], Value::Null);
        assert_eq!(view["restart_required"], false);
        assert_eq!(view["locked"]["listen"], false);
    }

    #[tokio::test]
    async fn a_new_address_and_path_are_stored_and_wait_for_a_restart() {
        let (panel, owner) = world().await;
        let free = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let saved = panel
            .send(
                "PUT",
                "/api/panel",
                Some(&owner),
                Some(input(&free.to_string(), "secret/", Value::Null)),
            )
            .await;
        assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
        let view = saved.json();
        assert_eq!(
            view["listen"], "0.0.0.0:2053",
            "what runs does not change yet"
        );
        assert_eq!(view["stored_listen"], free.to_string());
        assert_eq!(view["stored_base_path"], "/secret");
        assert_eq!(view["restart_required"], true);

        let stored = crate::panel_settings::load(panel.state.db()).await;
        assert_eq!(stored.listen, Some(free));
        assert_eq!(stored.base_path.as_deref(), Some("/secret"));

        // Putting the running values back leaves nothing to restart for.
        let back = panel
            .send(
                "PUT",
                "/api/panel",
                Some(&owner),
                Some(input("0.0.0.0:2053", "/", Value::Null)),
            )
            .await
            .json();
        assert_eq!(back["restart_required"], false, "{back}");
    }

    #[tokio::test]
    async fn an_address_that_cannot_work_is_refused_and_nothing_is_stored() {
        let (panel, owner) = world().await;
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken_address = taken.local_addr().unwrap().to_string();
        let put = |body: Value| panel.send("PUT", "/api/panel", Some(&owner), Some(body));
        error(
            put(input("not an address", "/", Value::Null)).await,
            StatusCode::BAD_REQUEST,
            "invalid_listen",
        )
        .await;
        error(
            put(input("0.0.0.0:0", "/", Value::Null)).await,
            StatusCode::BAD_REQUEST,
            "invalid_listen",
        )
        .await;
        error(
            put(input("0.0.0.0:2053", "/a b", Value::Null)).await,
            StatusCode::BAD_REQUEST,
            "invalid_base_path",
        )
        .await;
        error(
            put(input(&taken_address, "/", Value::Null)).await,
            StatusCode::CONFLICT,
            "port_in_use",
        )
        .await;
        error(
            put(input("0.0.0.0:2053", "/", json!(999))).await,
            StatusCode::BAD_REQUEST,
            "invalid_certificate",
        )
        .await;
        assert_eq!(
            crate::panel_settings::load(panel.state.db()).await,
            crate::panel_settings::Stored::default()
        );
    }

    #[tokio::test]
    async fn the_path_cannot_be_inside_the_subscription_path() {
        let (panel, owner) = world().await;
        let reply = panel
            .send(
                "PUT",
                "/api/panel",
                Some(&owner),
                Some(input("0.0.0.0:2053", "/sub/panel", Value::Null)),
            )
            .await;
        error(reply, StatusCode::BAD_REQUEST, "path_conflict").await;
    }

    #[tokio::test]
    async fn choosing_a_certificate_for_a_plain_panel_needs_a_restart_and_back_again() {
        let (panel, owner) = world().await;
        let (id, _) = store(&panel, "panel.test", 60).await;
        let view = panel
            .send(
                "PUT",
                "/api/panel",
                Some(&owner),
                Some(input("0.0.0.0:2053", "/", json!(id))),
            )
            .await
            .json();
        assert_eq!(view["stored_certificate_id"], id);
        assert_eq!(view["tls"], false);
        assert_eq!(view["restart_required"], true);
        let view = panel
            .send(
                "PUT",
                "/api/panel",
                Some(&owner),
                Some(input("0.0.0.0:2053", "/", Value::Null)),
            )
            .await
            .json();
        assert_eq!(view["restart_required"], false);
    }

    #[tokio::test]
    async fn one_certificate_is_swapped_for_another_without_a_restart() {
        let (old_cert, old_key) = self_signed(&["panel.test"], 30);
        let resolver = Resolver::from_pem(&old_cert, &old_key).unwrap();
        let panel = panel_with(&[], Some(resolver.clone())).await;
        let owner = panel.set_up("").await;
        let (first, _) = store(&panel, "panel.test", 30).await;
        let (second, second_pem) = store(&panel, "panel.test", 90).await;
        panel.state.set_panel_certificate(Some(first));
        let before = resolver.leaf().unwrap();

        let view = panel
            .send(
                "PUT",
                "/api/panel",
                Some(&owner),
                Some(input("0.0.0.0:2053", "/", json!(second))),
            )
            .await
            .json();
        assert_eq!(view["certificate_id"], second);
        assert_eq!(view["tls"], true);
        assert_eq!(view["restart_required"], false, "{view}");
        let now = resolver.leaf().unwrap();
        assert_ne!(before, now);
        let wanted = rustls_pki_types::CertificateDer::pem_slice_iter(second_pem.as_bytes())
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(now, wanted.as_ref());

        // Back to plain HTTP is a change of kind: that waits.
        let view = panel
            .send(
                "PUT",
                "/api/panel",
                Some(&owner),
                Some(input("0.0.0.0:2053", "/", Value::Null)),
            )
            .await
            .json();
        assert_eq!(view["restart_required"], true);
    }

    #[tokio::test]
    async fn what_a_flag_fixed_cannot_be_changed_from_the_page() {
        let panel = panel_with(
            &["--listen", "127.0.0.1:2999", "--base-path", "/fixed"],
            None,
        )
        .await;
        let owner = panel.set_up("/fixed").await;
        let view = panel.get("/fixed/api/panel", Some(&owner)).await.json();
        assert_eq!(view["locked"]["listen"], true);
        assert_eq!(view["locked"]["base_path"], true);
        assert_eq!(view["locked"]["certificate"], false);
        let put = |body: Value| panel.send("PUT", "/fixed/api/panel", Some(&owner), Some(body));
        error(
            put(input("127.0.0.1:3000", "/fixed", Value::Null)).await,
            StatusCode::CONFLICT,
            "locked",
        )
        .await;
        error(
            put(input("127.0.0.1:2999", "/other", Value::Null)).await,
            StatusCode::CONFLICT,
            "locked",
        )
        .await;
        // Sending what is fixed along with a change to what is not is fine.
        let (id, _) = store(&panel, "panel.test", 60).await;
        let saved = put(input("127.0.0.1:2999", "/fixed", json!(id))).await;
        assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
        let stored = crate::panel_settings::load(panel.state.db()).await;
        assert_eq!(stored.listen, None, "nothing was stored for what is fixed");
        assert_eq!(stored.certificate_id, Some(id));
    }

    #[tokio::test]
    async fn a_restart_is_asked_for_and_arrives_after_the_answer() {
        let (panel, owner) = world().await;
        let reply = panel
            .send("POST", "/api/panel/restart", Some(&owner), None)
            .await;
        assert_eq!(reply.status, StatusCode::ACCEPTED, "{}", reply.body);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            panel.state.restart_requested(),
        )
        .await
        .expect("the restart was signalled");
    }

    use rustls_pki_types::pem::PemObject;
}
