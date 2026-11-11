use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use std::fmt;

#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Unauthorized(String),
    NotFound(String),
    /// Another request is already acting on the same resource (HTTP 409).
    Conflict(String),
    Upstream(String),
    Internal(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRequest(m)
            | Self::Unauthorized(m)
            | Self::NotFound(m)
            | Self::Conflict(m)
            | Self::Upstream(m)
            | Self::Internal(m) => write!(f, "{m}"),
        }
    }
}

#[derive(Serialize)]
struct ErrorPayload {
    error: String,
    /// Where the failure is and how to fix it, when the message is recognised.
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnosis: Option<crate::diagnose::Diagnosis>,
}

impl AppError {
    pub fn to_string_message(&self) -> String {
        match self {
            Self::BadRequest(m)
            | Self::Unauthorized(m)
            | Self::NotFound(m)
            | Self::Conflict(m)
            | Self::Upstream(m)
            | Self::Internal(m) => m.clone(),
        }
    }
}

impl From<hyper_cdc_orchestrator::persist::PersistError> for AppError {
    fn from(value: hyper_cdc_orchestrator::persist::PersistError) -> Self {
        match value {
            hyper_cdc_orchestrator::persist::PersistError::Upstream(m) => Self::Upstream(m),
            hyper_cdc_orchestrator::persist::PersistError::Internal(m) => Self::Internal(m),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            Self::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m),
            Self::NotFound(m) => (StatusCode::NOT_FOUND, m),
            Self::Conflict(m) => (StatusCode::CONFLICT, m),
            Self::Upstream(m) => (StatusCode::BAD_GATEWAY, m),
            Self::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };

        let diagnosis = crate::diagnose::diagnose(&message, crate::diagnose::Side::Unknown);
        (
            status,
            Json(ErrorPayload {
                error: message,
                diagnosis,
            }),
        )
            .into_response()
    }
}
