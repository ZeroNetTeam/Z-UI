//! Backups: made, kept, downloaded and put back. Only a sudo admin: a backup
//! holds every secret the panel has.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::Audit;
use crate::auth::Access;
use crate::backup::{self, FileInfo};
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::notify::settings::BackupView;
use crate::server::AppState;

/// The most a backup file may be to be put back.
pub const MAX_UPLOAD: usize = 1 << 30;

#[derive(Serialize, utoipa::ToSchema)]
pub struct BackupsView {
    /// The backups kept in the data directory, newest first.
    files: Vec<FileInfo>,
    settings: BackupView,
    /// When the last one was made.
    last_at: Option<i64>,
}

async fn view(state: &AppState) -> BackupsView {
    BackupsView {
        files: backup::list(state),
        settings: state.notify_settings().view().backup,
        last_at: backup::last_at(state).await,
    }
}

#[utoipa::path(
    get,
    path = "/backups",
    operation_id = "backups_list",
    tag = "backups",
    summary = "The backups kept, and when they are made",
    responses((status = 200, description = "The files and the schedule", body = BackupsView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn list(State(state): State<AppState>, _: Access) -> Json<BackupsView> {
    Json(view(&state).await)
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Made {
    file: String,
    bytes: u64,
    /// Telegram chats it was sent to.
    sent: u32,
    encrypted: bool,
}

#[utoipa::path(
    post,
    path = "/backups",
    operation_id = "backups_make",
    tag = "backups",
    summary = "Make a backup now",
    responses((status = 201, description = "Made, kept, and sent as the settings say", body = Made), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn make(
    State(state): State<AppState>,
    _: Access,
) -> ApiResult<(StatusCode, Audit, Json<Made>)> {
    let done = backup::run(&state, None)
        .await
        .map_err(|message| ApiError::bad_request("backup_failed", message))?;
    let audit = Audit::new("backup.make", &done.file).detail(format!("{} bytes", done.bytes));
    Ok((
        StatusCode::CREATED,
        audit,
        Json(Made {
            file: done.file,
            bytes: done.bytes,
            sent: done.sent,
            encrypted: done.encrypted,
        }),
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct BackupInput {
    enabled: bool,
    /// Make one every this many hours.
    interval_hours: u32,
    /// Keep this many in the data directory.
    keep: u32,
    send_telegram: bool,
    /// Include the hourly usage (large).
    include_usage: bool,
    /// A passphrase to seal them with; empty keeps the one there is.
    #[serde(default)]
    passphrase: String,
    /// Stop sealing them.
    #[serde(default)]
    clear_passphrase: bool,
}

#[utoipa::path(
    put,
    path = "/backups/settings",
    operation_id = "backups_settings",
    tag = "backups",
    summary = "Change when and how backups are made",
    request_body = BackupInput,
    responses((status = 200, description = "Saved", body = BackupsView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn settings(
    State(state): State<AppState>,
    _: Access,
    Json(input): Json<BackupInput>,
) -> ApiResult<(Audit, Json<BackupsView>)> {
    let mut settings = state.notify_settings();
    let passphrase = if input.clear_passphrase {
        String::new()
    } else if input.passphrase.trim().is_empty() {
        settings.backup.passphrase.clone()
    } else {
        input.passphrase.trim().to_string()
    };
    settings.backup = crate::notify::settings::Backup {
        enabled: input.enabled,
        interval_hours: input.interval_hours,
        keep: input.keep,
        send_telegram: input.send_telegram,
        include_usage: input.include_usage,
        passphrase,
    };
    let settings = settings
        .checked()
        .map_err(|message| ApiError::bad_request("invalid_notify", message))?;
    crate::notify::save(state.db(), &settings).await?;
    state.set_notify_settings(settings);
    Ok((Audit::new("backup.settings", ""), Json(view(&state).await)))
}

#[utoipa::path(
    get,
    path = "/backups/{name}",
    operation_id = "backups_download",
    tag = "backups",
    summary = "Download a backup file",
    params(("name" = String, Path, description = "the file's name")),
    responses((status = 200, description = "The file", body = String, content_type = "application/octet-stream"), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn download(
    State(state): State<AppState>,
    _: Access,
    Path(name): Path<String>,
) -> ApiResult<Response> {
    let bytes = backup::read(&state, &name)
        .ok_or_else(|| ApiError::not_found("there is no such backup"))?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        bytes,
    )
        .into_response())
}

#[utoipa::path(
    delete,
    path = "/backups/{name}",
    operation_id = "backups_delete",
    tag = "backups",
    summary = "Delete a backup file",
    params(("name" = String, Path, description = "the file's name")),
    responses((status = 204, description = "Deleted"), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn delete(
    State(state): State<AppState>,
    _: Access,
    Path(name): Path<String>,
) -> ApiResult<(StatusCode, Audit, ())> {
    if !backup::delete(&state, &name) {
        return Err(ApiError::not_found("there is no such backup"));
    }
    Ok((
        StatusCode::NO_CONTENT,
        Audit::new("backup.delete", &name),
        (),
    ))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct RestoredTable {
    name: String,
    rows: u64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Restored {
    /// Rows put back, by table.
    tables: Vec<RestoredTable>,
    /// The database version of the panel that made the backup.
    schema: i64,
    panel_version: String,
    /// When the backup was made.
    created_at: i64,
    /// A backup of what the panel held just before, to go back to.
    safety: String,
    /// Something that did not go quite right afterwards.
    warning: Option<String>,
}

#[utoipa::path(
    post,
    path = "/backups/restore",
    operation_id = "backups_restore",
    tag = "backups",
    summary = "Put a backup back, replacing everything the panel holds",
    description = "The body is the backup file. Everyone is signed out afterwards; the admins are those of the backup. What the panel held before is first kept as a file of its own.",
    params(("X-Backup-Passphrase" = Option<String>, Header, description = "the passphrase, for a sealed backup")),
    request_body(content = String, content_type = "application/octet-stream"),
    responses((status = 200, description = "Put back", body = Restored), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn restore(
    State(state): State<AppState>,
    _: Access,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(Audit, Json<Restored>)> {
    let passphrase = headers
        .get("x-backup-passphrase")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let done = backup::restore(&state, &body, passphrase)
        .await
        .map_err(|refused| ApiError::bad_request(refused.code, refused.message))?;
    let audit = Audit::new("backup.restore", &done.safety).detail(format!(
        "made {}",
        crate::notify::events::date(done.report.created_at)
    ));
    Ok((
        audit,
        Json(Restored {
            tables: done
                .report
                .tables
                .into_iter()
                .map(|(name, rows)| RestoredTable { name, rows })
                .collect(),
            schema: done.report.schema,
            panel_version: done.report.panel_version,
            created_at: done.report.created_at,
            safety: done.safety,
            warning: done.warning,
        }),
    ))
}
