//! Who a request is, and what they may do with it: a signed-in session or an
//! API key, the admin behind it, and the permissions that leaves them.

use std::net::IpAddr;
use std::time::Instant;

use axum::extract::{FromRequestParts, Request, State};
use axum::http::{header, request::Parts};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::permissions::{Level, Permissions, Section};
use super::{client_address, keys, session};
use crate::audit::Actor;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;

/// How a request proved who it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Via {
    Session,
    /// An API key. `full` keys may do whatever their admin may, and so may
    /// manage admins; keys limited to some permissions never may.
    Key {
        id: i64,
        name: String,
        full: bool,
    },
}

/// An authenticated request: the admin, and what they may do.
#[derive(Debug, Clone)]
pub struct Access {
    pub admin: z_db::Admin,
    pub role: Option<z_db::Role>,
    /// The permissions in force: the admin's, narrowed by the key if one is used.
    pub permissions: Permissions,
    /// Sees and changes only the users they made.
    pub own_users_only: bool,
    pub via: Via,
    pub address: IpAddr,
}

impl Access {
    pub fn can(&self, section: Section, level: Level) -> bool {
        self.permissions.can(section, level)
    }

    pub fn require(&self, section: Section, level: Level) -> ApiResult<()> {
        if self.can(section, level) {
            Ok(())
        } else {
            Err(ApiError::forbidden(
                "forbidden",
                format!("you may not {} {}", verb(level), section.name()),
            ))
        }
    }

    /// Managing admins and roles: a sudo admin, in a session or with a key
    /// that is not limited.
    pub fn require_sudo(&self) -> ApiResult<()> {
        let unlimited = match &self.via {
            Via::Session => true,
            Via::Key { full, .. } => *full,
        };
        if self.admin.is_sudo && unlimited {
            Ok(())
        } else {
            Err(ApiError::forbidden(
                "forbidden",
                "only a sudo admin may do this",
            ))
        }
    }

    /// Things done to oneself (a password, 2FA, API keys) need a sign-in: a
    /// key must not be able to hand out more keys or lock its owner out.
    pub fn require_session(&self) -> ApiResult<()> {
        match self.via {
            Via::Session => Ok(()),
            Via::Key { .. } => Err(ApiError::forbidden(
                "session_required",
                "sign in to the panel to do this; an API key cannot",
            )),
        }
    }

    /// The admin whose users this one is limited to, if limited.
    pub fn owner_scope(&self) -> Option<i64> {
        self.own_users_only.then_some(self.admin.id)
    }

    /// The access groups this admin is limited to giving users, if limited.
    pub fn group_scope(&self) -> Option<&[i64]> {
        (!self.admin.is_sudo && !self.admin.group_ids.is_empty())
            .then_some(self.admin.group_ids.as_slice())
    }

    /// Whether this admin may see (and so change) `user`.
    pub fn sees(&self, user: &z_db::User) -> bool {
        !self.own_users_only || user.admin_id == Some(self.admin.id)
    }

    /// What the audit log calls the way this request came in.
    pub fn via_label(&self) -> String {
        match &self.via {
            Via::Session => "session".into(),
            Via::Key { name, .. } => format!("key:{name}"),
        }
    }
}

fn verb(level: Level) -> &'static str {
    match level {
        Level::Read => "view",
        Level::Write => "change",
    }
}

/// What an admin may do by their own right: everything for a sudo admin, the
/// role's permissions for any other.
pub fn permissions_of(admin: &z_db::Admin, role: Option<&z_db::Role>) -> Permissions {
    if admin.is_sudo {
        Permissions::all()
    } else {
        role.map_or(Permissions::NONE, |role| {
            Permissions::from_stored(&role.permissions)
        })
    }
}

/// How long a key's last use is left alone before it is written again.
const TOUCH_EVERY: i64 = 60;

async fn session_access(parts: &Parts, state: &AppState) -> ApiResult<z_db::Admin> {
    let token = parts
        .headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(session::from_cookie_header)
        .ok_or_else(ApiError::unauthorized)?;
    let claims = state
        .session_keys()
        .verify(token)
        .ok_or_else(ApiError::unauthorized)?;
    state
        .db()
        .admin_by_id(claims.sub)
        .await?
        .filter(|admin| admin.token_version == claims.ver && admin.enabled)
        .ok_or_else(ApiError::unauthorized)
}

