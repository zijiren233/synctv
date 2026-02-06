//! Health check and metrics endpoints
//!
//! Provides simple health check for monitoring probes and Prometheus metrics.

use axum::{
    response::IntoResponse,
    routing::get,
    Router,
};

use crate::http::AppState;
use crate::observability::metrics;

/// Health check and metrics router
pub fn create_health_router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health_check))
        .route("/metrics", get(prometheus_metrics))
}

/// Basic health check (always returns OK if server is running)
pub async fn health_check() -> impl IntoResponse {
    "OK"
}

/// Prometheus metrics endpoint
pub async fn prometheus_metrics() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        metrics::gather_metrics(),
    )
}
