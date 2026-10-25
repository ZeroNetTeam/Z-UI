//! Signing in and out, the first-run setup, and an admin's own security:
//! password and two-factor sign-in.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::{self, Actor, Audit};
use crate::auth::access::{assemble, Access, Via};
use crate::auth::{client_address, is_secure, password, session, totp};
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::server::AppState;

/// How long a 2FA secret shown for scanning waits for its confirming code.
const PENDING_TOTP: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RoleRef {
    pub id: i64,
    pub name: String,
}

/// What an admin has handed out and may still hand out.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Quota {
    pub user_limit: i64,
    pub data_quota: i64,
    pub users: i64,
    pub allocated: i64,
    pub unlimited: i64,
    pub used: i64,
}

/// What the dashboard may know about the admin using it: who, and what they
/// may do, so it can show only that.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Me {
    pub id: i64,
    pub username: String,
    pub is_sudo: bool,
    pub totp_enabled: bool,
    pub last_login_at: Option<i64>,
    pub role: Option<RoleRef>,
    /// Every `<section>.<level>` they may use (a sudo admin has them all).
    pub permissions: Vec<String>,
    /// They see and change only the users they made.
    pub own_users_only: bool,
    /// The access groups they may give users; empty for every group.
    pub group_ids: Vec<i64>,
    pub quota: Quota,
}

impl Me {
    pub async fn build(state: &AppState, access: &Access) -> ApiResult<Self> {
        let admin = &access.admin;
        let held = state.db().holdings(admin.id).await?;
        Ok(Self {
            id: admin.id,
            username: admin.username.clone(),
            is_sudo: admin.is_sudo,
            totp_enabled: admin.totp_secret.is_some(),
            last_login_at: admin.last_login_at,
            role: access.role.as_ref().map(|role| RoleRef {
                id: role.id,
                name: role.name.clone(),
            }),
            permissions: access.permissions.names(),
            own_users_only: access.own_users_only,
            group_ids: admin.group_ids.clone(),
            quota: Quota {
                user_limit: admin.user_limit,
                data_quota: admin.data_quota,
                users: held.users,
                allocated: held.allocated,
                unlimited: held.unlimited,
                used: held.used,
            },
        })
    }
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct AuthState {
    /// No admin exists yet: the setup page is the way in.
    setup_required: bool,
    admin: Option<Me>,
}

/// Where the dashboard should go: setup, sign-in, or in.
#[utoipa::path(
    get,
    path = "/auth/state",
    operation_id = "auth_state",
    tag = "auth",
    summary = "Where the dashboard should go",
    responses((status = 200, description = "Whether setup is needed, and who is signed in", body = AuthState), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
    security(()),
)]
pub async fn state(State(state): State<AppState>, mut parts: Parts) -> ApiResult<Json<AuthState>> {
    let setup_required = state.db().admin_count().await? == 0;
    let access = Access::from_request_parts(&mut parts, &state).await.ok();
    let admin = match access {
        Some(access) => Some(Me::build(&state, &access).await?),
        None => None,
    };
    Ok(Json(AuthState {
        setup_required,
        admin,
    }))
}

/// A response that signs `admin` in, naming them as who did it.
async fn signed_in(
    state: &AppState,
    parts: &Parts,
    admin: &z_db::Admin,
    action: &'static str,
) -> ApiResult<Response> {
    let token = state
        .session_keys()
        .issue(admin.id, admin.token_version, totp::now());
    let cookie = session::cookie(
        &token,
        &state.config().under_base("/"),
        is_secure(parts, state.tls()),
    );
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        headers.insert(header::SET_COOKIE, value);
    }
    let address = client_address(parts);
    let access = assemble(state, admin.clone(), Via::Session, None, address).await?;
    let me = Me::build(state, &access).await?;
    let mut response = (headers, Json(me)).into_response();
    response
        .extensions_mut()
        .insert(Actor::signing_in(Some(admin), address));
    response
        .extensions_mut()
        .insert(Audit::new(action, &admin.username));
    Ok(response)
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SetupRequest {
    token: String,
    username: String,
    password: String,
}