async fn key_access(
    state: &AppState,
    key: &str,
    address: IpAddr,
) -> ApiResult<(z_db::Admin, Via, Option<Permissions>)> {
    let attempt = format!("{address}");
    if let Some(seconds) = state
        .key_limiter()
        .locked(address, &attempt, Instant::now())
    {
        let mut error = ApiError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "too_many_attempts",
            "too many requests with a wrong API key; wait and try again",
        );
        error.retry_after = Some(seconds);
        return Err(error);
    }
    let invalid = || {
        state
            .key_limiter()
            .failed(address, &attempt, Instant::now());
        ApiError::new(
            axum::http::StatusCode::UNAUTHORIZED,
            "invalid_api_key",
            "the API key is not valid",
        )
    };
    let stored = state
        .db()
        .api_key_by_hash(&keys::hash(key))
        .await?
        .ok_or_else(invalid)?;
    if stored.expires_at.is_some_and(|at| at <= z_db::now()) {
        return Err(ApiError::new(
            axum::http::StatusCode::UNAUTHORIZED,
            "api_key_expired",
            "the API key has expired",
        ));
    }
    if !keys::allows(&stored.allowed_ips, address) {
        return Err(ApiError::forbidden(
            "ip_not_allowed",
            "this API key may not be used from this address",
        ));
    }
    let admin = state
        .db()
        .admin_by_id(stored.admin_id)
        .await?
        .filter(|admin| admin.enabled)
        .ok_or_else(ApiError::unauthorized)?;
    let now = z_db::now();
    if stored.last_used_at.is_none_or(|at| now - at >= TOUCH_EVERY)
        || stored.last_used_ip.as_deref() != Some(&attempt)
    {
        state.db().touch_api_key(stored.id, &attempt).await?;
    }
    let full = stored.permissions.trim() == "*";
    let limit = (!full).then(|| Permissions::from_stored(&stored.permissions));
    Ok((
        admin,
        Via::Key {
            id: stored.id,
            name: stored.name,
            full,
        },
        limit,
    ))
}

/// Work out what `admin` may do, with the key's limit if they came by one.
pub async fn assemble(
    state: &AppState,
    admin: z_db::Admin,
    via: Via,
    limit: Option<Permissions>,
    address: IpAddr,
) -> ApiResult<Access> {
    let role = match admin.role_id {
        Some(id) if !admin.is_sudo => state.db().role(id).await?,
        _ => None,
    };
    let mut permissions = permissions_of(&admin, role.as_ref());
    if let Some(limit) = limit {
        permissions = permissions.and(limit);
    }
    let own_users_only = !admin.is_sudo && role.as_ref().is_some_and(|role| role.own_users_only);
    Ok(Access {
        admin,
        role,
        permissions,
        own_users_only,
        via,
        address,
    })
}

/// Find out who a request is from its API key or its session cookie.
pub async fn authenticate(parts: &Parts, state: &AppState) -> ApiResult<Access> {
    let address = client_address(parts);
    let (admin, via, limit) = match keys::from_headers(&parts.headers) {
        Some(key) => key_access(state, key, address).await?,
        None => (session_access(parts, state).await?, Via::Session, None),
    };
    assemble(state, admin, via, limit, address).await
}

impl FromRequestParts<AppState> for Access {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        // The guard of a route has already found out.
        if let Some(access) = parts.extensions.get::<Access>() {
            return Ok(access.clone());
        }
        authenticate(parts, state).await
    }
}

/// What a route needs of whoever calls it.
#[derive(Debug, Clone, Copy)]
pub enum Need {
    /// Anyone signed in, by session or key.
    Signed,
    /// Anyone signed in to the panel: a key will not do, for what is done to
    /// oneself (a password, 2FA).
    Session,
    /// Read for a GET, write for anything else.
    Section(Section),
    /// As [`Need::Section`], and signed in to the panel: a key will not do.
    SessionSection(Section),
    /// This level on this section, whatever the method.
    Exactly(Section, Level),
    /// A sudo admin.
    Sudo,
}

impl Need {
    fn check(self, access: &Access, method: &axum::http::Method) -> ApiResult<()> {
        match self {
            Need::Signed => Ok(()),
            Need::Session => access.require_session(),
            Need::Section(section) => access.require(section, Level::of_method(method)),
            Need::SessionSection(section) => {
                access.require_session()?;
                access.require(section, Level::of_method(method))
            }
            Need::Exactly(section, level) => access.require(section, level),
            Need::Sudo => access.require_sudo(),
        }
    }

