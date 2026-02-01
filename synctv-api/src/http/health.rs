//! Health check endpoints
//!
//! Provides simple health check for monitoring probes.

use axum::{
    response::IntoResponse,
    routing::get,
    Router,
};

use crate::http::AppState;

/// Health check router
pub fn create_health_router() -> Router<AppState> {
    Router::new().route("/health", get(health_check))
}

/// Basic health check (always returns OK if server is running)
pub async fn health_check() -> impl IntoResponse {
    "OK"
}
