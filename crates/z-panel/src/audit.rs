//! The audit log: what admins did, written down as they do it.
//!
//! Every request that changes something is recorded after it is answered:
//! who, how (a session or which API key), from where, what, and whether it
//! worked. A handler says what it did by adding an [`Audit`] to its reply;
//! a request that failed, or whose handler said nothing, is recorded under
//! the route it used so that nothing goes unseen.

use std::net::IpAddr;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponseParts, Response, ResponseParts};

use crate::auth::access::Access;
use crate::auth::permissions::Level;
use crate::error::ErrorCode;
use crate::server::AppState;

/// How long entries are kept.
pub const RETENTION: Duration = Duration::from_secs(180 * 24 * 60 * 60);

/// What a handler did, to be recorded when its reply is sent.
#[derive(Debug, Clone)]
pub struct Audit {
    action: &'static str,
    target: String,
    detail: String,
}

impl Audit {
    /// `action` is `<thing>.<verb>` (`user.create`); `target` names what it
    /// was done to (the username).
    pub fn new(action: &'static str, target: impl Into<String>) -> Self {
        Self {
            action,
            target: target.into(),
            detail: String::new(),
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    /// For a caller that is not an HTTP request (the bot) and writes the
    /// entry itself.
    pub fn parts(&self) -> (&'static str, &str, &str) {
        (self.action, &self.target, &self.detail)
    }
}

impl IntoResponseParts for Audit {
    type Error = std::convert::Infallible;

    fn into_response_parts(self, mut parts: ResponseParts) -> Result<ResponseParts, Self::Error> {
        parts.extensions_mut().insert(self);
        Ok(parts)
    }
}

/// Who made a request, attached to its response.
#[derive(Debug, Clone)]
pub struct Actor {
    pub admin_id: Option<i64>,
    pub name: String,
    pub via: String,
    pub address: IpAddr,
    /// What the request does is not worth a line in the log.
    pub quiet: bool,
}

impl Actor {
    pub fn of(access: &Access, quiet: bool) -> Self {
        Self {
            admin_id: Some(access.admin.id),
            name: access.admin.username.clone(),
            via: access.via_label(),
            address: access.address,
            quiet,
        }
    }

    /// Someone who is signing in, not yet (or not) an admin.
    pub fn signing_in(admin: Option<&z_db::Admin>, address: IpAddr) -> Self {
        Self {
            admin_id: admin.map(|admin| admin.id),
            name: admin.map_or_else(|| "(unknown)".into(), |admin| admin.username.clone()),
            via: "session".into(),
            address,
            quiet: false,
        }
    }
}

impl IntoResponseParts for Actor {
    type Error = std::convert::Infallible;

    fn into_response_parts(self, mut parts: ResponseParts) -> Result<ResponseParts, Self::Error> {
        parts.extensions_mut().insert(self);
        Ok(parts)
    }
}

/// Write one entry. A log that cannot be written must not fail the request
/// it is about, so a failure is only reported.
pub async fn write(
    state: &AppState,
    actor: &Actor,
    action: &str,
    target: &str,
    detail: &str,
    status: u16,
) {
    let entry = z_db::NewAudit {
        admin_id: actor.admin_id,
        admin_name: actor.name.chars().take(64).collect(),
        via: actor.via.chars().take(80).collect(),
        ip: actor.address.to_string(),
        action: action.chars().take(64).collect(),
        target: target.chars().take(128).collect(),
        detail: detail.chars().take(1000).collect(),
        status: i64::from(status),
    };
    if let Err(error) = state.db().add_audit(&entry).await {
        tracing::warn!(%error, action, "the audit log could not be written");
    }
}

/// `users.post` for a `POST` to `/users/12/reset`, with `12/reset` as its
/// target: what a request is called when nobody said otherwise.
fn route_name(method: &axum::http::Method, path: &str) -> (String, String) {
    let mut parts = path.trim_matches('/').splitn(2, '/');
    let section = parts.next().unwrap_or_default();
    let rest = parts.next().unwrap_or_default();
    (
        format!("{section}.{}", method.as_str().to_ascii_lowercase()),
        rest.to_owned(),
    )
}

/// Middleware over the whole API: record every request that changes
/// something and was made by a known admin.
pub async fn record(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if Level::of_method(request.method()) == Level::Read {
        return next.run(request).await;
    }
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let response = next.run(request).await;
    let Some(actor) = response.extensions().get::<Actor>().cloned() else {
        return response;
    };
    if actor.quiet {
        return response;
    }
    let status = response.status().as_u16();
    let said = response.extensions().get::<Audit>().cloned();
    let code = response
        .extensions()
        .get::<ErrorCode>()
        .map_or("", |code| code.0);
    match said {
        Some(said) if status < 400 => {
            write(
                &state,
                &actor,
                said.action,
                &said.target,
                &said.detail,
                status,
            )
            .await;
        }
        _ => {
            let (action, target) = route_name(&method, &path);
            write(&state, &actor, &action, &target, code, status).await;
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Method;

    #[test]
    fn an_unnamed_request_is_called_after_its_route() {
        assert_eq!(
            route_name(&Method::POST, "/users/12/reset"),
            ("users.post".to_string(), "12/reset".to_string())
        );
        assert_eq!(
            route_name(&Method::DELETE, "/inbounds/3"),
            ("inbounds.delete".to_string(), "3".to_string())
        );
        assert_eq!(
            route_name(&Method::PUT, "/core"),
            ("core.put".to_string(), String::new())
        );
    }
}
