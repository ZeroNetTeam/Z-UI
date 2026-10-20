//! Admins: who can sign in, with which role and which quotas. Only a sudo
//! admin manages them, and the panel never lets the last one go.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::Audit;
use crate::auth::{password, Access};
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;

/// An admin as a sudo admin sees them.
#[derive(Serialize)]
pub struct AdminRow {
    id: i64,
    username: String,
    is_sudo: bool,
    enabled: bool,
    role_id: Option<i64>,
    role: Option<String>,
    totp_enabled: bool,
    last_login_at: Option<i64>,
    created_at: i64,
    user_limit: i64,
    data_quota: i64,
    group_ids: Vec<i64>,
    note: String,
    /// The users they own, the data handed out across them, how many have no
    /// limit to count, and what they used this period.
    users: i64,
    allocated: i64,
    unlimited: i64,
    used: i64,
    keys: i64,
}

fn row(
    admin: z_db::Admin,
    roles: &HashMap<i64, String>,
    held: z_db::Holdings,
    keys: i64,
) -> AdminRow {
    AdminRow {
        role: admin.role_id.and_then(|id| roles.get(&id).cloned()),
        id: admin.id,
        username: admin.username,
        is_sudo: admin.is_sudo,
        enabled: admin.enabled,
        role_id: admin.role_id,
        totp_enabled: admin.totp_secret.is_some(),
        last_login_at: admin.last_login_at,
        created_at: admin.created_at,
        user_limit: admin.user_limit,
        data_quota: admin.data_quota,
        group_ids: admin.group_ids,
        note: admin.note,
        users: held.users,
        allocated: held.allocated,
        unlimited: held.unlimited,
        used: held.used,
        keys,
    }
}

async fn role_names(state: &AppState) -> ApiResult<HashMap<i64, String>> {
    Ok(state
        .db()
        .roles()
        .await?
        .into_iter()
        .map(|role| (role.id, role.name))
        .collect())
}

async fn one(state: &AppState, id: i64) -> ApiResult<AdminRow> {
    let admin = state.db().admin_by_id(id).await?.ok_or_else(not_found)?;
    let roles = role_names(state).await?;
    let held = state.db().holdings(id).await?;
    let keys = state.db().api_key_counts().await?;
    let count = keys.get(&id).copied().unwrap_or(0);
    Ok(row(admin, &roles, held, count))
}

fn not_found() -> ApiError {
    ApiError::not_found("there is no such admin")
}

pub async fn list(State(state): State<AppState>, access: Access) -> ApiResult<Json<Vec<AdminRow>>> {
    access.require_sudo()?;
    let roles = role_names(&state).await?;
    let holdings = state.db().all_holdings().await?;
    let keys = state.db().api_key_counts().await?;
    Ok(Json(
        state
            .db()
            .admins()
            .await?
            .into_iter()
            .map(|admin| {
                let held = holdings.get(&admin.id).copied().unwrap_or_default();
                let count = keys.get(&admin.id).copied().unwrap_or(0);
                row(admin, &roles, held, count)
            })
            .collect(),
    ))
}

/// What a sudo admin sends to make or change an admin.
#[derive(Deserialize)]
pub struct AdminInput {
    /// Only when making one; a username does not change.
    #[serde(default)]
    username: String,
    /// Required when making one; sent later it sets a new password and signs
    /// the admin out everywhere.
    password: Option<String>,
    #[serde(default)]
    is_sudo: bool,
    role_id: Option<i64>,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    user_limit: i64,
    #[serde(default)]
    data_quota: i64,
    #[serde(default)]
    group_ids: Vec<i64>,
    #[serde(default)]
    note: String,
}

fn yes() -> bool {
    true
}

async fn fields(state: &AppState, input: &AdminInput) -> ApiResult<z_db::AdminFields> {
    if input.user_limit < 0 || input.data_quota < 0 {
        return Err(ApiError::bad_request(
            "invalid_limit",
            "limits cannot be negative",
        ));
    }
    let role_id = if input.is_sudo {
        None
    } else {
        let id = input.role_id.ok_or_else(|| {
            ApiError::bad_request("role_required", "an admin who is not sudo needs a role")
        })?;
        if state.db().role(id).await?.is_none() {
            return Err(ApiError::bad_request(
                "invalid_role",
                "there is no such role",
            ));
        }
        Some(id)
    };
    let known: Vec<i64> = state
        .db()
        .access_groups()
        .await?
        .into_iter()
        .map(|group| group.id)
        .collect();
    if input.group_ids.iter().any(|id| !known.contains(id)) {
        return Err(ApiError::bad_request(
            "invalid_group",
            "there is no such group",
        ));
    }
    let mut group_ids = input.group_ids.clone();
    group_ids.sort_unstable();
    group_ids.dedup();
    Ok(z_db::AdminFields {
        is_sudo: input.is_sudo,
        role_id,
        enabled: input.enabled,
        user_limit: input.user_limit,
        data_quota: input.data_quota,
        group_ids,
        note: input.note.chars().take(512).collect(),
    })
}

