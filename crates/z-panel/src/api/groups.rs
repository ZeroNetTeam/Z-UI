//! Access groups, which give users their inbounds, and user templates.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::Audit;
use crate::auth::Access;
use crate::core_config;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::jobs::period_seconds;
use crate::server::AppState;

#[derive(Serialize, utoipa::ToSchema)]
pub struct GroupView {
    id: i64,
    name: String,
    all_inbounds: bool,
    inbound_ids: Vec<i64>,
    /// How many users are in it.
    users: usize,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct GroupInput {
    name: String,
    #[serde(default)]
    all_inbounds: bool,
    #[serde(default)]
    inbound_ids: Vec<i64>,
}

#[utoipa::path(
    get,
    path = "/groups",
    operation_id = "groups_list",
    tag = "groups",
    summary = "List access groups",
    responses((status = 200, description = "The groups", body = Vec<GroupView>), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn list(
    State(state): State<AppState>,
    access: Access,
) -> ApiResult<Json<Vec<GroupView>>> {
    let groups = state.db().access_groups().await?;
    // How many users are in a group is counted among the users this admin
    // sees; an admin limited to some groups sees only those.
    let users: Vec<_> = state
        .db()
        .all_users()
        .await?
        .into_iter()
        .filter(|user| access.sees(user))
        .collect();
    Ok(Json(
        groups
            .into_iter()
            .filter(|g| {
                access
                    .group_scope()
                    .is_none_or(|allowed| allowed.contains(&g.id))
            })
            .map(|g| GroupView {
                users: users.iter().filter(|u| u.group_ids.contains(&g.id)).count(),
                id: g.id,
                name: g.name,
                all_inbounds: g.all_inbounds,
                inbound_ids: g.inbound_ids,
            })
            .collect(),
    ))
}

/// Save a group, then rebuild the core's configuration: a group reaches
/// every user in it at once.
async fn save(state: &AppState, id: Option<i64>, input: GroupInput) -> ApiResult<GroupView> {
    let name = input.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(ApiError::bad_request(
            "invalid_name",
            "a name of 1 to 64 characters",
        ));
    }
    let _guard = state.core_lock().lock().await;
    let inbounds = state.db().inbounds().await?;
    let mut inbound_ids = input.inbound_ids;
    inbound_ids.sort_unstable();
    inbound_ids.dedup();
    if inbound_ids
        .iter()
        .any(|id| !inbounds.iter().any(|inbound| inbound.id == *id))
    {
        return Err(ApiError::bad_request(
            "invalid_inbound",
            "there is no such inbound",
        ));
    }
    let saved = state
        .db()
        .save_access_group(id, &name, input.all_inbounds, &inbound_ids)
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::CONFLICT,
                "name_taken",
                format!("there is already a group {name:?}"),
            )
        })?;
    core_config::apply(state, &inbounds).await?;
    let users = state.db().all_users().await?;
    Ok(GroupView {
        users: users
            .iter()
            .filter(|u| u.group_ids.contains(&saved))
            .count(),
        id: saved,
        name,
        all_inbounds: input.all_inbounds,
        inbound_ids,
    })
}

