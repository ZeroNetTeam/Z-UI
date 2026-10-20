//! Hosts: how clients reach an inbound, for the links built from it.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::Audit;
use crate::auth::Access;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;

#[derive(Serialize)]
pub struct HostView {
    id: i64,
    inbound_id: i64,
    remark: String,
    address: String,
    port: Option<i64>,
    sni: String,
    host: String,
    path: String,
    alpn: String,
    fingerprint: String,
    allow_insecure: bool,
    enabled: bool,
    priority: i64,
}

impl From<z_db::Host> for HostView {
    fn from(h: z_db::Host) -> Self {
        let f = h.fields;
        Self {
            id: h.id,
            inbound_id: h.inbound_id,
            remark: f.remark,
            address: f.address,
            port: f.port,
            sni: f.sni,
            host: f.host,
            path: f.path,
            alpn: f.alpn,
            fingerprint: f.fingerprint,
            allow_insecure: f.allow_insecure,
            enabled: f.enabled,
            priority: f.priority,
        }
    }
}

#[derive(Deserialize)]
pub struct HostInput {
    inbound_id: i64,
    remark: String,
    address: String,
    port: Option<i64>,
    #[serde(default)]
    sni: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    alpn: String,
    #[serde(default)]
    fingerprint: String,
    #[serde(default)]
    allow_insecure: bool,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    priority: i64,
}

fn yes() -> bool {
    true
}

/// Fingerprints a client can imitate, as share links name them.
const FINGERPRINTS: &[&str] = &[
    "",
    "chrome",
    "firefox",
    "safari",
    "ios",
    "android",
    "edge",
    "360",
    "qq",
    "random",
    "randomized",
];

async fn checked(state: &AppState, input: HostInput) -> Result<(i64, z_db::HostFields), ApiError> {
    if state.db().inbound(input.inbound_id).await?.is_none() {
        return Err(ApiError::bad_request(
            "invalid_inbound",
            "there is no such inbound",
        ));
    }
    let address = input.address.trim();
    if address.is_empty() || address.len() > 255 || address.contains(char::is_whitespace) {
        return Err(ApiError::bad_request(
            "invalid_address",
            "an address or domain is needed",
        ));
    }
    if input.port.is_some_and(|port| !(1..=65535).contains(&port)) {
        return Err(ApiError::bad_request(
            "invalid_port",
            "a port is between 1 and 65535",
        ));
    }
    if !FINGERPRINTS.contains(&input.fingerprint.as_str()) {
        return Err(ApiError::bad_request(
            "invalid_fingerprint",
            "unknown fingerprint",
        ));
    }
    let remark = input.remark.trim();
    if remark.is_empty() {
        return Err(ApiError::bad_request("invalid_remark", "a name is needed"));
    }
    Ok((
        input.inbound_id,
        z_db::HostFields {
            remark: remark.chars().take(128).collect(),
            address: address.to_string(),
            port: input.port,
            sni: input.sni.trim().to_string(),
            host: input.host.trim().to_string(),
            path: input.path.trim().to_string(),
            alpn: input.alpn.trim().to_string(),
            fingerprint: input.fingerprint,
            allow_insecure: input.allow_insecure,
            enabled: input.enabled,
            priority: input.priority,
        },
    ))
}

fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not_found", "there is no such host")
}

pub async fn list(State(state): State<AppState>, _: Access) -> ApiResult<Json<Vec<HostView>>> {
    Ok(Json(
        state
            .db()
            .hosts()
            .await?
            .into_iter()
            .map(HostView::from)
            .collect(),
    ))
}

pub async fn create(
    State(state): State<AppState>,
    _: Access,
    Json(input): Json<HostInput>,
) -> ApiResult<(StatusCode, Audit, Json<HostView>)> {
    let (inbound_id, fields) = checked(&state, input).await?;
    let host = state.db().create_host(inbound_id, &fields).await?;
    let audit = Audit::new("host.create", &host.fields.address);
    Ok((StatusCode::CREATED, audit, Json(host.into())))
}

pub async fn update(
    State(state): State<AppState>,
    _: Access,
    Path(id): Path<i64>,
    Json(input): Json<HostInput>,
) -> ApiResult<(Audit, Json<HostView>)> {
    let (inbound_id, fields) = checked(&state, input).await?;
    let host = state
        .db()
        .update_host(id, inbound_id, &fields)
        .await?
        .ok_or_else(not_found)?;
    let audit = Audit::new("host.update", &host.fields.address);
    Ok((audit, Json(host.into())))
}

pub async fn delete(
    State(state): State<AppState>,
    _: Access,
    Path(id): Path<i64>,
) -> ApiResult<(StatusCode, Audit, ())> {
    let address = state
        .db()
        .hosts()
        .await?
        .into_iter()
        .find(|host| host.id == id)
        .map(|host| host.fields.address);
    if state.db().delete_host(id).await? {
        let audit = Audit::new("host.delete", address.unwrap_or_else(|| format!("#{id}")));
        Ok((StatusCode::NO_CONTENT, audit, ()))
    } else {
        Err(not_found())
    }
}
