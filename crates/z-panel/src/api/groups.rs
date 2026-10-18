//! Access groups, which give users their inbounds, and user templates.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::CurrentAdmin;
use crate::core_config;
use crate::error::{ApiError, ApiResult};
use crate::jobs::period_seconds;
use crate::server::AppState;

#[derive(Serialize)]
pub struct GroupView {
    id: i64,
    name: String,
    all_inbounds: bool,
    inbound_ids: Vec<i64>,
    /// How many users are in it.
    users: usize,
}

#[derive(Deserialize)]
pub struct GroupInput {
    name: String,
    #[serde(default)]
    all_inbounds: bool,
    #[serde(default)]
    inbound_ids: Vec<i64>,
}

pub async fn list(
    State(state): State<AppState>,
    _: CurrentAdmin,
) -> ApiResult<Json<Vec<GroupView>>> {
    let groups = state.db().access_groups().await?;
    let users = state.db().all_users().await?;
    Ok(Json(
        groups
            .into_iter()
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

pub async fn create(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Json(input): Json<GroupInput>,
) -> ApiResult<(StatusCode, Json<GroupView>)> {
    Ok((StatusCode::CREATED, Json(save(&state, None, input).await?)))
}

pub async fn update(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
    Json(input): Json<GroupInput>,
) -> ApiResult<Json<GroupView>> {
    if !state.db().access_groups().await?.iter().any(|g| g.id == id) {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "there is no such group",
        ));
    }
    Ok(Json(save(&state, Some(id), input).await?))
}

pub async fn delete(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let _guard = state.core_lock().lock().await;
    if !state.db().delete_access_group(id).await? {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "there is no such group",
        ));
    }
    let inbounds = state.db().inbounds().await?;
    core_config::apply(&state, &inbounds).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, Deserialize)]
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

pub async fn templates(
    State(state): State<AppState>,
    _: CurrentAdmin,
) -> ApiResult<Json<Vec<TemplateView>>> {
    Ok(Json(
        state
            .db()
            .user_templates()
            .await?
            .into_iter()
            .map(TemplateView::from)
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

pub async fn create_template(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Json(input): Json<TemplateView>,
) -> ApiResult<(StatusCode, Json<TemplateView>)> {
    Ok((
        StatusCode::CREATED,
        Json(save_template(&state, 0, input).await?),
    ))
}

pub async fn update_template(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
    Json(input): Json<TemplateView>,
) -> ApiResult<Json<TemplateView>> {
    Ok(Json(save_template(&state, id, input).await?))
}

pub async fn delete_template(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    if state.db().delete_user_template(id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "there is no such template",
        ))
    }
}
