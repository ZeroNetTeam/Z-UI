//! Moving here from 3X-UI, PasarGuard or Marzban (sudo only): the old panel's
//! database file is uploaded, read into a preview, and imported when the
//! admin has seen what it holds. An uploaded file is kept in the data
//! directory only until the import is done, or a day.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::audit::Audit;
use crate::auth::Access;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::migrate::apply::{Options, Report};
use crate::migrate::model::{Kind, Summary, Warning};
use crate::migrate::{self, ReadError};
use crate::server::AppState;

/// The most a database file may be.
pub const MAX_UPLOAD: usize = 1 << 30;
/// The most a configuration file of Marzban's may be.
const MAX_CONFIG: usize = 8 << 20;
/// How long an uploaded file is kept.
const KEEP: Duration = Duration::from_secs(24 * 3600);
/// How many warnings are sent at most; the rest are only counted.
pub const MAX_WARNINGS: usize = 300;

fn dir(state: &AppState) -> PathBuf {
    state.config().data_dir.join("migrate")
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn database_path(state: &AppState, id: &str) -> Result<PathBuf, ApiError> {
    let path = dir(state).join(format!("{id}.db"));
    if valid_id(id) && path.is_file() {
        Ok(path)
    } else {
        Err(ApiError::not_found("there is no such upload"))
    }
}

fn config_path(state: &AppState, id: &str) -> PathBuf {
    dir(state).join(format!("{id}.json"))
}

fn forget(state: &AppState, id: &str) {
    if valid_id(id) {
        let _ = std::fs::remove_file(dir(state).join(format!("{id}.db")));
        let _ = std::fs::remove_file(config_path(state, id));
    }
}

/// Files left from days ago are removed: they hold another panel's secrets.
fn sweep(state: &AppState) {
    let Ok(entries) = std::fs::read_dir(dir(state)) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|at| SystemTime::now().duration_since(at).ok())
            .is_some_and(|age| age > KEEP);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[derive(Deserialize)]
pub struct KindQuery {
    kind: Option<String>,
}

fn kind_of(query: &KindQuery) -> Result<Option<Kind>, ApiError> {
    match query
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        None => Ok(None),
        Some(text) => Kind::parse(text).map(Some).ok_or_else(|| {
            ApiError::bad_request("unknown_panel", "kind is 3x-ui, pasarguard or marzban")
        }),
    }
}

fn read_error(error: ReadError) -> ApiError {
    match error {
        ReadError::Open(error) => ApiError::bad_request(
            "not_a_database",
            format!("this is not a database that can be read: {error}"),
        ),
        ReadError::Unknown => ApiError::bad_request(
            "unknown_panel",
            "this does not look like the database of 3X-UI, PasarGuard or Marzban",
        ),
        ReadError::Read(error) => ApiError::bad_request(
            "unreadable_source",
            format!("the database was opened but could not be read through: {error}"),
        ),
    }
}

/// What is in an uploaded database.
#[derive(Serialize, utoipa::ToSchema)]
pub struct Analysis {
    /// Names the upload in the calls that follow.
    id: String,
    kind: Kind,
    summary: Summary,
    /// What would be left out or only approximated, the first few hundred.
    warnings: Vec<Warning>,
    warnings_total: usize,
    /// Marzban keeps its inbounds in a file, not in its database: upload
    /// `xray_config.json` too, or no inbounds come over.
    needs_xray_config: bool,
    bytes: u64,
}

pub fn capped(mut warnings: Vec<Warning>) -> (Vec<Warning>, usize) {
    let total = warnings.len();
    warnings.truncate(MAX_WARNINGS);
    (warnings, total)
}

async fn plan_of(
    state: &AppState,
    id: &str,
    kind: Option<Kind>,
) -> Result<migrate::model::Plan, ApiError> {
    let path = database_path(state, id)?;
    let config: Option<Value> = std::fs::read(config_path(state, id))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    migrate::read(&path.to_string_lossy(), kind, config.as_ref())
        .await
        .map_err(read_error)
}

async fn analyse(state: &AppState, id: &str, kind: Option<Kind>) -> Result<Analysis, ApiError> {
    let plan = plan_of(state, id, kind).await?;
    let bytes = std::fs::metadata(database_path(state, id)?).map_or(0, |meta| meta.len());
    let needs_xray_config =
        plan.kind == Kind::Marzban && plan.warnings.iter().any(|w| w.code == "no_xray_config");
    let (warnings, warnings_total) = capped(plan.warnings.clone());
    Ok(Analysis {
        id: id.into(),
        kind: plan.kind,
        summary: plan.summary(),
        warnings,
        warnings_total,
        needs_xray_config,
        bytes,
    })
}

