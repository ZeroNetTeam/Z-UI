//! An admin's own API keys. A key acts as its admin, with at most the
//! permissions it names. Making, listing and revoking keys needs a sign-in:
//! a key cannot hand out more keys.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::Audit;
use crate::auth::keys;
use crate::auth::permissions::Permissions;
use crate::auth::Access;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;

/// Most keys one admin may have.
const MAX_KEYS: usize = 50;

#[derive(Serialize)]
pub struct KeyView {
    id: i64,
    name: String,
    /// The start of the key, to tell it from the others.
    prefix: String,
    /// It may do whatever its admin may.
    full: bool,
    /// Otherwise: the permissions it is limited to.
    permissions: Vec<String>,
    allowed_ips: Vec<String>,
    expires_at: Option<i64>,
    expired: bool,
    last_used_at: Option<i64>,
    last_used_ip: Option<String>,
    created_at: i64,
}

impl From<z_db::ApiKey> for KeyView {
    fn from(key: z_db::ApiKey) -> Self {
        let full = key.permissions.trim() == "*";
        Self {
            id: key.id,
            name: key.name,
            prefix: key.prefix,
            full,
            permissions: if full {
                Vec::new()
            } else {
                Permissions::from_stored(&key.permissions).names()
            },
            allowed_ips: key
                .allowed_ips
                .split(',')
                .map(str::trim)
                .filter(|ip| !ip.is_empty())
                .map(str::to_owned)
                .collect(),
            expired: key.expires_at.is_some_and(|at| at <= z_db::now()),
            expires_at: key.expires_at,
            last_used_at: key.last_used_at,
            last_used_ip: key.last_used_ip,
            created_at: key.created_at,
        }
    }
}

pub async fn list(State(state): State<AppState>, access: Access) -> ApiResult<Json<Vec<KeyView>>> {
    access.require_session()?;
    Ok(Json(
        state
            .db()
            .api_keys_of(access.admin.id)
            .await?
            .into_iter()
            .map(KeyView::from)
            .collect(),
    ))
}

/// What a key may do: all its admin may, or a list of permissions.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum Scope {
    All(String),
    Only(Vec<String>),
}

#[derive(Deserialize)]
pub struct KeyInput {
    name: String,
    /// `"*"`, or a list such as `["users.read", "hosts.write"]`.
    permissions: Scope,
    #[serde(default)]
    allowed_ips: Vec<String>,
    expires_at: Option<i64>,
}

#[derive(Serialize)]
pub struct Created {
    /// The key itself, which is shown this once and kept nowhere.
    key: String,
    #[serde(flatten)]
    info: KeyView,
}

pub async fn create(
    State(state): State<AppState>,
    access: Access,
    Json(input): Json<KeyInput>,
) -> ApiResult<(StatusCode, Audit, Json<Created>)> {
    access.require_session()?;
    let name = input.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(ApiError::bad_request(
            "invalid_name",
            "a name of 1 to 64 characters",
        ));
    }
    let permissions = match &input.permissions {
        Scope::All(all) if all.trim() == "*" => "*".to_string(),
        Scope::All(_) => {
            return Err(ApiError::bad_request(
                "invalid_permission",
                "permissions are \"*\" or a list",
            ))
        }
        Scope::Only(list) => {
            let wanted = Permissions::parse(list.iter().map(String::as_str))
                .map_err(|message| ApiError::bad_request("invalid_permission", message))?;
            if wanted == Permissions::NONE {
                return Err(ApiError::bad_request(
                    "invalid_permission",
                    "a key needs at least one permission",
                ));
            }
            if wanted.and(access.permissions) != wanted {
                return Err(ApiError::forbidden(
                    "permission_exceeds",
                    "a key cannot be given more than you may do yourself",
                ));
            }
            wanted.to_stored()
        }
    };
    let ranges = keys::parse_ranges(&input.allowed_ips)
        .map_err(|message| ApiError::bad_request("invalid_address", message))?;
    if input.expires_at.is_some_and(|at| at <= z_db::now()) {
        return Err(ApiError::bad_request(
            "invalid_expiry",
            "the expiry is already past",
        ));
    }
    if state.db().api_keys_of(access.admin.id).await?.len() >= MAX_KEYS {
        return Err(ApiError::conflict(
            "too_many_keys",
            format!("at most {MAX_KEYS} keys; revoke one first"),
        ));
    }
    let (key, prefix, hash) = keys::generate();
    let stored = state
        .db()
        .create_api_key(
            access.admin.id,
            &name,
            &prefix,
            &hash,
            &permissions,
            &ranges.join(","),
            input.expires_at,
        )
        .await?;
    let audit = Audit::new("api_key.create", &name).detail(permissions);
    Ok((
        StatusCode::CREATED,
        audit,
        Json(Created {
            key,
            info: stored.into(),
        }),
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    access: Access,
    Path(id): Path<i64>,
) -> ApiResult<(StatusCode, Audit, ())> {
    access.require_session()?;
    let key = state
        .db()
        .delete_api_key(id, access.admin.id)
        .await?
        .ok_or_else(|| ApiError::not_found("there is no such key"))?;
    Ok((
        StatusCode::NO_CONTENT,
        Audit::new("api_key.delete", &key.name),
        (),
    ))
}