/// Create the first admin, with the token printed at start. Refused once
/// any admin exists.
#[utoipa::path(
    post,
    path = "/setup",
    operation_id = "auth_setup",
    tag = "auth",
    summary = "Make the first admin",
    request_body = SetupRequest,
    responses((status = 200, description = "Signed in as the new sudo admin", body = Me), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
    security(()),
)]
pub async fn setup(
    State(state): State<AppState>,
    parts: Parts,
    Json(request): Json<SetupRequest>,
) -> ApiResult<Response> {
    if state.db().admin_count().await? > 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "setup_done",
            "the panel already has an admin",
        ));
    }
    let token_ok = state.setup_token().as_deref().is_some_and(|expected| {
        constant_time_eq(expected.as_bytes(), request.token.trim().as_bytes())
    });
    if !token_ok {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "setup_token",
            "the setup link is wrong; use the one printed in the panel's log",
        ));
    }
    let username = request.username.trim();
    password::check_username(username)?;
    password::check_strength(&request.password)?;
    let hash = password::hash(&request.password).await?;
    let admin = state.db().create_admin(username, &hash, true).await?;
    *state.setup_token() = None;
    tracing::info!(username = %admin.username, "first admin created");
    state.db().touch_admin_login(admin.id).await?;
    let admin = refreshed(&state, admin.id).await?;
    signed_in(&state, &parts, &admin, "auth.setup").await
}

/// Who a set-password link is for.
#[derive(Serialize, utoipa::ToSchema)]
pub struct InviteView {
    username: String,
}

/// The admin a link is for, or why it is not good (told the same for a token
/// nobody made, one that ran out and one already used).
async fn invited(
    state: &AppState,
    parts: &Parts,
    token: &str,
    consume: bool,
) -> ApiResult<z_db::Admin> {
    let address = client_address(parts);
    if state
        .key_limiter()
        .locked(address, "invite", Instant::now())
        .is_some()
    {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_attempts",
            "too many wrong links; wait and try again",
        ));
    }
    let hash = crate::auth::invite::hash(token);
    let found = if consume {
        state.db().consume_invite(&hash).await?
    } else {
        state.db().invite(&hash).await?
    };
    found.filter(|admin| admin.enabled).ok_or_else(|| {
        state
            .key_limiter()
            .failed(address, "invite", Instant::now());
        ApiError::new(
            StatusCode::NOT_FOUND,
            "invalid_invite",
            "this link is wrong, has been used, or has run out",
        )
    })
}