#[utoipa::path(
    post,
    path = "/migrate",
    operation_id = "migrate_upload",
    tag = "migrate",
    summary = "Upload another panel's database and see what it holds",
    description = "The body is the database file: 3X-UI's `x-ui.db`, PasarGuard's or Marzban's SQLite database. Nothing is imported yet; the answer says what would be.",
    params(("kind" = Option<String>, Query, description = "`3x-ui`, `pasarguard` or `marzban`, when it is not to be found out from the file")),
    request_body(content = String, content_type = "application/octet-stream"),
    responses((status = 201, description = "What the file holds", body = Analysis), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn upload(
    State(state): State<AppState>,
    _: Access,
    Query(query): Query<KindQuery>,
    body: Bytes,
) -> ApiResult<(StatusCode, Audit, Json<Analysis>)> {
    let kind = kind_of(&query)?;
    if body.is_empty() {
        return Err(ApiError::bad_request("empty_upload", "the file is empty"));
    }
    let folder = dir(&state);
    std::fs::create_dir_all(&folder).map_err(ApiError::internal)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700));
    }
    sweep(&state);
    let id = hex::encode(rand::random::<[u8; 16]>());
    let path = folder.join(format!("{id}.db"));
    std::fs::write(&path, &body).map_err(ApiError::internal)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    match analyse(&state, &id, kind).await {
        Ok(analysis) => {
            let audit = Audit::new("migrate.upload", analysis.kind.label()).detail(format!(
                "{} users, {} inbounds",
                analysis.summary.users, analysis.summary.inbounds
            ));
            Ok((StatusCode::CREATED, audit, Json(analysis)))
        }
        Err(error) => {
            forget(&state, &id);
            Err(error)
        }
    }
}

#[utoipa::path(
    get,
    path = "/migrate/{id}",
    operation_id = "migrate_preview",
    tag = "migrate",
    summary = "What an uploaded database would bring",
    params(("id" = String, Path, description = "from the upload"), ("kind" = Option<String>, Query, description = "see the upload")),
    responses((status = 200, description = "What the file holds", body = Analysis), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn preview(
    State(state): State<AppState>,
    _: Access,
    Path(id): Path<String>,
    Query(query): Query<KindQuery>,
) -> ApiResult<Json<Analysis>> {
    Ok(Json(analyse(&state, &id, kind_of(&query)?).await?))
}

#[utoipa::path(
    put,
    path = "/migrate/{id}/xray-config",
    operation_id = "migrate_xray_config",
    tag = "migrate",
    summary = "Give Marzban's inbounds, which it keeps in a file",
    description = "The body is Marzban's `xray_config.json`. PasarGuard keeps its inbounds in its database, and 3X-UI too; this is only for Marzban.",
    params(("id" = String, Path, description = "from the upload"), ("kind" = Option<String>, Query, description = "see the upload")),
    request_body(content = String, content_type = "application/json"),
    responses((status = 200, description = "What the file holds now", body = Analysis), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn xray_config(
    State(state): State<AppState>,
    _: Access,
    Path(id): Path<String>,
    Query(query): Query<KindQuery>,
    body: Bytes,
) -> ApiResult<(Audit, Json<Analysis>)> {
    database_path(&state, &id)?;
    if body.len() > MAX_CONFIG {
        return Err(ApiError::bad_request(
            "too_big",
            "a configuration file is not that big",
        ));
    }
    let config: Value = serde_json::from_slice(&body).map_err(|error| {
        ApiError::bad_request("invalid_config", format!("this is not JSON: {error}"))
    })?;
    if !config["inbounds"].is_array() {
        return Err(ApiError::bad_request(
            "invalid_config",
            "this is not an Xray configuration: it has no `inbounds`",
        ));
    }
    std::fs::write(config_path(&state, &id), &body).map_err(ApiError::internal)?;
    let analysis = analyse(&state, &id, kind_of(&query)?).await?;
    let audit = Audit::new("migrate.config", analysis.kind.label());
    Ok((audit, Json(analysis)))
}

/// What an import came to, with the warnings capped.
#[derive(Serialize, utoipa::ToSchema)]
pub struct Imported {
    #[serde(flatten)]
    report: Report,
    warnings_total: usize,
}

#[utoipa::path(
    post,
    path = "/migrate/{id}/apply",
    operation_id = "migrate_apply",
    tag = "migrate",
    summary = "Import what an uploaded database holds",
    description = "A backup of what the panel holds is made first (`…-before-import`); restoring it undoes the import. The uploaded file is removed afterwards.",
    params(("id" = String, Path, description = "from the upload"), ("kind" = Option<String>, Query, description = "see the upload")),
    request_body = Options,
    responses((status = 200, description = "What was imported", body = Imported), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn apply(
    State(state): State<AppState>,
    _: Access,
    Path(id): Path<String>,
    Query(query): Query<KindQuery>,
    Json(options): Json<Options>,
) -> ApiResult<(Audit, Json<Imported>)> {
    let Ok(_busy) = state.import_lock().try_lock() else {
        return Err(ApiError::conflict(
            "import_running",
            "another import is still running",
        ));
    };
    let plan = plan_of(&state, &id, kind_of(&query)?).await?;
    let kind = plan.kind;
    let mut report = migrate::apply::apply(&state, plan, &options).await?;
    forget(&state, &id);
    let (warnings, warnings_total) = capped(std::mem::take(&mut report.warnings));
    report.warnings = warnings;
    let audit = Audit::new("migrate.apply", kind.label()).detail(format!(
        "{} users, {} inbounds, {} groups; backup {}",
        report.imported.users, report.imported.inbounds, report.imported.groups, report.backup
    ));
    Ok((
        audit,
        Json(Imported {
            report,
            warnings_total,
        }),
    ))
}

#[utoipa::path(
    delete,
    path = "/migrate/{id}",
    operation_id = "migrate_discard",
    tag = "migrate",
    summary = "Throw an uploaded database away",
    params(("id" = String, Path, description = "from the upload")),
    responses((status = 204, description = "Removed"), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn discard(
    State(state): State<AppState>,
    _: Access,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Audit, ())> {
    database_path(&state, &id)?;
    forget(&state, &id);
    Ok((
        StatusCode::NO_CONTENT,
        Audit::new("migrate.discard", id),
        (),
    ))
}
