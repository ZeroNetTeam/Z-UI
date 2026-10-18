//! Inbounds: listed, made, changed and removed, each change applied to the
//! core before it is stored (see [`crate::core_config`]).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use z_common::node::Applied;

use crate::auth::CurrentAdmin;
use crate::core_config;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;

/// An inbound as the dashboard sees it: the JSON parts parsed.
#[derive(Debug, Serialize)]
pub struct InboundView {
    id: i64,
    tag: String,
    remark: String,
    enabled: bool,
    listen: String,
    port: i64,
    protocol: String,
    settings: Value,
    stream: Value,
    sniffing: Value,
    created_at: i64,
    updated_at: i64,
}

impl From<z_db::Inbound> for InboundView {
    fn from(inbound: z_db::Inbound) -> Self {
        let parse = |text: &str| serde_json::from_str(text).unwrap_or(Value::Null);
        Self {
            settings: parse(&inbound.settings),
            stream: parse(&inbound.stream),
            sniffing: parse(&inbound.sniffing),
            id: inbound.id,
            tag: inbound.tag,
            remark: inbound.remark,
            enabled: inbound.enabled,
            listen: inbound.listen,
            port: inbound.port,
            protocol: inbound.protocol,
            created_at: inbound.created_at,
            updated_at: inbound.updated_at,
        }
    }
}

/// What the dashboard sends to make or change an inbound.
#[derive(Debug, Deserialize)]
pub struct InboundInput {
    #[serde(default)]
    tag: String,
    remark: String,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    listen: String,
    port: i64,
    protocol: String,
    #[serde(default = "empty_object")]
    settings: Value,
    #[serde(default = "empty_object")]
    stream: Value,
    #[serde(default = "empty_object")]
    sniffing: Value,
}

fn yes() -> bool {
    true
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

#[derive(Serialize)]
pub struct Changed<T> {
    #[serde(flatten)]
    item: T,
    /// What the core did with the change.
    applied: Applied,
}

/// A tag made of letters, digits, `-`, `_` and `.`, as routing rules name it.
fn check_tag(tag: &str) -> Result<(), ApiError> {
    let valid = (1..=64).contains(&tag.len())
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "invalid_tag",
            "a tag is 1 to 64 letters, digits, '-', '_' or '.'",
        ))
    }
}

/// Check `input` and turn it into what is stored. `taken` holds the tags of
/// the other inbounds, to make or check a unique one.
fn fields(input: InboundInput, taken: &[&str]) -> Result<z_db::InboundFields, ApiError> {
    if core_config::accounts_key(&input.protocol).is_none() {
        return Err(ApiError::bad_request(
            "invalid_protocol",
            format!("the panel does not make {:?} inbounds", input.protocol),
        ));
    }
    if !(1..=65535).contains(&input.port) {
        return Err(ApiError::bad_request(
            "invalid_port",
            "a port is between 1 and 65535",
        ));
    }
    for (name, value) in [("settings", &input.settings), ("stream", &input.stream)] {
        if !value.is_object() {
            return Err(ApiError::bad_request(
                "invalid_inbound",
                format!("{name} must be a JSON object"),
            ));
        }
    }
    let remark = input.remark.trim();
    if remark.is_empty() || remark.chars().count() > 128 {
        return Err(ApiError::bad_request(
            "invalid_remark",
            "a name of 1 to 128 characters is needed",
        ));
    }
    let tag = match input.tag.trim() {
        "" => {
            // `vless-443`, then `vless-443-2`, … until free.
            let base = format!("{}-{}", input.protocol, input.port);
            let mut tag = base.clone();
            let mut n = 2;
            while taken.contains(&tag.as_str()) {
                tag = format!("{base}-{n}");
                n += 1;
            }
            tag
        }
        tag => {
            check_tag(tag)?;
            if taken.contains(&tag) {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "tag_taken",
                    format!("another inbound is tagged {tag:?}"),
                ));
            }
            tag.to_string()
        }
    };
    Ok(z_db::InboundFields {
        tag,
        remark: remark.to_string(),
        enabled: input.enabled,
        listen: input.listen.trim().to_string(),
        port: input.port,
        protocol: input.protocol,
        settings: input.settings.to_string(),
        stream: input.stream.to_string(),
        sniffing: input.sniffing.to_string(),
    })
}