pub async fn create(
    State(state): State<AppState>,
    access: Access,
    Json(input): Json<AdminInput>,
) -> ApiResult<(StatusCode, Audit, Json<AdminRow>)> {
    access.require_sudo()?;
    let username = input.username.trim();
    password::check_username(username)?;
    let secret = input.password.as_deref().unwrap_or_default();
    password::check_strength(secret)?;
    let fields = fields(&state, &input).await?;
    let hash = password::hash(secret).await?;
    let admin = state
        .db()
        .create_admin_with(username, &hash, &fields)
        .await
        .map_err(|_| {
            ApiError::conflict(
                "username_taken",
                format!("there is already an admin {username:?}"),
            )
        })?;
    let detail = describe(&fields);
    let audit = Audit::new("admin.create", &admin.username).detail(detail);
    Ok((
        StatusCode::CREATED,
        audit,
        Json(one(&state, admin.id).await?),
    ))
}

/// What an admin was set to, for the audit log.
fn describe(fields: &z_db::AdminFields) -> String {
    format!(
        "{}, {}, user limit {}, data quota {}",
        if fields.is_sudo {
            "sudo".to_string()
        } else {
            format!("role #{}", fields.role_id.unwrap_or(0))
        },
        if fields.enabled {
            "enabled"
        } else {
            "disabled"
        },
        fields.user_limit,
        fields.data_quota
    )
}

pub async fn update(
    State(state): State<AppState>,
    access: Access,
    Path(id): Path<i64>,
    Json(input): Json<AdminInput>,
) -> ApiResult<(Audit, Json<AdminRow>)> {
    access.require_sudo()?;
    let before = state.db().admin_by_id(id).await?.ok_or_else(not_found)?;
    let fields = fields(&state, &input).await?;
    if id == access.admin.id
        && (fields.is_sudo != before.is_sudo
            || fields.enabled != before.enabled
            || fields.role_id != before.role_id)
    {
        return Err(ApiError::conflict(
            "cannot_change_self",
            "you cannot change your own role or turn yourself off; ask another sudo admin",
        ));
    }
    let was_sudo = before.is_sudo && before.enabled;
    let stays_sudo = fields.is_sudo && fields.enabled;
    if was_sudo && !stays_sudo && state.db().sudo_count().await? <= 1 {
        return Err(ApiError::conflict(
            "last_sudo",
            "this is the last sudo admin; make another first",
        ));
    }
    if let Some(secret) = input.password.as_deref().filter(|s| !s.is_empty()) {
        password::check_strength(secret)?;
    }
    let after = state
        .db()
        .update_admin(id, &fields)
        .await?
        .ok_or_else(not_found)?;
    let mut detail = describe(&fields);
    if let Some(secret) = input.password.as_deref().filter(|s| !s.is_empty()) {
        let hash = password::hash(secret).await?;
        state.db().set_admin_password(id, &hash).await?;
        detail.push_str(", new password");
    }
    // A disabled admin is signed out at once.
    if before.enabled && !after.enabled {
        state.db().end_admin_sessions(id).await?;
    }
    let audit = Audit::new("admin.update", &after.username).detail(detail);
    Ok((audit, Json(one(&state, id).await?)))
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    /// Who gets their users; the admin deleting them by default.
    transfer_to: Option<i64>,
}

pub async fn delete(
    State(state): State<AppState>,
    access: Access,
    Path(id): Path<i64>,
    Query(query): Query<DeleteQuery>,
) -> ApiResult<(StatusCode, Audit, ())> {
    access.require_sudo()?;
    let target = state.db().admin_by_id(id).await?.ok_or_else(not_found)?;
    if id == access.admin.id {
        return Err(ApiError::conflict(
            "cannot_change_self",
            "you cannot delete yourself",
        ));
    }
    if target.is_sudo && target.enabled && state.db().sudo_count().await? <= 1 {
        return Err(ApiError::conflict(
            "last_sudo",
            "this is the last sudo admin",
        ));
    }
    let heir = query.transfer_to.unwrap_or(access.admin.id);
    if heir == id || state.db().admin_by_id(heir).await?.is_none() {
        return Err(ApiError::bad_request(
            "invalid_heir",
            "the users need an admin to go to",
        ));
    }
    let held = state.db().holdings(id).await?;
    state.db().delete_admin(id, heir).await?;
    let audit = Audit::new("admin.delete", &target.username)
        .detail(format!("{} users moved to admin #{heir}", held.users));
    Ok((StatusCode::NO_CONTENT, audit, ()))
}

/// Turn off an admin's two-factor sign-in, for one who lost their
/// authenticator. It also signs them out.
pub async fn reset_totp(
    State(state): State<AppState>,
    access: Access,
    Path(id): Path<i64>,
) -> ApiResult<(StatusCode, Audit, ())> {
    access.require_sudo()?;
    let target = state.db().admin_by_id(id).await?.ok_or_else(not_found)?;
    if id == access.admin.id {
        return Err(ApiError::conflict(
            "cannot_change_self",
            "turn off your own two-factor sign-in from your account",
        ));
    }
    if target.totp_secret.is_none() {
        return Err(ApiError::bad_request(
            "totp_disabled",
            "two-factor sign-in is already off",
        ));
    }
    state.db().set_admin_totp(id, None).await?;
    Ok((
        StatusCode::NO_CONTENT,
        Audit::new("admin.reset_2fa", &target.username),
        (),
    ))
}

/// End every session an admin has open.
pub async fn sign_out(
    State(state): State<AppState>,
    access: Access,
    Path(id): Path<i64>,
) -> ApiResult<(StatusCode, Audit, ())> {
    access.require_sudo()?;
    let target = state.db().admin_by_id(id).await?.ok_or_else(not_found)?;
    state.db().end_admin_sessions(id).await?;
    Ok((
        StatusCode::NO_CONTENT,
        Audit::new("admin.sign_out", &target.username),
        (),
    ))
}
