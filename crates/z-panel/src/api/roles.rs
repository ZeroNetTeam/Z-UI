//! Roles: what an admin may do, set by section. Only a sudo admin edits them.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::Audit;
use crate::auth::permissions::{Permissions, Section};
use crate::auth::Access;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;

#[derive(Serialize)]
pub struct RoleView {
    id: i64,
    name: String,
    /// Every `<section>.<level>` it gives, writing counting as reading too.
    permissions: Vec<String>,
    /// Whoever has it sees and changes only the users they made.
    own_users_only: bool,
    /// How many admins have it.
    admins: i64,
    created_at: i64,
}

#[derive(Serialize)]
pub struct CatalogEntry {
    section: &'static str,
    /// Whether there is anything to change there, not only to look at.
    writable: bool,
}

#[derive(Serialize)]
pub struct RoleList {
    /// Every section a permission can be given for.
    catalog: Vec<CatalogEntry>,
    roles: Vec<RoleView>,
}

#[derive(Deserialize)]
pub struct RoleInput {
    name: String,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    own_users_only: bool,
}

async fn view(state: &AppState, role: z_db::Role) -> ApiResult<RoleView> {
    Ok(RoleView {
        admins: state.db().role_admin_count(role.id).await?,
        permissions: Permissions::from_stored(&role.permissions).names(),
        id: role.id,
        name: role.name,
        own_users_only: role.own_users_only,
        created_at: role.created_at,
    })
}

pub async fn list(State(state): State<AppState>, access: Access) -> ApiResult<Json<RoleList>> {
    access.require_sudo()?;
    let mut roles = Vec::new();
    for role in state.db().roles().await? {
        roles.push(view(&state, role).await?);
    }
    Ok(Json(RoleList {
        catalog: Section::ALL
            .into_iter()
            .map(|section| CatalogEntry {
                section: section.name(),
                writable: section.writable(),
            })
            .collect(),
        roles,
    }))
}

/// The name and the stored permissions of a role as sent.
fn checked(input: &RoleInput) -> ApiResult<(String, String)> {
    let name = input.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(ApiError::bad_request(
            "invalid_name",
            "a name of 1 to 64 characters",
        ));
    }
    let permissions = Permissions::parse(input.permissions.iter().map(String::as_str))
        .map_err(|message| ApiError::bad_request("invalid_permission", message))?;
    Ok((name, permissions.to_stored()))
}

fn name_taken(name: &str) -> ApiError {
    ApiError::conflict("name_taken", format!("there is already a role {name:?}"))
}

pub async fn create(
    State(state): State<AppState>,
    access: Access,
    Json(input): Json<RoleInput>,
) -> ApiResult<(StatusCode, Audit, Json<RoleView>)> {
    access.require_sudo()?;
    let (name, permissions) = checked(&input)?;
    let role = state
        .db()
        .create_role(&name, &permissions, input.own_users_only)
        .await
        .map_err(|_| name_taken(&name))?;
    let audit = Audit::new("role.create", &role.name).detail(permissions);
    Ok((StatusCode::CREATED, audit, Json(view(&state, role).await?)))
}

pub async fn update(
    State(state): State<AppState>,
    access: Access,
    Path(id): Path<i64>,
    Json(input): Json<RoleInput>,
) -> ApiResult<(Audit, Json<RoleView>)> {
    access.require_sudo()?;
    let (name, permissions) = checked(&input)?;
    let role = state
        .db()
        .update_role(id, &name, &permissions, input.own_users_only)
        .await
        .map_err(|_| name_taken(&name))?
        .ok_or_else(|| ApiError::not_found("there is no such role"))?;
    let audit = Audit::new("role.update", &role.name).detail(permissions);
    Ok((audit, Json(view(&state, role).await?)))
}

pub async fn delete(
    State(state): State<AppState>,
    access: Access,
    Path(id): Path<i64>,
) -> ApiResult<(StatusCode, Audit, ())> {
    access.require_sudo()?;
    let role = state
        .db()
        .role(id)
        .await?
        .ok_or_else(|| ApiError::not_found("there is no such role"))?;
    if state.db().role_admin_count(id).await? > 0 {
        return Err(ApiError::conflict(
            "role_in_use",
            "admins have this role; give them another first",
        ));
    }
    state.db().delete_role(id).await?;
    Ok((
        StatusCode::NO_CONTENT,
        Audit::new("role.delete", &role.name),
        (),
    ))
}
