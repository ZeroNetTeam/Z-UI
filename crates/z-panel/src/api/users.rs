//! Users: listed, made, changed and removed. Each change reaches the core
//! for that user alone ([`crate::accounts::sync_user`]).

use std::collections::HashSet;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::accounts;
use crate::auth::CurrentAdmin;
use crate::error::{ApiError, ApiResult};
use crate::jobs::{period_seconds, INTERVAL};
use crate::server::AppState;

/// What the dashboard sees of a user.
#[derive(Debug, Serialize)]
pub struct UserView {
    id: i64,
    username: String,
    status: String,
    data_limit: i64,
    used_traffic: i64,
    lifetime_traffic: i64,
    expire_at: Option<i64>,
    on_hold_duration: Option<i64>,
    on_hold_timeout: Option<i64>,
    reset_strategy: String,
    last_reset_at: Option<i64>,
    ip_limit: i64,
    speed_up: i64,
    speed_down: i64,
    note: String,
    online_at: Option<i64>,
    /// Seen connected within the last pass or two.
    online: bool,
    group_ids: Vec<i64>,
    next_plan: Option<NextPlan>,
    hwid_limit: i64,
    /// When and with what app the subscription was last fetched.
    sub_updated_at: Option<i64>,
    sub_user_agent: String,
    uuid: String,
    password: String,
    sub_token: String,
    created_at: i64,
    updated_at: i64,
}

/// What a user moves on to when it runs out of data or time.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NextPlan {
    #[serde(default)]
    data_limit: i64,
    #[serde(default)]
    expire_days: i64,
    #[serde(default)]
    carry_over: bool,
}

impl From<z_db::NextPlan> for NextPlan {
    fn from(p: z_db::NextPlan) -> Self {
        Self {
            data_limit: p.data_limit,
            expire_days: p.expire_days,
            carry_over: p.carry_over,
        }
    }
}

impl From<NextPlan> for z_db::NextPlan {
    fn from(p: NextPlan) -> Self {
        Self {
            data_limit: p.data_limit,
            expire_days: p.expire_days,
            carry_over: p.carry_over,
        }
    }
}

impl From<z_db::User> for UserView {
    fn from(u: z_db::User) -> Self {
        let online = u
            .online_at
            .is_some_and(|at| z_db::now() - at <= 3 * INTERVAL.as_secs() as i64);
        Self {
            id: u.id,
            username: u.username,
            status: u.status,
            data_limit: u.data_limit,
            used_traffic: u.used_traffic,
            lifetime_traffic: u.lifetime_traffic,
            expire_at: u.expire_at,
            on_hold_duration: u.on_hold_duration,
            on_hold_timeout: u.on_hold_timeout,
            reset_strategy: u.reset_strategy,
            last_reset_at: u.last_reset_at,
            ip_limit: u.ip_limit,
            speed_up: u.speed_up,
            speed_down: u.speed_down,
            note: u.note,
            online_at: u.online_at,
            online,
            group_ids: u.group_ids,
            next_plan: u.next_plan.map(NextPlan::from),
            hwid_limit: u.hwid_limit,
            sub_updated_at: u.sub_updated_at,
            sub_user_agent: u.sub_user_agent,
            uuid: u.uuid,
            password: u.password,
            sub_token: u.sub_token,
            created_at: u.created_at,
            updated_at: u.updated_at,
        }
    }
}

/// What the dashboard sends to make or change a user.
#[derive(Debug, Clone, Deserialize)]
pub struct UserInput {
    /// Left out by a bulk creation, which numbers its own.
    #[serde(default)]
    username: String,
    /// `active`, `disabled` or `on_hold`; the panel works out `limited` and
    /// `expired` itself.
    #[serde(default = "active")]
    status: String,
    #[serde(default)]
    data_limit: i64,
    expire_at: Option<i64>,
    on_hold_duration: Option<i64>,
    on_hold_timeout: Option<i64>,
    #[serde(default = "no_reset")]
    reset_strategy: String,
    #[serde(default)]
    ip_limit: i64,
    #[serde(default)]
    speed_up: i64,
    #[serde(default)]
    speed_down: i64,
    #[serde(default)]
    note: String,
    #[serde(default)]
    group_ids: Vec<i64>,
    next_plan: Option<NextPlan>,
    #[serde(default)]
    hwid_limit: i64,
}

fn active() -> String {
    "active".into()
}

fn no_reset() -> String {
    "no_reset".into()
}

