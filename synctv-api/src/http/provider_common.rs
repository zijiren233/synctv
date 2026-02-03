//! Common Provider Route Utilities
//!
//! Shared functionality across all provider routes

use axum::{extract::State, routing::get, Json, Router, response::IntoResponse};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::json;
use synctv_core::provider::ProviderError;

use super::AppState;

/// Register common provider routes
///
/// Routes:
/// - GET /instances - List all available provider instances
pub fn register_common_routes() -> Router<AppState> {
    Router::new().route("/instances", get(list_instances))
}

/// List all available provider instances
async fn list_instances(State(state): State<AppState>) -> impl IntoResponse {
    let instances = state.provider_instance_manager.list().await;

    Json(json!({
        "instances": instances
    }))
}

/// Convert ProviderError to HTTP response
pub fn error_response(e: ProviderError) -> (StatusCode, Json<serde_json::Value>) {
    let (status, message, details) = match &e {
        ProviderError::NetworkError(msg) => (StatusCode::BAD_GATEWAY, msg.clone(), msg.clone()),
        ProviderError::ApiError(msg) => (StatusCode::BAD_GATEWAY, msg.clone(), msg.clone()),
        ProviderError::ParseError(msg) => (StatusCode::BAD_REQUEST, msg.clone(), msg.clone()),
        ProviderError::InvalidConfig(msg) => (StatusCode::BAD_REQUEST, msg.clone(), msg.clone()),
        ProviderError::NotFound => (StatusCode::NOT_FOUND, "Resource not found".to_string(), "Resource not found".to_string()),
        ProviderError::InstanceNotFound(msg) => (StatusCode::NOT_FOUND, msg.clone(), msg.clone()),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "Provider error".to_string(), e.to_string()),
    };

    let body = json!({
        "error": message,
        "details": details
    });

    (status, Json(body))
}

/// Convert String error to HTTP response (for implementation layer errors)
pub fn parse_provider_error(error_msg: &str) -> ProviderError {
    // Parse common error patterns and convert to ProviderError
    let lower = error_msg.to_lowercase();

    if lower.contains("network") || lower.contains("connection") {
        ProviderError::NetworkError(error_msg.to_string())
    } else if lower.contains("not found") {
        ProviderError::NotFound
    } else if lower.contains("parse") || lower.contains("invalid") {
        ProviderError::ParseError(error_msg.to_string())
    } else {
        // Unauthorized, authentication, or any other error
        ProviderError::ApiError(error_msg.to_string())
    }
}

/// Extract instance_name from query parameter
#[derive(Debug, Deserialize)]
pub struct InstanceQuery {
    #[serde(default)]
    pub instance_name: Option<String>,
}

impl InstanceQuery {
    pub fn as_deref(&self) -> Option<&str> {
        self.instance_name.as_deref()
    }
}
