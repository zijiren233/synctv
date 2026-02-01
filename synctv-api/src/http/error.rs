// HTTP error handling

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Result type for HTTP handlers
pub type AppResult<T> = Result<T, AppError>;

/// Application error with HTTP status code
#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub message: String,
}

impl AppError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    pub fn internal_server_error(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    // Convenience alias
    pub fn internal(message: impl Into<String>) -> Self {
        Self::internal_server_error(message)
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}

impl std::error::Error for AppError {}

/// Error response JSON structure
#[derive(Debug, Serialize, Deserialize)]
struct ErrorResponse {
    error: String,
    status: u16,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status;
        let body = Json(ErrorResponse {
            error: self.message,
            status: status.as_u16(),
        });

        (status, body).into_response()
    }
}

/// Convert synctv_core errors to HTTP errors
impl From<synctv_core::Error> for AppError {
    fn from(err: synctv_core::Error) -> Self {
        use synctv_core::Error;

        match err {
            Error::NotFound(msg) => AppError::not_found(msg),
            Error::AlreadyExists(msg) => AppError::conflict(msg),
            Error::Unauthorized(msg) => AppError::unauthorized(msg),
            Error::Authentication(msg) => AppError::unauthorized(msg),
            Error::Authorization(msg) => AppError::forbidden(msg),
            Error::PermissionDenied(msg) => AppError::forbidden(msg),
            Error::InvalidInput(msg) => AppError::bad_request(msg),
            Error::Database(e) => {
                tracing::error!("Database error: {}", e);
                AppError::internal_server_error("Database error")
            }
            Error::Redis(e) => {
                tracing::error!("Redis error: {}", e);
                AppError::internal_server_error("Service temporarily unavailable")
            }
            Error::Serialization(e) => {
                tracing::error!("Serialization error: {}", e);
                AppError::internal_server_error("Data processing error")
            }
            Error::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                AppError::internal_server_error("Internal server error")
            }
        }
    }
}

/// Convert serde_json errors to HTTP errors
impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::bad_request(format!("JSON error: {}", err))
    }
}

/// Convert anyhow errors to HTTP errors
impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        tracing::error!("Anyhow error: {}", err);
        AppError::internal_server_error("Internal server error")
    }
}