fn bad(code: &'static str, message: &str) -> ApiError {
    ApiError::bad_request(code, message)
}

/// A username, which is also the account's name in the core: 3 to 64
/// letters, digits, `.`, `_`, `-` or `@`.
fn check_username(username: &str) -> Result<(), ApiError> {
    let valid = (3..=64).contains(&username.len())
        && username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'@'));
    if valid {
        Ok(())
    } else {
        Err(bad(
            "invalid_username",
            "a username is 3 to 64 letters, digits, '.', '_', '-' or '@'",
        ))
    }
}

async fn fields(state: &AppState, input: UserInput) -> Result<z_db::UserFields, ApiError> {
    let username = input.username.trim().to_string();
    check_username(&username)?;
    if !matches!(input.status.as_str(), "active" | "disabled" | "on_hold") {
        return Err(bad(
            "invalid_status",
            "a status is active, disabled or on_hold",
        ));
    }
    if input.reset_strategy != "no_reset" && period_seconds(&input.reset_strategy).is_none() {
        return Err(bad(
            "invalid_reset_strategy",
            "a reset is no_reset, day, week, month or year",
        ));
    }
    for (value, name) in [
        (input.data_limit, "data_limit"),
        (input.ip_limit, "ip_limit"),
        (input.speed_up, "speed_up"),
        (input.speed_down, "speed_down"),
        (input.hwid_limit, "hwid_limit"),
        (
            input.next_plan.map_or(0, |p| p.data_limit),
            "next_plan.data_limit",
        ),
        (
            input.next_plan.map_or(0, |p| p.expire_days),
            "next_plan.expire_days",
        ),
    ] {
        if value < 0 {
            return Err(ApiError::bad_request(
                "invalid_limit",
                format!("{name} cannot be negative"),
            ));
        }
    }
    let on_hold = input.status == "on_hold";
    if on_hold && input.on_hold_duration.is_none_or(|d| d <= 0) {
        return Err(bad(
            "invalid_on_hold",
            "an on-hold user needs the length of its time",
        ));
    }
    let known: HashSet<i64> = state
        .db()
        .access_groups()
        .await?
        .into_iter()
        .map(|group| group.id)
        .collect();
    if input.group_ids.iter().any(|id| !known.contains(id)) {
        return Err(bad("invalid_group", "there is no such group"));
    }
    let mut group_ids = input.group_ids;
    group_ids.sort_unstable();
    group_ids.dedup();
    Ok(z_db::UserFields {
        username,
        status: input.status,
        data_limit: input.data_limit,
        // An on-hold user's date is set when its time starts.
        expire_at: if on_hold {
            None
        } else {
            input.expire_at.filter(|at| *at > 0)
        },
        on_hold_duration: input.on_hold_duration.filter(|_| on_hold),
        on_hold_timeout: input.on_hold_timeout.filter(|_| on_hold),
        reset_strategy: input.reset_strategy,
        ip_limit: input.ip_limit,
        speed_up: input.speed_up,
        speed_down: input.speed_down,
        note: input.note.chars().take(512).collect(),
        group_ids,
        next_plan: input.next_plan.map(z_db::NextPlan::from),
        hwid_limit: input.hwid_limit,
    })
}

fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not_found", "there is no such user")
}

fn taken(username: &str) -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "username_taken",
        format!("there is already a user {username:?}"),
    )
}

/// Credentials for a new user, or a revoked one.
fn credentials() -> (String, String, String) {
    (
        zero_runtime::keys::uuid(),
        zero_runtime::keys::password(18),
        hex::encode(rand::random::<[u8; 16]>()),
    )
}

/// Reach the core with one user's change. If that fails half-way, rebuild
/// the whole configuration from the database, which is the truth.
async fn reach_core(
    state: &AppState,
    before: Option<&z_db::User>,
    after: Option<&z_db::User>,
) -> Result<(), ApiError> {
    if let Err(error) = accounts::sync_user(state, before, after).await {
        tracing::warn!(error = %error.message, "a user change did not reach the core; rebuilding");
        let inbounds = state.db().inbounds().await?;
        crate::core_config::apply(state, &inbounds).await?;
    }
    Ok(())
}

/// Store the status the counters call for, or move the user on to its
/// next plan (see [`crate::jobs::settle`]).
async fn settle(state: &AppState, user: z_db::User) -> Result<z_db::User, ApiError> {
    let stored = user.status.clone();
    crate::jobs::settle(state.db(), user, &stored, z_db::now()).await
}