/// An inbound as it will be stored, for building the configuration first.
fn draft(id: i64, fields: &z_db::InboundFields) -> z_db::Inbound {
    z_db::Inbound {
        id,
        tag: fields.tag.clone(),
        remark: fields.remark.clone(),
        enabled: fields.enabled,
        listen: fields.listen.clone(),
        port: fields.port,
        protocol: fields.protocol.clone(),
        settings: fields.settings.clone(),
        stream: fields.stream.clone(),
        sniffing: fields.sniffing.clone(),
        created_at: 0,
        updated_at: 0,
    }
}

fn not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "not_found",
        "there is no such inbound",
    )
}

pub async fn list(
    State(state): State<AppState>,
    _: CurrentAdmin,
) -> ApiResult<Json<Vec<InboundView>>> {
    let inbounds = state.db().inbounds().await?;
    Ok(Json(inbounds.into_iter().map(InboundView::from).collect()))
}

pub async fn get(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
) -> ApiResult<Json<InboundView>> {
    let inbound = state.db().inbound(id).await?.ok_or_else(not_found)?;
    Ok(Json(inbound.into()))
}

pub async fn create(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Json(input): Json<InboundInput>,
) -> ApiResult<(StatusCode, Json<Changed<InboundView>>)> {
    let _guard = state.core_lock().lock().await;
    let mut inbounds = state.db().inbounds().await?;
    let taken: Vec<&str> = inbounds.iter().map(|i| i.tag.as_str()).collect();
    let fields = fields(input, &taken)?;
    inbounds.push(draft(0, &fields));
    let applied = core_config::apply(&state, &inbounds).await?;
    let inbound = state.db().create_inbound(&fields).await?;
    tracing::info!(tag = %inbound.tag, port = inbound.port, "inbound created");
    Ok((
        StatusCode::CREATED,
        Json(Changed {
            item: inbound.into(),
            applied,
        }),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
    Json(input): Json<InboundInput>,
) -> ApiResult<Json<Changed<InboundView>>> {
    let _guard = state.core_lock().lock().await;
    let mut inbounds = state.db().inbounds().await?;
    let index = inbounds
        .iter()
        .position(|inbound| inbound.id == id)
        .ok_or_else(not_found)?;
    let taken: Vec<&str> = inbounds
        .iter()
        .filter(|inbound| inbound.id != id)
        .map(|inbound| inbound.tag.as_str())
        .collect();
    // An empty tag keeps the one it has.
    let mut input = input;
    if input.tag.trim().is_empty() {
        input.tag = inbounds[index].tag.clone();
    }
    let fields = fields(input, &taken)?;
    inbounds[index] = draft(id, &fields);
    let applied = core_config::apply(&state, &inbounds).await?;
    let inbound = state
        .db()
        .update_inbound(id, &fields)
        .await?
        .ok_or_else(not_found)?;
    Ok(Json(Changed {
        item: inbound.into(),
        applied,
    }))
}

#[derive(Deserialize)]
pub struct EnabledInput {
    enabled: bool,
}

/// Turn an inbound on or off, keeping everything else.
pub async fn set_enabled(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
    Json(input): Json<EnabledInput>,
) -> ApiResult<Json<Changed<InboundView>>> {
    let _guard = state.core_lock().lock().await;
    let mut inbounds = state.db().inbounds().await?;
    let inbound = inbounds
        .iter_mut()
        .find(|inbound| inbound.id == id)
        .ok_or_else(not_found)?;
    inbound.enabled = input.enabled;
    let fields = z_db::InboundFields {
        tag: inbound.tag.clone(),
        remark: inbound.remark.clone(),
        enabled: inbound.enabled,
        listen: inbound.listen.clone(),
        port: inbound.port,
        protocol: inbound.protocol.clone(),
        settings: inbound.settings.clone(),
        stream: inbound.stream.clone(),
        sniffing: inbound.sniffing.clone(),
    };
    let applied = core_config::apply(&state, &inbounds).await?;
    let inbound = state
        .db()
        .update_inbound(id, &fields)
        .await?
        .ok_or_else(not_found)?;
    Ok(Json(Changed {
        item: inbound.into(),
        applied,
    }))
}

pub async fn delete(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
) -> ApiResult<Json<Applied>> {
    let _guard = state.core_lock().lock().await;
    let mut inbounds = state.db().inbounds().await?;
    let before = inbounds.len();
    inbounds.retain(|inbound| inbound.id != id);
    if inbounds.len() == before {
        return Err(not_found());
    }
    let applied = core_config::apply(&state, &inbounds).await?;
    state.db().delete_inbound(id).await?;
    state.db().forget_inbound_in_groups(id).await?;
    Ok(Json(applied))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::{json, Value};

    use crate::server::testing::panel;

    const CERT: &str = include_str!(
        "../../../../resources/zray-core/crates/zero-runtime/tests/fixtures/loopback-cert.pem"
    );
    const KEY: &str = include_str!(
        "../../../../resources/zray-core/crates/zero-runtime/tests/fixtures/loopback-key.pem"
    );
    const OTHER_KEY: &str = include_str!(
        "../../../../resources/zray-core/crates/zero-runtime/tests/fixtures/big-decoy-key.pem"
    );

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    async fn listening(port: u16) -> bool {
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
    }

    async fn reality(panel: &crate::server::testing::Panel, session: &str, port: u16) -> Value {
        let keys = panel.get("/api/keys/x25519", Some(session)).await.json();
        let short = panel
            .get("/api/keys/short-id?bytes=8", Some(session))
            .await
            .json();
        json!({
            "remark": "Reality", "listen": "127.0.0.1", "port": port, "protocol": "vless",
            "settings": {"decryption": "none", "flow": "xtls-rprx-vision"},
            "stream": {"network": "raw", "security": "reality", "realitySettings": {
                "target": "www.example.com:443", "serverNames": ["www.example.com"],
                "privateKey": keys["private_key"], "publicKey": keys["public_key"],
                "shortIds": [short["short_id"]],
            }},
            "sniffing": {"enabled": true, "destOverride": ["http", "tls"]},
        })
    }

    #[tokio::test]
    async fn an_inbound_starts_the_core_and_its_changes_reach_it() {
        let panel = panel("/").await;
        let session = panel.set_up("").await;
        assert_eq!(
            panel.get("/api/inbounds", None).await.status,
            StatusCode::UNAUTHORIZED
        );

        let port = free_port();
        let created = panel
            .send(
                "POST",
                "/api/inbounds",
                Some(&session),
                Some(reality(&panel, &session, port).await),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        let created = created.json();
        assert_eq!(created["applied"], "started");
        assert_eq!(created["tag"], format!("vless-{port}"));
        assert!(listening(port).await, "the core listens on the new port");
        let health = panel.get("/api/health", Some(&session)).await.json();
        assert_eq!(health["core"]["running"], true);
        assert_eq!(health["core"]["inbounds"], json!([format!("vless-{port}")]));

        // Moving it is a handover to a new listener.
        let id = created["id"].as_i64().unwrap();
        let moved_port = free_port();
        let mut moved = reality(&panel, &session, moved_port).await;
        moved["remark"] = json!("Moved");
        let reply = panel
            .send(
                "PUT",
                &format!("/api/inbounds/{id}"),
                Some(&session),
                Some(moved),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(reply.json()["applied"], "restarted");
        assert_eq!(
            reply.json()["tag"],
            format!("vless-{port}"),
            "the tag stays"
        );
        assert!(listening(moved_port).await);

        // Off: the last inbound gone, the core stops.
        let off = panel
            .send(
                "PUT",
                &format!("/api/inbounds/{id}/enabled"),
                Some(&session),
                Some(json!({"enabled": false})),
            )
            .await;
        assert_eq!(off.json()["applied"], "stopped", "{}", off.body);
        assert!(!listening(moved_port).await);
        let on = panel
            .send(
                "PUT",
                &format!("/api/inbounds/{id}/enabled"),
                Some(&session),
                Some(json!({"enabled": true})),
            )
            .await;
        assert_eq!(on.json()["applied"], "started");

        let deleted = panel
            .send(
                "DELETE",
                &format!("/api/inbounds/{id}"),
                Some(&session),
                None,
            )
            .await;
        assert_eq!(deleted.json(), json!("stopped"));
        assert!(panel
            .get("/api/inbounds", Some(&session))
            .await
            .json()
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn the_core_judges_and_nothing_it_refuses_is_stored() {
        let panel = panel("/").await;
        let session = panel.set_up("").await;
        let port = free_port();
        let first = panel
            .send(
                "POST",
                "/api/inbounds",
                Some(&session),
                Some(reality(&panel, &session, port).await),
            )
            .await;
        assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);

        // The same port again.
        let twin = panel
            .send(
                "POST",
                "/api/inbounds",
                Some(&session),
                Some(reality(&panel, &session, port).await),
            )
            .await;
        assert_eq!(twin.json()["error"], "port_conflict", "{}", twin.body);

        // Settings the core cannot use: its own words come back.
        let mut broken = reality(&panel, &session, free_port()).await;
        broken["stream"]["realitySettings"]["privateKey"] = json!("not-a-key");
        let refused = panel
            .send("POST", "/api/inbounds", Some(&session), Some(broken))
            .await;
        assert_eq!(
            refused.json()["error"],
            "invalid_inbound",
            "{}",
            refused.body
        );
        assert!(refused.json()["message"]
            .as_str()
            .unwrap()
            .contains("privateKey"));

        // A port something else holds is refused before anything stops.
        let squatter = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = squatter.local_addr().unwrap().port();
        let busy = panel
            .send(
                "POST",
                "/api/inbounds",
                Some(&session),
                Some(reality(&panel, &session, taken).await),
            )
            .await;
        assert_eq!(busy.json()["error"], "core_refused", "{}", busy.body);
        assert!(listening(port).await, "the first inbound kept serving");

        let mut wrong = reality(&panel, &session, free_port()).await;
        wrong["protocol"] = json!("telnet");
        assert_eq!(
            panel
                .send("POST", "/api/inbounds", Some(&session), Some(wrong))
                .await
                .json()["error"],
            "invalid_protocol"
        );
        let stored = panel.get("/api/inbounds", Some(&session)).await.json();
        assert_eq!(
            stored.as_array().unwrap().len(),
            1,
            "only the inbound the core took"
        );
        drop(squatter);
    }

    #[tokio::test]
    async fn certificates_are_checked_shared_and_kept_while_used() {
        let panel = panel("/").await;
        let session = panel.set_up("").await;
        let mismatched = panel
            .send(
                "POST",
                "/api/certificates",
                Some(&session),
                Some(json!({"cert_pem": CERT, "key_pem": OTHER_KEY})),
            )
            .await;
        assert_eq!(
            mismatched.json()["error"],
            "invalid_certificate",
            "{}",
            mismatched.body
        );

        let created = panel
            .send(
                "POST",
                "/api/certificates",
                Some(&session),
                Some(json!({"cert_pem": CERT, "key_pem": KEY})),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        let certificate = created.json();
        let domains = certificate["domains"].as_array().unwrap();
        assert!(domains.contains(&json!("zray.test")), "{certificate}");
        assert!(
            certificate["not_after"].as_i64().unwrap()
                > certificate["not_before"].as_i64().unwrap()
        );
        assert!(
            certificate.get("key_pem").is_none(),
            "the key is never sent back"
        );
        let id = certificate["id"].as_i64().unwrap();

        let port = free_port();
        let trojan = panel
            .send(
                "POST",
                "/api/inbounds",
                Some(&session),
                Some(json!({
                    "remark": "Trojan TLS", "listen": "127.0.0.1", "port": port, "protocol": "trojan",
                    "stream": {"network": "raw", "security": "tls", "tlsSettings": {"certificateId": id, "alpn": ["http/1.1"]}},
                })),
            )
            .await;
        assert_eq!(trojan.status, StatusCode::CREATED, "{}", trojan.body);
        assert!(listening(port).await);

        let in_use = panel
            .send(
                "DELETE",
                &format!("/api/certificates/{id}"),
                Some(&session),
                None,
            )
            .await;
        assert_eq!(in_use.status, StatusCode::CONFLICT);
        let inbound = trojan.json()["id"].as_i64().unwrap();
        panel
            .send(
                "DELETE",
                &format!("/api/inbounds/{inbound}"),
                Some(&session),
                None,
            )
            .await;
        let freed = panel
            .send(
                "DELETE",
                &format!("/api/certificates/{id}"),
                Some(&session),
                None,
            )
            .await;
        assert_eq!(freed.status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn hosts_belong_to_an_inbound() {
        let panel = panel("/").await;
        let session = panel.set_up("").await;
        let inbound = panel
            .send(
                "POST",
                "/api/inbounds",
                Some(&session),
                Some(reality(&panel, &session, free_port()).await),
            )
            .await
            .json();
        let id = inbound["id"].as_i64().unwrap();
        let host = json!({"inbound_id": id, "remark": "Direct", "address": "203.0.113.5", "fingerprint": "chrome"});
        let created = panel
            .send("POST", "/api/hosts", Some(&session), Some(host.clone()))
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        let mut bad = host.clone();
        bad["fingerprint"] = json!("netscape");
        assert_eq!(
            panel
                .send("POST", "/api/hosts", Some(&session), Some(bad))
                .await
                .json()["error"],
            "invalid_fingerprint"
        );
        let mut orphan = host;
        orphan["inbound_id"] = json!(9999);
        assert_eq!(
            panel
                .send("POST", "/api/hosts", Some(&session), Some(orphan))
                .await
                .json()["error"],
            "invalid_inbound"
        );
        assert_eq!(
            panel
                .get("/api/hosts", Some(&session))
                .await
                .json()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        panel
            .send(
                "DELETE",
                &format!("/api/inbounds/{id}"),
                Some(&session),
                None,
            )
            .await;
        assert!(panel
            .get("/api/hosts", Some(&session))
            .await
            .json()
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn keys_are_made_on_request() {
        let panel = panel("/").await;
        let session = panel.set_up("").await;
        let pair = panel.get("/api/keys/x25519", Some(&session)).await.json();
        let again = panel
            .get(
                &format!(
                    "/api/keys/x25519?private={}",
                    pair["private_key"].as_str().unwrap()
                ),
                Some(&session),
            )
            .await
            .json();
        assert_eq!(pair, again, "a private key gives back its public half");
        assert_eq!(
            panel.get("/api/keys/uuid", Some(&session)).await.json()["uuid"]
                .as_str()
                .unwrap()
                .len(),
            36
        );
        let ss = panel
            .get("/api/keys/password?bytes=16", Some(&session))
            .await
            .json();
        assert_eq!(
            ss["password"].as_str().unwrap().len(),
            24,
            "16 bytes in base64"
        );
        let enc = panel
            .get("/api/keys/vless-encryption", Some(&session))
            .await
            .json();
        assert!(enc["decryption"]
            .as_str()
            .unwrap()
            .starts_with("mlkem768x25519plus."));
        assert_eq!(
            panel.get("/api/keys/uuid", None).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
}
