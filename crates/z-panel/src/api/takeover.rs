//! Replacing another panel on this machine (sudo only): where a takeover
//! has got to, "I made a mistake" (the old panel back), and deleting the old
//! panel's files. The takeover itself is started from the server
//! (`z-ui takeover`, or the installer's `--replace`).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use crate::audit::Audit;
use crate::auth::Access;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::migrate::model::Kind;
use crate::server::AppState;
use crate::takeover::run::{self, Context};
use crate::takeover::state::{self, Phase};

#[derive(Serialize, utoipa::ToSchema)]
pub struct TakeoverView {
    phase: Phase,
    /// The panel that was replaced.
    kind: Kind,
    prepared_at: i64,
    switched_at: Option<i64>,
    /// When the old panel's files are deleted by themselves.
    expires_at: Option<i64>,
    ended_at: Option<i64>,
    /// Users this brought over, and inbounds it switched on.
    users: usize,
    inbounds: usize,
    /// Whether "I made a mistake" can be done: the old panel's files are
    /// still where they were.
    can_rollback: bool,
    /// Whether the old panel's files can be deleted now.
    can_delete: bool,
}

fn view(st: &state::State, root: &std::path::Path) -> TakeoverView {
    let switched = st.phase == Phase::Switched;
    TakeoverView {
        phase: st.phase,
        kind: st.detected.kind,
        prepared_at: st.prepared_at,
        switched_at: st.switched_at,
        expires_at: st.expires_at,
        ended_at: st.ended_at,
        users: st.created_users.len(),
        inbounds: st.enable_tags.len(),
        can_rollback: switched && run::old_panel_present(root, &st.detected),
        can_delete: switched,
    }
}

#[utoipa::path(
    get,
    path = "/takeover",
    operation_id = "takeover_state",
    tag = "takeover",
    summary = "Where the replacing of another panel has got to",
    responses((status = 200, description = "The takeover, or null when there was none", body = Option<TakeoverView>), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn get(State(state): State<AppState>, _: Access) -> Json<Option<TakeoverView>> {
    Json(
        state::load(&state.config().data_dir)
            .as_ref()
            .map(|st| view(st, &state.takeover_root())),
    )
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Started {
    started: bool,
}

#[utoipa::path(
    post,
    path = "/takeover/rollback",
    operation_id = "takeover_rollback",
    tag = "takeover",
    summary = "I made a mistake: put the old panel back",
    description = "This panel is stopped, what its users used here is added to the old panel's own counters, and the old panel is started. It runs on its own after this answers, because it stops this panel. Possible until the old panel's files are deleted (a week after the takeover).",
    responses((status = 202, description = "Started", body = Started), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn rollback(
    State(state): State<AppState>,
    _: Access,
) -> ApiResult<(StatusCode, Audit, Json<Started>)> {
    let data = state.config().data_dir.clone();
    let st = state::load(&data)
        .ok_or_else(|| ApiError::not_found("there is no takeover to go back from"))?;
    if st.phase != Phase::Switched {
        return Err(ApiError::conflict(
            "wrong_phase",
            "this panel has not replaced another, or already went back",
        ));
    }
    if !run::old_panel_present(&state.takeover_root(), &st.detected) {
        return Err(ApiError::conflict(
            "old_panel_gone",
            "the old panel's files are gone, so there is nothing to go back to",
        ));
    }
    let exe = std::env::current_exe().map_err(ApiError::internal)?;
    let data_text = data.to_string_lossy().into_owned();
    state
        .host()
        .spawn_detached(
            &exe.to_string_lossy(),
            &["--data-dir", &data_text, "takeover", "rollback", "--yes"],
        )
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "spawn_failed", e))?;
    Ok((
        StatusCode::ACCEPTED,
        Audit::new("takeover.rollback_start", st.detected.kind.label()),
        Json(Started { started: true }),
    ))
}

#[utoipa::path(
    post,
    path = "/takeover/delete-old",
    operation_id = "takeover_delete_old",
    tag = "takeover",
    summary = "Delete the old panel's files now",
    description = "There is no way back after this. They are deleted by themselves a week after the takeover.",
    responses((status = 200, description = "Done", body = TakeoverView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn delete_old(
    State(state): State<AppState>,
    _: Access,
) -> ApiResult<(Audit, Json<TakeoverView>)> {
    let host = state.host();
    let root = state.takeover_root();
    let cx = Context {
        root: &root,
        host: &*host,
        state: &state,
        say: &|_| {},
    };
    run::delete_old(&cx)
        .await
        .map_err(|e| ApiError::conflict(e.code, e.message))?;
    let st = state::load(&state.config().data_dir)
        .ok_or_else(|| ApiError::not_found("there is no takeover"))?;
    Ok((
        Audit::new("takeover.delete_old_start", st.detected.kind.label()),
        Json(view(&st, &root)),
    ))
}