#[utoipa::path(
    post,
    path = "/groups",
    operation_id = "groups_create",
    tag = "groups",
    summary = "Make a group",
    request_body = GroupInput,
    responses((status = 201, description = "Made", body = GroupView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn create(
    State(state): State<AppState>,
    _: Access,
    Json(input): Json<GroupInput>,
) -> ApiResult<(StatusCode, Audit, Json<GroupView>)> {
    let group = save(&state, None, input).await?;
    let audit = Audit::new("group.create", &group.name);
    Ok((StatusCode::CREATED, audit, Json(group)))
}

#[utoipa::path(
    put,
    path = "/groups/{id}",
    operation_id = "groups_update",
    tag = "groups",
    summary = "Change a group",
    params(("id" = i64, Path, description = "the group's id")),
    request_body = GroupInput,
    responses((status = 200, description = "Changed", body = GroupView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn update(
    State(state): State<AppState>,
    _: Access,
    Path(id): Path<i64>,
    Json(input): Json<GroupInput>,
) -> ApiResult<(Audit, Json<GroupView>)> {
    if !state.db().access_groups().await?.iter().any(|g| g.id == id) {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "there is no such group",
        ));
    }
    let group = save(&state, Some(id), input).await?;
    Ok((Audit::new("group.update", &group.name), Json(group)))
}

#[utoipa::path(
    delete,
    path = "/groups/{id}",
    operation_id = "groups_delete",
    tag = "groups",
    summary = "Delete a group",
    params(("id" = i64, Path, description = "the group's id")),
    responses((status = 204, description = "Deleted"), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn delete(
    State(state): State<AppState>,
    _: Access,
    Path(id): Path<i64>,
) -> ApiResult<(StatusCode, Audit, ())> {
    let _guard = state.core_lock().lock().await;
    let name = state
        .db()
        .access_groups()
        .await?
        .into_iter()
        .find(|g| g.id == id)
        .map(|g| g.name);
    if !state.db().delete_access_group(id).await? {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "there is no such group",
        ));
    }
    let inbounds = state.db().inbounds().await?;
    core_config::apply(&state, &inbounds).await?;
    let audit = Audit::new("group.delete", name.unwrap_or_else(|| format!("#{id}")));
    Ok((StatusCode::NO_CONTENT, audit, ()))
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct TemplateView {
    #[serde(default)]
    id: i64,
    name: String,
    #[serde(default)]
    data_limit: i64,
    #[serde(default)]
    expire_days: i64,
    #[serde(default)]
    on_hold: bool,
    #[serde(default = "no_reset")]
    reset_strategy: String,
    #[serde(default)]
    ip_limit: i64,
    #[serde(default)]
    speed_up: i64,
    #[serde(default)]
    speed_down: i64,
    #[serde(default)]
    hwid_limit: i64,
    #[serde(default)]
    group_ids: Vec<i64>,
}

fn no_reset() -> String {
    "no_reset".into()
}

impl From<z_db::UserTemplate> for TemplateView {
    fn from(t: z_db::UserTemplate) -> Self {
        Self {
            id: t.id,
            name: t.name,
            data_limit: t.data_limit,
            expire_days: t.expire_days,
            on_hold: t.on_hold,
            reset_strategy: t.reset_strategy,
            ip_limit: t.ip_limit,
            speed_up: t.speed_up,
            speed_down: t.speed_down,
            hwid_limit: t.hwid_limit,
            group_ids: t.group_ids,
        }
    }
}

#[utoipa::path(
    get,
    path = "/templates",
    operation_id = "groups_templates",
    tag = "templates",
    summary = "List user templates",
    responses((status = 200, description = "The templates", body = Vec<TemplateView>), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn templates(
    State(state): State<AppState>,
    access: Access,
) -> ApiResult<Json<Vec<TemplateView>>> {
    Ok(Json(
        state
            .db()
            .user_templates()
            .await?
            .into_iter()
            .map(TemplateView::from)
            .map(|mut template| {
                // An admin limited to some groups is shown only those.
                if let Some(allowed) = access.group_scope() {
                    template.group_ids.retain(|id| allowed.contains(id));
                }
                template
            })
            .collect(),
    ))
}

async fn save_template(state: &AppState, id: i64, input: TemplateView) -> ApiResult<TemplateView> {
    let name = input.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(ApiError::bad_request(
            "invalid_name",
            "a name of 1 to 64 characters",
        ));
    }
    if input.reset_strategy != "no_reset" && period_seconds(&input.reset_strategy).is_none() {
        return Err(ApiError::bad_request(
            "invalid_reset_strategy",
            "unknown reset period",
        ));
    }
    if [
        input.data_limit,
        input.expire_days,
        input.ip_limit,
        input.speed_up,
        input.speed_down,
        input.hwid_limit,
    ]
    .iter()
    .any(|value| *value < 0)
    {
        return Err(ApiError::bad_request(
            "invalid_limit",
            "limits cannot be negative",
        ));
    }
    let template = z_db::UserTemplate {
        id,
        name: name.clone(),
        data_limit: input.data_limit,
        expire_days: input.expire_days,
        on_hold: input.on_hold,
        reset_strategy: input.reset_strategy,
        ip_limit: input.ip_limit,
        speed_up: input.speed_up,
        speed_down: input.speed_down,
        hwid_limit: input.hwid_limit,
        group_ids: input.group_ids,
        created_at: 0,
    };
    let id = state
        .db()
        .save_user_template(&template)
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::CONFLICT,
                "name_taken",
                format!("there is already a template {name:?}"),
            )
        })?;
    Ok(TemplateView {
        id,
        ..template.into()
    })
}

#[utoipa::path(
    post,
    path = "/templates",
    operation_id = "groups_create_template",
    tag = "templates",
    summary = "Make a template",
    request_body = TemplateView,
    responses((status = 201, description = "Made", body = TemplateView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn create_template(
    State(state): State<AppState>,
    _: Access,
    Json(input): Json<TemplateView>,
) -> ApiResult<(StatusCode, Audit, Json<TemplateView>)> {
    let template = save_template(&state, 0, input).await?;
    let audit = Audit::new("template.create", &template.name);
    Ok((StatusCode::CREATED, audit, Json(template)))
}

#[utoipa::path(
    put,
    path = "/templates/{id}",
    operation_id = "groups_update_template",
    tag = "templates",
    summary = "Change a template",
    params(("id" = i64, Path, description = "the template's id")),
    request_body = TemplateView,
    responses((status = 200, description = "Changed", body = TemplateView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn update_template(
    State(state): State<AppState>,
    _: Access,
    Path(id): Path<i64>,
    Json(input): Json<TemplateView>,
) -> ApiResult<(Audit, Json<TemplateView>)> {
    let template = save_template(&state, id, input).await?;
    Ok((
        Audit::new("template.update", &template.name),
        Json(template),
    ))
}

#[utoipa::path(
    delete,
    path = "/templates/{id}",
    operation_id = "groups_delete_template",
    tag = "templates",
    summary = "Delete a template",
    params(("id" = i64, Path, description = "the template's id")),
    responses((status = 204, description = "Deleted"), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn delete_template(
    State(state): State<AppState>,
    _: Access,
    Path(id): Path<i64>,
) -> ApiResult<(StatusCode, Audit, ())> {
    let name = state
        .db()
        .user_templates()
        .await?
        .into_iter()
        .find(|t| t.id == id)
        .map(|t| t.name);
    if state.db().delete_user_template(id).await? {
        let audit = Audit::new("template.delete", name.unwrap_or_else(|| format!("#{id}")));
        Ok((StatusCode::NO_CONTENT, audit, ()))
    } else {
        Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "there is no such template",
        ))
    }
}