    /// Whether what this route does is not worth a line in the audit log.
    fn quiet(self) -> bool {
        matches!(self, Need::Exactly(_, Level::Read))
    }
}

/// Route middleware: sign the request in, check it is allowed, and let the
/// handler know who it is.
pub async fn guard(
    State((state, need)): State<(AppState, Need)>,
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();
    let access = match authenticate(&parts, &state).await {
        Ok(access) => access,
        Err(error) => return error.into_response(),
    };
    let actor = Actor::of(&access, need.quiet());
    if let Err(error) = need.check(&access, &parts.method) {
        let mut response = error.into_response();
        response.extensions_mut().insert(actor);
        return response;
    }
    parts.extensions.insert(access);
    let mut response = next.run(Request::from_parts(parts, body)).await;
    response.extensions_mut().insert(actor);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admin(is_sudo: bool, role_id: Option<i64>) -> z_db::Admin {
        z_db::Admin {
            id: 7,
            username: "ali".into(),
            password_hash: String::new(),
            totp_secret: None,
            is_sudo,
            token_version: 0,
            created_at: 0,
            last_login_at: None,
            role_id,
            enabled: true,
            user_limit: 0,
            data_quota: 0,
            group_ids: Vec::new(),
            note: String::new(),
        }
    }

    fn role(permissions: &str) -> z_db::Role {
        z_db::Role {
            id: 1,
            name: "r".into(),
            permissions: permissions.into(),
            own_users_only: false,
            created_at: 0,
        }
    }

    #[test]
    fn a_sudo_admin_has_everything_and_another_only_their_roles() {
        assert_eq!(permissions_of(&admin(true, None), None), Permissions::all());
        let reseller = permissions_of(&admin(false, Some(1)), Some(&role("users.write")));
        assert!(reseller.can(Section::Users, Level::Write));
        assert!(!reseller.can(Section::Core, Level::Read));
        assert_eq!(
            permissions_of(&admin(false, Some(1)), None),
            Permissions::NONE,
            "a role that is gone gives nothing"
        );
        assert_eq!(permissions_of(&admin(false, None), None), Permissions::NONE);
    }

    fn access(is_sudo: bool, via: Via) -> Access {
        Access {
            admin: admin(is_sudo, None),
            role: None,
            permissions: permissions_of(&admin(is_sudo, None), None),
            own_users_only: false,
            via,
            address: IpAddr::from([127, 0, 0, 1]),
        }
    }

    #[test]
    fn only_a_sudo_in_a_session_or_with_a_full_key_manages_admins() {
        assert!(access(true, Via::Session).require_sudo().is_ok());
        let key = |full| Via::Key {
            id: 1,
            name: "bot".into(),
            full,
        };
        assert!(access(true, key(true)).require_sudo().is_ok());
        assert!(access(true, key(false)).require_sudo().is_err());
        assert!(access(false, Via::Session).require_sudo().is_err());
        assert!(access(true, key(true)).require_session().is_err());
        assert!(access(true, Via::Session).require_session().is_ok());
    }

    #[test]
    fn a_reseller_sees_only_the_users_they_made() {
        let mut reseller = access(false, Via::Session);
        reseller.own_users_only = true;
        let mut user = z_db::User {
            id: 1,
            username: "u".into(),
            status: "active".into(),
            uuid: String::new(),
            password: String::new(),
            sub_token: String::new(),
            data_limit: 0,
            used_traffic: 0,
            lifetime_traffic: 0,
            expire_at: None,
            on_hold_duration: None,
            on_hold_timeout: None,
            reset_strategy: "no_reset".into(),
            last_reset_at: None,
            ip_limit: 0,
            speed_up: 0,
            speed_down: 0,
            note: String::new(),
            online_at: None,
            admin_id: Some(7),
            created_at: 0,
            updated_at: 0,
            group_ids: Vec::new(),
            next_plan: None,
            hwid_limit: 0,
            sub_updated_at: None,
            sub_user_agent: String::new(),
        };
        assert!(reseller.sees(&user));
        assert_eq!(reseller.owner_scope(), Some(7));
        user.admin_id = Some(8);
        assert!(!reseller.sees(&user));
        user.admin_id = None;
        assert!(!reseller.sees(&user), "nobody's users are not theirs");
        assert!(access(true, Via::Session).sees(&user));
    }
}
