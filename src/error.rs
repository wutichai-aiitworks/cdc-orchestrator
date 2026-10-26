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
    Upstream(String),
    Internal(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRequest(m)
            | Self::Unauthorized(m)
            | Self::NotFound(m)
            | Self::Upstream(m)
            | Self::Internal(m) => write!(f, "{m}"),
        }
    }
}

#[derive(Serialize)]
struct ErrorPayload {
    error: String,
}

impl AppError {
    pub fn to_string_message(&self) -> String {
        match self {
            Self::BadRequest(m)
            | Self::Unauthorized(m)
            | Self::NotFound(m)
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
            Self::Upstream(m) => (StatusCode::BAD_GATEWAY, m),
            Self::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };

        (status, Json(ErrorPayload { error: message })).into_response()
    }
}
