//! Reading the audit log.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::Access;
use crate::error::ApiResult;
use crate::server::AppState;

#[derive(Deserialize)]
pub struct AuditQuery {
    limit: Option<i64>,
    /// Entries older than this id: the `next` of the page before.
    before: Option<i64>,
    admin: Option<i64>,
    /// Entries whose action starts with this (`user.`).
    action: Option<String>,
    /// Part of the admin, action, target or detail.
    q: Option<String>,
    /// Only requests that failed.
    #[serde(default)]
    failed: bool,
}

#[derive(Serialize)]
pub struct EntryView {
    id: i64,
    at: i64,
    admin_id: Option<i64>,
    admin: String,
    via: String,
    ip: String,
    action: String,
    target: String,
    detail: String,
    status: i64,
}

#[derive(Serialize)]
pub struct AuditPage {
    entries: Vec<EntryView>,
    /// Pass as `before` for the next, older page; none at the end.
    next: Option<i64>,
}

pub async fn list(
    State(state): State<AppState>,
    _: Access,
    Query(query): Query<AuditQuery>,
) -> ApiResult<Json<AuditPage>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let entries = state
        .db()
        .audit(&z_db::AuditQuery {
            admin_id: query.admin,
            action: query.action,
            search: query.q,
            failed_only: query.failed,
            before: query.before,
            limit,
        })
        .await?;
    let next = (entries.len() as i64 == limit)
        .then(|| entries.last().map(|entry| entry.id))
        .flatten();
    Ok(Json(AuditPage {
        entries: entries
            .into_iter()
            .map(|entry| EntryView {
                id: entry.id,
                at: entry.at,
                admin_id: entry.admin_id,
                admin: entry.admin_name,
                via: entry.via,
                ip: entry.ip,
                action: entry.action,
                target: entry.target,
                detail: entry.detail,
                status: entry.status,
            })
            .collect(),
        next,
    }))
}