#[derive(Deserialize)]
pub struct ListQuery {
    search: Option<String>,
    status: Option<String>,
    group: Option<i64>,
    #[serde(default)]
    offset: i64,
    limit: Option<i64>,
}

#[derive(Serialize)]
pub struct UserPage {
    users: Vec<UserView>,
    total: i64,
}

pub async fn list(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<UserPage>> {
    let (users, total) = state
        .db()
        .users(&z_db::UserQuery {
            search: query.search,
            status: query.status.filter(|s| !s.is_empty()),
            group_id: query.group,
            offset: query.offset,
            limit: query.limit.unwrap_or(25),
        })
        .await?;
    Ok(Json(UserPage {
        users: users.into_iter().map(UserView::from).collect(),
        total,
    }))
}

pub async fn get(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
) -> ApiResult<Json<UserView>> {
    Ok(Json(
        state.db().user(id).await?.ok_or_else(not_found)?.into(),
    ))
}

pub async fn create(
    State(state): State<AppState>,
    CurrentAdmin(admin): CurrentAdmin,
    Json(input): Json<UserInput>,
) -> ApiResult<(StatusCode, Json<UserView>)> {
    let fields = fields(&state, input).await?;
    let _guard = state.core_lock().lock().await;
    let (uuid, password, token) = credentials();
    let user = state
        .db()
        .create_user(&fields, &uuid, &password, &token, Some(admin.id))
        .await
        .map_err(|_| taken(&fields.username))?;
    let user = settle(&state, user).await?;
    reach_core(&state, None, Some(&user)).await?;
    tracing::info!(user = %user.username, "user created");
    Ok((StatusCode::CREATED, Json(user.into())))
}

pub async fn update(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
    Json(input): Json<UserInput>,
) -> ApiResult<Json<UserView>> {
    let mut fields = fields(&state, input).await?;
    let _guard = state.core_lock().lock().await;
    let before = state.db().user(id).await?.ok_or_else(not_found)?;
    // An on-hold user whose time already started keeps its date.
    if fields.status == "on_hold" && before.status != "on_hold" && before.expire_at.is_some() {
        fields.status = "active".into();
        fields.expire_at = before.expire_at;
        fields.on_hold_duration = None;
        fields.on_hold_timeout = None;
    }
    // `limited` and `expired` are the panel's; an admin sends `active`.
    if fields.status == "active" && matches!(before.status.as_str(), "limited" | "expired") {
        fields.status = before.status.clone();
    }
    let after = state
        .db()
        .update_user(id, &fields)
        .await
        .map_err(|_| taken(&fields.username))?
        .ok_or_else(not_found)?;
    let after = settle(&state, after).await?;
    reach_core(&state, Some(&before), Some(&after)).await?;
    Ok(Json(after.into()))
}

pub async fn delete(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let _guard = state.core_lock().lock().await;
    let before = state.db().user(id).await?.ok_or_else(not_found)?;
    state.db().delete_user(id).await?;
    reach_core(&state, Some(&before), None).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Zero the period's traffic; a user limited by it is let back in.
pub async fn reset(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
) -> ApiResult<Json<UserView>> {
    let _guard = state.core_lock().lock().await;
    let before = state.db().user(id).await?.ok_or_else(not_found)?;
    state.db().reset_user_traffic(id).await?;
    let after = state.db().user(id).await?.ok_or_else(not_found)?;
    let after = settle(&state, after).await?;
    reach_core(&state, Some(&before), Some(&after)).await?;
    Ok(Json(after.into()))
}

/// New credentials and subscription link; the old ones stop working and
/// their sessions end.
pub async fn revoke(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
) -> ApiResult<Json<UserView>> {
    let _guard = state.core_lock().lock().await;
    let before = state.db().user(id).await?.ok_or_else(not_found)?;
    let (uuid, password, token) = credentials();
    state
        .db()
        .set_user_credentials(id, &uuid, &password, &token)
        .await?;
    let after = state.db().user(id).await?.ok_or_else(not_found)?;
    reach_core(&state, Some(&before), Some(&after)).await?;
    state.node().end_sessions(&after.username).await;
    Ok(Json(after.into()))
}

#[derive(Serialize)]
pub struct Ended {
    sessions: usize,
}

pub async fn end_sessions(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
) -> ApiResult<Json<Ended>> {
    let user = state.db().user(id).await?.ok_or_else(not_found)?;
    Ok(Json(Ended {
        sessions: state.node().end_sessions(&user.username).await,
    }))
}

#[derive(Serialize)]
pub struct Connected {
    sessions: u64,
    addresses: Vec<String>,
}

/// The sessions a user has open now, and the addresses they come from.
pub async fn online(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
) -> ApiResult<Json<Connected>> {
    let user = state.db().user(id).await?.ok_or_else(not_found)?;
    let account = state
        .node()
        .online()
        .await
        .into_iter()
        .find(|account| account.email == user.username);
    Ok(Json(account.map_or(
        Connected {
            sessions: 0,
            addresses: Vec::new(),
        },
        |account| Connected {
            sessions: account.sessions,
            addresses: account.addresses,
        },
    )))
}

#[derive(Deserialize)]
pub struct UsageQuery {
    days: Option<i64>,
}

#[derive(Serialize)]
pub struct UsagePoint {
    hour: i64,
    uplink: i64,
    downlink: i64,
}

pub async fn usage(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
    Query(query): Query<UsageQuery>,
) -> ApiResult<Json<Vec<UsagePoint>>> {
    let days = query.days.unwrap_or(7).clamp(1, 366);
    let since = z_db::now() - days * 86_400;
    let points = state.db().user_usage(id, since).await?;
    Ok(Json(
        points
            .into_iter()
            .map(|p| UsagePoint {
                hour: p.hour,
                uplink: p.uplink,
                downlink: p.downlink,
            })
            .collect(),
    ))
}

/// Users by status, with `total` and `online` beside them.
pub async fn counts(
    State(state): State<AppState>,
    _: CurrentAdmin,
) -> ApiResult<Json<std::collections::HashMap<String, i64>>> {
    let mut counts = state.db().user_counts().await?;
    counts.insert("total".into(), counts.values().sum());
    let users = state.db().all_users().await?;
    let online = users
        .into_iter()
        .filter(|user| UserView::from(user.clone()).online)
        .count();
    counts.insert("online".into(), online as i64);
    Ok(Json(counts))
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BulkAction {
    Enable,
    Disable,
    Delete,
    Reset,
    Revoke,
    /// Push the expiry `days` later (from now for a user without one); an
    /// on-hold user gets `days` more once it starts.
    Extend {
        days: i64,
    },
    /// Raise the data limit by `bytes` (a user without a limit keeps none).
    AddData {
        bytes: i64,
    },
    /// Put the users in exactly these groups.
    SetGroups {
        group_ids: Vec<i64>,
    },
}

#[derive(Deserialize)]
pub struct BulkRequest {
    ids: Vec<i64>,
    #[serde(flatten)]
    action: BulkAction,
}

#[derive(Serialize)]
pub struct BulkResult {
    changed: usize,
}

/// One action on many users.
pub async fn bulk(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Json(request): Json<BulkRequest>,
) -> ApiResult<Json<BulkResult>> {
    if request.ids.len() > 10_000 {
        return Err(bad("too_many", "at most 10000 users at once"));
    }
    if let BulkAction::SetGroups { group_ids } = &request.action {
        let known = state.db().access_groups().await?;
        if group_ids
            .iter()
            .any(|id| !known.iter().any(|g| g.id == *id))
        {
            return Err(bad("invalid_group", "there is no such group"));
        }
    }
    let _guard = state.core_lock().lock().await;
    let db = state.db();
    let mut changed = 0;
    for id in request.ids {
        let Some(before) = db.user(id).await? else {
            continue;
        };
        let mut fields = z_db::UserFields {
            username: before.username.clone(),
            status: before.status.clone(),
            data_limit: before.data_limit,
            expire_at: before.expire_at,
            on_hold_duration: before.on_hold_duration,
            on_hold_timeout: before.on_hold_timeout,
            reset_strategy: before.reset_strategy.clone(),
            ip_limit: before.ip_limit,
            speed_up: before.speed_up,
            speed_down: before.speed_down,
            note: before.note.clone(),
            group_ids: before.group_ids.clone(),
            next_plan: before.next_plan,
            hwid_limit: before.hwid_limit,
        };
        match &request.action {
            BulkAction::Delete => {
                db.delete_user(id).await?;
                reach_core(&state, Some(&before), None).await?;
                changed += 1;
                continue;
            }
            BulkAction::Reset => db.reset_user_traffic(id).await?,
            BulkAction::Revoke => {
                let (uuid, password, token) = credentials();
                db.set_user_credentials(id, &uuid, &password, &token)
                    .await?;
            }
            BulkAction::Enable => {
                if before.status == "disabled" {
                    fields.status = "active".into();
                }
            }
            BulkAction::Disable => fields.status = "disabled".into(),
            // An on-hold user's time has not started: it gets more of it.
            BulkAction::Extend { days } if before.status == "on_hold" => {
                fields.on_hold_duration =
                    Some(before.on_hold_duration.unwrap_or(0) + days * 86_400);
            }
            BulkAction::Extend { days } => {
                let base = before
                    .expire_at
                    .filter(|at| *at > z_db::now())
                    .unwrap_or_else(z_db::now);
                fields.expire_at = Some(base + days * 86_400);
                if fields.status == "expired" {
                    fields.status = "active".into();
                }
            }
            BulkAction::SetGroups { group_ids } => {
                fields.group_ids = group_ids.clone();
                fields.group_ids.sort_unstable();
                fields.group_ids.dedup();
            }
            BulkAction::AddData { bytes } => {
                if fields.data_limit > 0 {
                    fields.data_limit = (fields.data_limit + bytes).max(0);
                }
                if fields.status == "limited" {
                    fields.status = "active".into();
                }
            }
        }
        if !matches!(request.action, BulkAction::Reset | BulkAction::Revoke) {
            db.update_user(id, &fields).await?;
        }
        let after = db.user(id).await?.ok_or_else(not_found)?;
        let after = settle(&state, after).await?;
        reach_core(&state, Some(&before), Some(&after)).await?;
        if matches!(request.action, BulkAction::Revoke) {
            state.node().end_sessions(&after.username).await;
        }
        changed += 1;
    }
    Ok(Json(BulkResult { changed }))
}

#[derive(Deserialize)]
pub struct BulkCreate {
    /// Usernames are `prefix` followed by a number from `start`.
    prefix: String,
    #[serde(default = "one")]
    start: i64,
    count: i64,
    #[serde(flatten)]
    user: UserInput,
}

fn one() -> i64 {
    1
}

/// Many users alike, `prefix1`, `prefix2`, … Names already taken are skipped.
pub async fn bulk_create(
    State(state): State<AppState>,
    CurrentAdmin(admin): CurrentAdmin,
    Json(request): Json<BulkCreate>,
) -> ApiResult<(StatusCode, Json<BulkResult>)> {
    if !(1..=1000).contains(&request.count) {
        return Err(bad("too_many", "1 to 1000 users at once"));
    }
    let mut changed = 0;
    for n in request.start..request.start + request.count {
        let mut input = request.user.clone();
        input.username = format!("{}{n}", request.prefix.trim());
        let fields = fields(&state, input).await?;
        let _guard = state.core_lock().lock().await;
        let (uuid, password, token) = credentials();
        let Ok(user) = state
            .db()
            .create_user(&fields, &uuid, &password, &token, Some(admin.id))
            .await
        else {
            continue;
        };
        let user = settle(&state, user).await?;
        reach_core(&state, None, Some(&user)).await?;
        changed += 1;
    }
    Ok((StatusCode::CREATED, Json(BulkResult { changed })))
}

#[cfg(test)]
mod tests {

    use axum::http::StatusCode;
    use serde_json::{json, Value};
    use tokio::net::TcpStream;

    use crate::server::testing::{client, connect, echo_sink, ended, free_port, panel, Panel};

    /// A panel with one plain VLESS inbound, and its port.
    async fn with_inbound() -> (Panel, String, u16) {
        let panel = panel("/").await;
        let session = panel.set_up("").await;
        // The clients here reach a sink on 127.0.0.1.
        panel
            .state
            .db()
            .set_setting(
                crate::core_settings::SETTING,
                &serde_json::json!({
                    "blockPrivate": false,
                    "outbounds": [{"tag": "direct", "protocol": "freedom"}],
                })
                .to_string(),
            )
            .await
            .unwrap();
        let port = free_port();
        let inbound = panel
            .send("POST", "/api/inbounds", Some(&session), Some(json!({
                "remark": "plain", "listen": "127.0.0.1", "port": port, "protocol": "vless",
                "settings": {"decryption": "none"}, "stream": {"network": "raw", "security": "none"},
            })))
            .await;
        assert_eq!(inbound.status, StatusCode::CREATED, "{}", inbound.body);
        (panel, session, port)
    }

    async fn user(panel: &Panel, session: &str, body: Value) -> Value {
        let created = panel
            .send("POST", "/api/users", Some(session), Some(body))
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        created.json()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_user_connects_is_counted_and_is_held_to_its_limits() {
        let (panel, session, port) = with_inbound().await;
        let groups = panel.get("/api/groups", Some(&session)).await.json();
        let default = groups[0]["id"].clone();
        let alice = user(
            &panel,
            &session,
            json!({"username": "alice", "group_ids": [default]}),
        )
        .await;
        assert_eq!(alice["status"], "active");
        let id = alice["id"].as_i64().unwrap();
        let sink = echo_sink().await;
        let socks = client(port, alice["uuid"].as_str().unwrap()).await;

        let payload = vec![7u8; 100_000];
        let mut open = connected(socks, sink, &payload).await;
        crate::jobs::enforce_once(&panel.state).await.unwrap();
        let counted = panel
            .get(&format!("/api/users/{id}"), Some(&session))
            .await
            .json();
        assert!(
            counted["used_traffic"].as_i64().unwrap() >= 200_000,
            "{counted}"
        );
        assert_eq!(counted["online"], true);

        // A limit below what it used: limited at once, and cut off.
        let mut edit = json!({"username": "alice", "group_ids": [default], "data_limit": 1000});
        let limited = panel
            .send(
                "PUT",
                &format!("/api/users/{id}"),
                Some(&session),
                Some(edit.clone()),
            )
            .await;
        assert_eq!(limited.json()["status"], "limited", "{}", limited.body);
        assert!(ended(&mut open).await, "the limited user kept its session");
        assert!(
            connect(socks, sink, b"ping").await.is_none(),
            "a limited user connected"
        );

        // A reset lets it back in.
        let reset = panel
            .send(
                "POST",
                &format!("/api/users/{id}/reset"),
                Some(&session),
                None,
            )
            .await;
        assert_eq!(reset.json()["status"], "active", "{}", reset.body);
        assert!(
            connect(socks, sink, b"ping").await.is_some(),
            "the reset user is refused"
        );

        // The enforcement pass catches it going over again.
        let _again = connected(socks, sink, &payload).await;
        crate::jobs::enforce_once(&panel.state).await.unwrap();
        let over = panel
            .get(&format!("/api/users/{id}"), Some(&session))
            .await
            .json();
        assert_eq!(over["status"], "limited");

        // Disabled, and back.
        edit["data_limit"] = json!(0);
        panel
            .send(
                "PUT",
                &format!("/api/users/{id}"),
                Some(&session),
                Some(edit),
            )
            .await;
        let off = panel
            .send(
                "POST",
                "/api/users/bulk",
                Some(&session),
                Some(json!({"action": "disable", "ids": [id]})),
            )
            .await;
        assert_eq!(off.json()["changed"], 1, "{}", off.body);
        assert!(
            connect(socks, sink, b"ping").await.is_none(),
            "a disabled user connected"
        );
        panel
            .send(
                "POST",
                "/api/users/bulk",
                Some(&session),
                Some(json!({"action": "enable", "ids": [id]})),
            )
            .await;
        assert!(connect(socks, sink, b"ping").await.is_some());

        // Past its date.
        let past = panel
            .send(
                "PUT",
                &format!("/api/users/{id}"),
                Some(&session),
                Some(json!({
                    "username": "alice", "group_ids": [default], "expire_at": z_db::now() - 1,
                })),
            )
            .await;
        assert_eq!(past.json()["status"], "expired", "{}", past.body);
        assert!(
            connect(socks, sink, b"ping").await.is_none(),
            "an expired user connected"
        );

        let usage = panel
            .get(&format!("/api/users/{id}/usage"), Some(&session))
            .await
            .json();
        assert!(usage[0]["downlink"].as_i64().unwrap() > 0, "{usage}");
    }

    async fn connected(socks: u16, sink: u16, payload: &[u8]) -> TcpStream {
        connect(socks, sink, payload)
            .await
            .expect("the user connects")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_on_hold_user_starts_its_time_on_the_first_connection() {
        let (panel, session, port) = with_inbound().await;
        let default = panel.get("/api/groups", Some(&session)).await.json()[0]["id"].clone();
        let held = user(&panel, &session, json!({
            "username": "held", "status": "on_hold", "on_hold_duration": 3600, "group_ids": [default],
            "expire_at": 1,
        }))
        .await;
        assert_eq!(held["status"], "on_hold");
        assert!(held["expire_at"].is_null(), "no date until it starts");
        let extended = panel
            .send(
                "POST",
                "/api/users/bulk",
                Some(&session),
                Some(json!({"action": "extend", "days": 1, "ids": [held["id"]]})),
            )
            .await;
        assert_eq!(extended.json()["changed"], 1, "{}", extended.body);
        let held = panel
            .get(&format!("/api/users/{}", held["id"]), Some(&session))
            .await
            .json();
        assert!(held["expire_at"].is_null(), "still no date: {held}");
        assert_eq!(held["on_hold_duration"], 3600 + 86_400);
        let sink = echo_sink().await;
        let socks = client(port, held["uuid"].as_str().unwrap()).await;
        let _open = connected(socks, sink, b"first").await;
        crate::jobs::enforce_once(&panel.state).await.unwrap();
        let started = panel
            .get(&format!("/api/users/{}", held["id"]), Some(&session))
            .await
            .json();
        assert_eq!(started["status"], "active", "{started}");
        let expire = started["expire_at"].as_i64().unwrap();
        assert!(
            (expire - z_db::now() - 3600 - 86_400).abs() < 30,
            "{started}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_user_out_of_data_moves_on_to_its_next_plan() {
        let (panel, session, port) = with_inbound().await;
        let default = panel.get("/api/groups", Some(&session)).await.json()[0]["id"].clone();
        let frank = user(
            &panel,
            &session,
            json!({
                "username": "frank", "group_ids": [default], "data_limit": 150_000,
                "next_plan": {"data_limit": 10_000_000, "expire_days": 30, "carry_over": false},
            }),
        )
        .await;
        assert_eq!(frank["next_plan"]["expire_days"], 30);
        let id = frank["id"].as_i64().unwrap();
        let sink = echo_sink().await;
        let socks = client(port, frank["uuid"].as_str().unwrap()).await;
        let _open = connected(socks, sink, &vec![1u8; 100_000]).await;

        let online = panel
            .get(&format!("/api/users/{id}/online"), Some(&session))
            .await
            .json();
        assert_eq!(online["addresses"], json!(["127.0.0.1"]), "{online}");

        crate::jobs::enforce_once(&panel.state).await.unwrap();
        let moved = panel
            .get(&format!("/api/users/{id}"), Some(&session))
            .await
            .json();
        assert_eq!(moved["status"], "active", "{moved}");
        assert_eq!(moved["data_limit"], 10_000_000);
        assert_eq!(moved["used_traffic"], 0);
        assert!(moved["next_plan"].is_null());
        let expire = moved["expire_at"].as_i64().unwrap();
        assert!((expire - z_db::now() - 30 * 86_400).abs() < 30);
        assert!(connect(socks, sink, b"still in").await.is_some());

        // Without a plan left, the next time it runs out it is limited.
        let lowered = panel
            .send(
                "PUT",
                &format!("/api/users/{id}"),
                Some(&session),
                Some(json!({
                    "username": "frank", "group_ids": [default], "data_limit": 1,
                    "expire_at": expire,
                })),
            )
            .await;
        assert_eq!(
            lowered.json()["status"],
            "active",
            "nothing counted since the switch"
        );
        let _more = connected(socks, sink, b"a few more bytes").await;
        crate::jobs::enforce_once(&panel.state).await.unwrap();
        let now = panel
            .get(&format!("/api/users/{id}"), Some(&session))
            .await
            .json();
        assert_eq!(now["status"], "limited", "{now}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn users_are_checked_listed_and_revoked() {
        let (panel, session, port) = with_inbound().await;
        let default = panel.get("/api/groups", Some(&session)).await.json()[0]["id"].clone();
        for (body, code) in [
            (json!({"username": "a b"}), "invalid_username"),
            (
                json!({"username": "bob", "status": "limited"}),
                "invalid_status",
            ),
            (
                json!({"username": "bob", "reset_strategy": "hourly"}),
                "invalid_reset_strategy",
            ),
            (
                json!({"username": "bob", "data_limit": -1}),
                "invalid_limit",
            ),
            (
                json!({"username": "bob", "status": "on_hold"}),
                "invalid_on_hold",
            ),
            (
                json!({"username": "bob", "group_ids": [999]}),
                "invalid_group",
            ),
        ] {
            let refused = panel
                .send("POST", "/api/users", Some(&session), Some(body))
                .await;
            assert_eq!(refused.json()["error"], code, "{}", refused.body);
        }
        let bob = user(
            &panel,
            &session,
            json!({"username": "bob", "group_ids": [default], "note": "office"}),
        )
        .await;
        let twin = panel
            .send(
                "POST",
                "/api/users",
                Some(&session),
                Some(json!({"username": "bob"})),
            )
            .await;
        assert_eq!(twin.status, StatusCode::CONFLICT);

        let made = panel
            .send("POST", "/api/users/bulk-create", Some(&session), Some(json!({
                "prefix": "guest", "count": 3, "group_ids": [default], "data_limit": 1_000_000_000,
            })))
            .await;
        assert_eq!(made.json()["changed"], 3, "{}", made.body);
        let page = panel
            .get("/api/users?search=guest&limit=2", Some(&session))
            .await
            .json();
        assert_eq!(page["total"], 3);
        assert_eq!(page["users"].as_array().unwrap().len(), 2);
        assert_eq!(
            panel
                .get("/api/users?search=OFFICE", Some(&session))
                .await
                .json()["total"],
            1
        );
        assert_eq!(
            panel.get("/api/users/counts", Some(&session)).await.json()["active"],
            4
        );

        // Revoked: the old credential no longer connects, the new one does.
        let sink = echo_sink().await;
        let old = client(port, bob["uuid"].as_str().unwrap()).await;
        let mut open = connected(old, sink, b"hi").await;
        let revoked = panel
            .send(
                "POST",
                &format!("/api/users/{}/revoke", bob["id"]),
                Some(&session),
                None,
            )
            .await
            .json();
        assert_ne!(revoked["uuid"], bob["uuid"]);
        assert_ne!(revoked["sub_token"], bob["sub_token"]);
        assert!(ended(&mut open).await);
        assert!(connect(old, sink, b"hi").await.is_none());
        let new = client(port, revoked["uuid"].as_str().unwrap()).await;
        assert!(connect(new, sink, b"hi").await.is_some());

        let gone = panel
            .send(
                "DELETE",
                &format!("/api/users/{}", bob["id"]),
                Some(&session),
                None,
            )
            .await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT);
        assert!(
            connect(new, sink, b"hi").await.is_none(),
            "a deleted user connected"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_group_decides_who_reaches_an_inbound() {
        let (panel, session, port) = with_inbound().await;
        let inbound_id = panel.get("/api/inbounds", Some(&session)).await.json()[0]["id"].clone();
        let empty = panel
            .send(
                "POST",
                "/api/groups",
                Some(&session),
                Some(json!({"name": "none", "inbound_ids": []})),
            )
            .await
            .json();
        let carol = user(
            &panel,
            &session,
            json!({"username": "carol", "group_ids": [empty["id"]]}),
        )
        .await;
        let sink = echo_sink().await;
        let socks = client(port, carol["uuid"].as_str().unwrap()).await;
        assert!(
            connect(socks, sink, b"x").await.is_none(),
            "no group gives this inbound"
        );
        let given = panel
            .send(
                "PUT",
                &format!("/api/groups/{}", empty["id"]),
                Some(&session),
                Some(json!({"name": "none", "inbound_ids": [inbound_id]})),
            )
            .await;
        assert_eq!(given.status, StatusCode::OK, "{}", given.body);
        assert!(
            connect(socks, sink, b"x").await.is_some(),
            "the group now gives it"
        );

        let other = panel
            .send(
                "POST",
                "/api/groups",
                Some(&session),
                Some(json!({"name": "other", "inbound_ids": []})),
            )
            .await
            .json();
        let moved = panel
            .send(
                "POST",
                "/api/users/bulk",
                Some(&session),
                Some(json!({"action": "set_groups", "group_ids": [other["id"]], "ids": [carol["id"]]})),
            )
            .await;
        assert_eq!(moved.json()["changed"], 1, "{}", moved.body);
        assert!(
            connect(socks, sink, b"x").await.is_none(),
            "moved out of the group"
        );
        let groups = panel.get("/api/groups", Some(&session)).await.json();
        let count = |name: &str| {
            groups
                .as_array()
                .unwrap()
                .iter()
                .find(|g| g["name"] == name)
                .unwrap()["users"]
                .clone()
        };
        assert_eq!((count("none"), count("other")), (json!(0), json!(1)));

        let template = panel
            .send("POST", "/api/templates", Some(&session), Some(json!({"name": "monthly", "data_limit": 5, "expire_days": 30, "reset_strategy": "month"})))
            .await;
        assert_eq!(template.status, StatusCode::CREATED, "{}", template.body);
        assert_eq!(
            panel.get("/api/templates", Some(&session)).await.json()[0]["name"],
            "monthly"
        );
    }
}
