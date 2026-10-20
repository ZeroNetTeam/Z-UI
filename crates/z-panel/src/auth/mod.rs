//! Who is making a request, and whether they may.

pub mod access;
pub mod keys;
pub mod password;
pub mod permissions;
pub mod rate_limit;
pub mod session;
pub mod totp;

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, request::Parts, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::server::AppState;

pub use access::Access;

/// The address a request came from (the TCP peer; a proxy in front is not
/// trusted to say otherwise).
pub fn client_address(parts: &Parts) -> IpAddr {
    parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(IpAddr::from([0, 0, 0, 0]), |info| info.0.ip())
}

/// Whether the browser reached the panel over HTTPS, so the session cookie
/// may be marked `Secure`.
pub fn is_secure(parts: &Parts, tls: bool) -> bool {
    tls || parts
        .headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|proto| proto.eq_ignore_ascii_case("https"))
}

/// Refuse a request that changes something when a browser says it came
/// from another site. `SameSite=Strict` already keeps the session cookie
/// off such requests; this is the second lock on the same door.
pub async fn same_origin(State(_): State<AppState>, request: Request, next: Next) -> Response {
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        let headers = request.headers();
        let origin_host = headers
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            .map(|origin| origin.split("://").nth(1).unwrap_or(origin).to_owned());
        let host = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok());
        if let (Some(origin), Some(host)) = (origin_host, host) {
            if !origin.eq_ignore_ascii_case(host) {
                return (StatusCode::FORBIDDEN, "cross-origin request refused").into_response();
            }
        }
    }
    next.run(request).await
}