#[utoipa::path(
    get,
    path = "/auth/invite/{token}",
    operation_id = "auth_invite",
    tag = "auth",
    summary = "Whose link this is",
    description = "A link that lets an admin brought over from another panel set a password. Good once, for a week.",
    params(("token" = String, Path, description = "the token in the link")),
    responses((status = 200, description = "The admin the link is for", body = InviteView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
    security(()),
)]
pub async fn invite(
    State(state): State<AppState>,
    parts: Parts,
    axum::extract::Path(token): axum::extract::Path<String>,
) -> ApiResult<Json<InviteView>> {
    let admin = invited(&state, &parts, &token, false).await?;
    Ok(Json(InviteView {
        username: admin.username,
    }))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct InviteRequest {
    password: String,
}

#[utoipa::path(
    post,
    path = "/auth/invite/{token}",
    operation_id = "auth_invite_accept",
    tag = "auth",
    summary = "Set a password through a link",
    params(("token" = String, Path, description = "the token in the link")),
    request_body = InviteRequest,
    responses((status = 200, description = "The password is set and the admin is signed in", body = Me), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
    security(()),
)]
pub async fn accept_invite(
    State(state): State<AppState>,
    parts: Parts,
    axum::extract::Path(token): axum::extract::Path<String>,
    Json(request): Json<InviteRequest>,
) -> ApiResult<Response> {
    // A weak password is told before the link is used up.
    password::check_strength(&request.password)?;
    let admin = invited(&state, &parts, &token, true).await?;
    let hash = password::hash(&request.password).await?;
    state.db().set_admin_password(admin.id, &hash).await?;
    tracing::info!(username = %admin.username, "an admin set a password through a link");
    state.db().touch_admin_login(admin.id).await?;
    let admin = refreshed(&state, admin.id).await?;
    signed_in(&state, &parts, &admin, "auth.invite").await
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct LoginRequest {
    username: String,
    password: String,
    /// The authenticator's six digits, when 2FA is on.
    #[serde(default)]
    code: Option<String>,
}

/// Write a failed sign-in in the audit log. A name nobody has is not kept:
/// it may well be a password typed into the wrong box.
async fn failed_sign_in(
    state: &AppState,
    admin: Option<&z_db::Admin>,
    address: IpAddr,
    detail: &str,
) {
    state.notifier().login_failed(
        admin.map(|admin| (admin.id, admin.username.as_str())),
        &address.to_string(),
    );
    let target = admin.map_or("(unknown user)", |admin| admin.username.as_str());
    audit::write(
        state,
        &Actor::signing_in(admin, address),
        "auth.login",
        target,
        detail,
        401,
    )
    .await;
}

#[utoipa::path(
    post,
    path = "/auth/login",
    operation_id = "auth_login",
    tag = "auth",
    summary = "Sign in",
    request_body = LoginRequest,
    responses((status = 200, description = "Signed in: the session cookie is set", body = Me), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
    security(()),
)]
pub async fn login(
    State(state): State<AppState>,
    parts: Parts,
    Json(request): Json<LoginRequest>,
) -> ApiResult<Response> {
    let address = client_address(&parts);
    let username = request.username.trim();
    if let Some(seconds) = state.limiter().locked(address, username, Instant::now()) {
        state
            .notifier()
            .login_locked(&address.to_string(), seconds.div_ceil(60));
        let mut error = ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_attempts",
            "too many failed sign-ins; wait and try again",
        );
        error.retry_after = Some(seconds);
        return Err(error);
    }
    let found = state.db().admin_by_username(username).await?;
    // A missing user costs the same time as a wrong password.
    let hash = match &found {
        Some(admin) => admin.password_hash.as_str(),
        None => password::decoy_hash().await,
    };
    let password_ok = password::verify(&request.password, hash).await;
    let Some(admin) = found.clone().filter(|_| password_ok) else {
        state.limiter().failed(address, username, Instant::now());
        tracing::warn!(%address, %username, "failed sign-in");
        failed_sign_in(&state, found.as_ref(), address, "wrong password").await;
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "wrong username or password",
        ));
    };
    if !admin.enabled {
        failed_sign_in(&state, Some(&admin), address, "account disabled").await;
        return Err(ApiError::forbidden(
            "account_disabled",
            "this account is disabled; ask a sudo admin",
        ));
    }
    if let Some(secret) = &admin.totp_secret {
        let Some(code) = request
            .code
            .as_deref()
            .filter(|code| !code.trim().is_empty())
        else {
            // Right password: ask for the second factor, without a failure.
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "totp_required",
                "enter the code from your authenticator app",
            ));
        };
        if !totp::verify(secret, code, totp::now()) {
            state.limiter().failed(address, username, Instant::now());
            failed_sign_in(&state, Some(&admin), address, "wrong authenticator code").await;
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "totp_invalid",
                "the authenticator code is wrong",
            ));
        }
    }
    state.limiter().succeeded(username);
    // An address this admin has not signed in from before is worth knowing,
    // unless it is their very first sign-in.
    if admin.last_login_at.is_some()
        && !state
            .db()
            .admin_login_seen(admin.id, &address.to_string())
            .await?
    {
        state.notify(crate::notify::Event::NewLogin {
            admin: admin.username.clone(),
            admin_id: admin.id,
            address: address.to_string(),
        });
    }
    state.db().touch_admin_login(admin.id).await?;
    let admin = refreshed(&state, admin.id).await?;
    tracing::info!(%address, username = %admin.username, "signed in");
    signed_in(&state, &parts, &admin, "auth.login").await
}

#[utoipa::path(
    post,
    path = "/auth/logout",
    operation_id = "auth_logout",
    tag = "auth",
    summary = "Sign out",
    responses((status = 204, description = "Signed out"), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
    security(()),
)]
pub async fn logout(State(state): State<AppState>, parts: Parts) -> Response {
    let cookie = session::clear_cookie(
        &state.config().under_base("/"),
        is_secure(&parts, state.tls()),
    );
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        headers.insert(header::SET_COOKIE, value);
    }
    (headers, StatusCode::NO_CONTENT).into_response()
}

