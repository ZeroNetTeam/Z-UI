//! API errors: a status and a stable code the dashboard translates, with an
//! English message for scripts and logs.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    /// Seconds before trying again, for rate limits.
    pub retry_after: Option<u64>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            retry_after: None,
        }
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "sign in first")
    }

    pub fn forbidden(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, message)
    }

    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(%error, "request failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "something went wrong on the server",
        )
    }
}

impl From<z_db::DbError> for ApiError {
    fn from(error: z_db::DbError) -> Self {
        Self::internal(error)
    }
}

/// What every failed request answers with, as the API documents it.
#[derive(Serialize, utoipa::ToSchema)]
#[allow(dead_code)]
pub struct ErrorBody {
    /// A stable code the dashboard translates: `forbidden`, `quota_exceeded`…
    error: String,
    /// The same in English, for scripts and logs.
    message: String,
    /// Seconds to wait before trying again, when rate limited.
    retry_after: Option<u64>,
}

#[derive(Serialize)]
struct Body<'a> {
    error: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after: Option<u64>,
}

/// The code of the error a response carries, for the audit log.
#[derive(Debug, Clone, Copy)]
pub struct ErrorCode(pub &'static str);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(Body {
            error: self.code,
            message: &self.message,
            retry_after: self.retry_after,
        });
        let mut response = (self.status, body).into_response();
        response.extensions_mut().insert(ErrorCode(self.code));
        if let Some(seconds) = self.retry_after {
            if let Ok(value) = seconds.to_string().parse() {
                response
                    .headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, value);
            }
        }
        response
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
