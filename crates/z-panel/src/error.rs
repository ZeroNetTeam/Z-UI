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

#[derive(Serialize)]
struct Body<'a> {
    error: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after: Option<u64>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(Body {
            error: self.code,
            message: &self.message,
            retry_after: self.retry_after,
        });
        let mut response = (self.status, body).into_response();
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