#[utoipa::path(
    get,
    path = "/auth/me",
    operation_id = "auth_me",
    tag = "auth",
    summary = "Who am I and what may I do",
    responses((status = 200, description = "The signed-in admin", body = Me), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn me(State(state): State<AppState>, access: Access) -> ApiResult<Json<Me>> {
    Ok(Json(Me::build(&state, &access).await?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct PasswordRequest {
    current: String,
    new: String,
}

/// Change one's own password. Every other session of this admin ends; this
/// one is renewed.
#[utoipa::path(
    post,
    path = "/auth/password",
    operation_id = "auth_change_password",
    tag = "auth",
    summary = "Change my password",
    request_body = PasswordRequest,
    responses((status = 200, description = "Changed; other sessions ended", body = Me), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn change_password(
    State(state): State<AppState>,
    access: Access,
    parts: Parts,
    Json(request): Json<PasswordRequest>,
) -> ApiResult<Response> {
    access.require_session()?;
    let admin = access.admin;
    if !password::verify(&request.current, &admin.password_hash).await {
        return Err(ApiError::bad_request(
            "invalid_credentials",
            "the current password is wrong",
        ));
    }
    password::check_strength(&request.new)?;
    let hash = password::hash(&request.new).await?;
    state.db().set_admin_password(admin.id, &hash).await?;
    let admin = refreshed(&state, admin.id).await?;
    signed_in(&state, &parts, &admin, "auth.password").await
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct TotpSetup {
    secret: String,
    otpauth: String,
    /// The link as a QR code, an SVG document.
    qr_svg: String,
}

/// Make a 2FA secret for this admin to scan. It takes effect only once a
/// code from it is confirmed ([`totp_enable`]).
#[utoipa::path(
    post,
    path = "/auth/2fa/begin",
    operation_id = "auth_totp_begin",
    tag = "auth",
    summary = "Start two-factor setup",
    responses((status = 200, description = "A secret to scan", body = TotpSetup), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn totp_begin(
    State(state): State<AppState>,
    access: Access,
) -> ApiResult<(Audit, Json<TotpSetup>)> {
    access.require_session()?;
    let admin = access.admin;
    if admin.totp_secret.is_some() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "totp_enabled",
            "two-factor sign-in is already on",
        ));
    }
    let secret = totp::new_secret();
    let otpauth = totp::otpauth_url(&secret, z_common::PRODUCT, &admin.username);
    let qr_svg = qrcode::QrCode::new(otpauth.as_bytes())
        .map_err(ApiError::internal)?
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .build();
    state
        .pending_totp()
        .insert(admin.id, (secret.clone(), Instant::now()));
    Ok((
        Audit::new("auth.2fa_begin", &admin.username),
        Json(TotpSetup {
            secret,
            otpauth,
            qr_svg,
        }),
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CodeRequest {
    code: String,
}

/// Turn 2FA on with the secret from [`totp_begin`], proven by a code.
#[utoipa::path(
    post,
    path = "/auth/2fa/enable",
    operation_id = "auth_totp_enable",
    tag = "auth",
    summary = "Turn two-factor sign-in on",
    request_body = CodeRequest,
    responses((status = 200, description = "On", body = Me), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn totp_enable(
    State(state): State<AppState>,
    access: Access,
    parts: Parts,
    Json(request): Json<CodeRequest>,
) -> ApiResult<Response> {
    access.require_session()?;
    let admin = access.admin;
    let pending = state
        .pending_totp()
        .get(&admin.id)
        .filter(|(_, at)| at.elapsed() < PENDING_TOTP)
        .map(|(secret, _)| secret.clone())
        .ok_or_else(|| ApiError::bad_request("totp_expired", "start two-factor setup again"))?;
    if !totp::verify(&pending, &request.code, totp::now()) {
        return Err(ApiError::bad_request(
            "totp_invalid",
            "the authenticator code is wrong",
        ));
    }
    state.db().set_admin_totp(admin.id, Some(&pending)).await?;
    state.pending_totp().remove(&admin.id);
    tracing::info!(username = %admin.username, "two-factor sign-in turned on");
    let admin = refreshed(&state, admin.id).await?;
    signed_in(&state, &parts, &admin, "auth.2fa_enable").await
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct DisableTotpRequest {
    password: String,
    code: String,
}

/// Turn 2FA off: both the password and a current code are needed, so a
/// session left open is not enough.
#[utoipa::path(
    post,
    path = "/auth/2fa/disable",
    operation_id = "auth_totp_disable",
    tag = "auth",
    summary = "Turn two-factor sign-in off",
    request_body = DisableTotpRequest,
    responses((status = 200, description = "Off", body = Me), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn totp_disable(
    State(state): State<AppState>,
    access: Access,
    parts: Parts,
    Json(request): Json<DisableTotpRequest>,
) -> ApiResult<Response> {
    access.require_session()?;
    let admin = access.admin;
    let Some(secret) = &admin.totp_secret else {
        return Err(ApiError::bad_request(
            "totp_disabled",
            "two-factor sign-in is already off",
        ));
    };
    if !password::verify(&request.password, &admin.password_hash).await {
        return Err(ApiError::bad_request(
            "invalid_credentials",
            "the password is wrong",
        ));
    }
    if !totp::verify(secret, &request.code, totp::now()) {
        return Err(ApiError::bad_request(
            "totp_invalid",
            "the authenticator code is wrong",
        ));
    }
    state.db().set_admin_totp(admin.id, None).await?;
    tracing::info!(username = %admin.username, "two-factor sign-in turned off");
    let admin = refreshed(&state, admin.id).await?;
    signed_in(&state, &parts, &admin, "auth.2fa_disable").await
}

async fn refreshed(state: &AppState, id: i64) -> ApiResult<z_db::Admin> {
    state
        .db()
        .admin_by_id(id)
        .await?
        .ok_or_else(ApiError::unauthorized)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::json;

    use crate::auth::totp;
    use crate::server::testing::panel;

    fn login(username: &str, password: &str) -> serde_json::Value {
        json!({ "username": username, "password": password })
    }

    #[tokio::test]
    async fn the_first_admin_needs_the_setup_token_and_only_once() {
        let panel = panel("/").await;
        let state = panel.get("/api/auth/state", None).await.json();
        assert_eq!(state["setup_required"], true);
        assert!(state["admin"].is_null());

        let wrong = panel
            .send(
                "POST",
                "/api/setup",
                None,
                Some(json!({"token": "guess", "username": "owner", "password": "a-long-password"})),
            )
            .await;
        assert_eq!(wrong.status, StatusCode::FORBIDDEN);
        assert_eq!(wrong.json()["error"], "setup_token");

        let weak = panel
            .send(
                "POST",
                "/api/setup",
                None,
                Some(json!({"token": "setup-token", "username": "owner", "password": "short"})),
            )
            .await;
        assert_eq!(weak.json()["error"], "weak_password");

        let session = panel.set_up("").await;
        let state = panel.get("/api/auth/state", Some(&session)).await.json();
        assert_eq!(state["setup_required"], false);
        assert_eq!(state["admin"]["username"], "owner");
        assert_eq!(state["admin"]["is_sudo"], true);

        let again = panel
            .send("POST", "/api/setup", None, Some(json!({"token": "setup-token", "username": "mallory", "password": "a-long-password"})))
            .await;
        assert_eq!(again.status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn sign_in_and_out() {
        let panel = panel("/").await;
        panel.set_up("").await;
        assert_eq!(
            panel.get("/api/auth/me", None).await.status,
            StatusCode::UNAUTHORIZED
        );

        let wrong = panel
            .send(
                "POST",
                "/api/auth/login",
                None,
                Some(login("owner", "nope-nope")),
            )
            .await;
        assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
        assert_eq!(wrong.json()["error"], "invalid_credentials");
        let nobody = panel
            .send(
                "POST",
                "/api/auth/login",
                None,
                Some(login("ghost", "a-long-password")),
            )
            .await;
        assert_eq!(
            nobody.json()["error"],
            "invalid_credentials",
            "no hint that the user is missing"
        );

        let ok = panel
            .send(
                "POST",
                "/api/auth/login",
                None,
                Some(login("owner", "a-long-password")),
            )
            .await;
        assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
        let set_cookie = ok.headers.get("set-cookie").unwrap().to_str().unwrap();
        assert!(set_cookie.contains("HttpOnly") && set_cookie.contains("SameSite=Strict"));
        let session = ok.session().unwrap();
        assert_eq!(
            panel.get("/api/auth/me", Some(&session)).await.json()["username"],
            "owner"
        );

        let out = panel
            .send("POST", "/api/auth/logout", Some(&session), None)
            .await;
        assert_eq!(out.status, StatusCode::NO_CONTENT);
        assert!(out.session().is_none(), "the cookie is cleared");
        assert!(out
            .headers
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Max-Age=0"));
    }

    #[tokio::test]
    async fn repeated_failures_lock_the_account_for_a_while() {
        let panel = panel("/").await;
        panel.set_up("").await;
        for _ in 0..5 {
            panel
                .send(
                    "POST",
                    "/api/auth/login",
                    None,
                    Some(login("owner", "wrong-password")),
                )
                .await;
        }
        let locked = panel
            .send(
                "POST",
                "/api/auth/login",
                None,
                Some(login("owner", "a-long-password")),
            )
            .await;
        assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(locked.json()["error"], "too_many_attempts");
        assert!(locked.headers.contains_key("retry-after"));
    }

    #[tokio::test]
    async fn a_new_password_ends_the_other_sessions() {
        let panel = panel("/").await;
        let first = panel.set_up("").await;
        let second = panel
            .send(
                "POST",
                "/api/auth/login",
                None,
                Some(login("owner", "a-long-password")),
            )
            .await
            .session()
            .unwrap();
        let wrong = panel
            .send(
                "POST",
                "/api/auth/password",
                Some(&first),
                Some(json!({"current": "nope-nope", "new": "another-long-one"})),
            )
            .await;
        assert_eq!(wrong.json()["error"], "invalid_credentials");

        let changed = panel
            .send(
                "POST",
                "/api/auth/password",
                Some(&first),
                Some(json!({"current": "a-long-password", "new": "another-long-one"})),
            )
            .await;
        assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
        let renewed = changed.session().unwrap();
        assert_eq!(
            panel.get("/api/auth/me", Some(&second)).await.status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            panel.get("/api/auth/me", Some(&first)).await.status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            panel.get("/api/auth/me", Some(&renewed)).await.status,
            StatusCode::OK
        );
        let old = panel
            .send(
                "POST",
                "/api/auth/login",
                None,
                Some(login("owner", "a-long-password")),
            )
            .await;
        assert_eq!(old.status, StatusCode::UNAUTHORIZED);
    }

    /// The code an authenticator app would show for `secret` now.
    fn code_for(secret: &str) -> String {
        totp::current_code(secret)
    }

    #[tokio::test]
    async fn two_factor_sign_in_on_and_off() {
        let panel = panel("/").await;
        let session = panel.set_up("").await;
        let setup = panel
            .send("POST", "/api/auth/2fa/begin", Some(&session), None)
            .await
            .json();
        let secret = setup["secret"].as_str().unwrap().to_string();
        assert!(setup["otpauth"]
            .as_str()
            .unwrap()
            .starts_with("otpauth://totp/Z-UI:owner?"));
        assert!(setup["qr_svg"].as_str().unwrap().contains("<svg"));

        let bad = panel
            .send(
                "POST",
                "/api/auth/2fa/enable",
                Some(&session),
                Some(json!({"code": "000000"})),
            )
            .await;
        // (Unless 000000 happens to be the code right now.)
        if code_for(&secret) != "000000" {
            assert_eq!(bad.json()["error"], "totp_invalid");
        }
        let on = panel
            .send(
                "POST",
                "/api/auth/2fa/enable",
                Some(&session),
                Some(json!({"code": code_for(&secret)})),
            )
            .await;
        assert_eq!(on.status, StatusCode::OK, "{}", on.body);
        assert_eq!(on.json()["totp_enabled"], true);
        let session = on.session().unwrap();

        // Now the password alone is not enough.
        let half = panel
            .send(
                "POST",
                "/api/auth/login",
                None,
                Some(login("owner", "a-long-password")),
            )
            .await;
        assert_eq!(half.json()["error"], "totp_required");
        let full = panel
            .send("POST", "/api/auth/login", None, Some(json!({"username": "owner", "password": "a-long-password", "code": code_for(&secret)})))
            .await;
        assert_eq!(full.status, StatusCode::OK, "{}", full.body);

        let off = panel
            .send(
                "POST",
                "/api/auth/2fa/disable",
                Some(&session),
                Some(json!({"password": "a-long-password", "code": code_for(&secret)})),
            )
            .await;
        assert_eq!(off.status, StatusCode::OK, "{}", off.body);
        assert_eq!(off.json()["totp_enabled"], false);
        let plain = panel
            .send(
                "POST",
                "/api/auth/login",
                None,
                Some(login("owner", "a-long-password")),
            )
            .await;
        assert_eq!(plain.status, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_cross_site_request_is_refused() {
        let panel = panel("/").await;
        let session = panel.set_up("").await;
        let request = axum::http::Request::post("/api/auth/logout")
            .header("cookie", &session)
            .header("origin", "https://evil.example")
            .header("host", "panel.example:2053")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = tower::ServiceExt::oneshot(panel.app.clone(), request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let request = axum::http::Request::post("/api/auth/logout")
            .header("cookie", &session)
            .header("origin", "http://panel.example:2053")
            .header("host", "panel.example:2053")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = tower::ServiceExt::oneshot(panel.app.clone(), request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
}
