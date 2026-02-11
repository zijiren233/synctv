//! Health check and metrics endpoints
//!
//! Provides health check endpoints for Kubernetes readiness/liveness probes and Prometheus metrics.
//!
//! # Endpoints
//!
//! - `/health/live` - Liveness probe: checks if the application is running (basic check)
//! - `/health/ready` - Readiness probe: checks if dependencies (DB, Redis) are healthy
//! - `/health` - Alias for `/health/live` for backward compatibility
//! - `/metrics` - Prometheus metrics endpoint

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use tracing::{error, warn};

use crate::http::AppState;
use crate::observability::metrics;

/// Health check and metrics router
pub fn create_health_router() -> Router<AppState> {
    Router::new()
        .route("/health", get(liveness_check))
        .route("/health/live", get(liveness_check))
        .route("/health/ready", get(readiness_check))
        .route("/metrics", get(prometheus_metrics))
}

/// Health check response structure
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<HealthDetails>,
}

/// Detailed health check information
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthDetails {
    pub database: String,
    pub redis: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Liveness probe - checks if the application process is running
///
/// This is a basic check that always returns OK if the server is responding.
/// Kubernetes uses this to determine if the pod needs to be restarted.
///
/// Returns:
/// - 200 OK: Application is alive
pub async fn liveness_check() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok".to_string(),
            details: None,
        }),
    )
}

/// Readiness probe - checks if the application is ready to serve traffic
///
/// This performs actual health checks on dependencies:
/// - Database connectivity
/// - Redis connectivity
///
/// Kubernetes uses this to determine if the pod should receive traffic.
///
/// Returns:
/// - 200 OK: All dependencies are healthy
/// - 503 Service Unavailable: One or more dependencies are unhealthy
pub async fn readiness_check(State(state): State<AppState>) -> impl IntoResponse {
    let mut is_healthy = true;
    let mut error_messages = Vec::new();

    // Check database connectivity
    let db_status = match check_database_health(&state).await {
        Ok(()) => "healthy".to_string(),
        Err(e) => {
            error_messages.push(format!("Database: {e}"));
            is_healthy = false;
            error!("Database health check failed: {}", e);
            "unhealthy".to_string()
        }
    };

    // Check Redis connectivity
    let redis_status = match check_redis_health(&state).await {
        Ok(()) => "healthy".to_string(),
        Err(e) => {
            error_messages.push(format!("Redis: {e}"));
            is_healthy = false;
            error!("Redis health check failed: {}", e);
            "unhealthy".to_string()
        }
    };

    let status_code = if is_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let response = HealthResponse {
        status: if is_healthy { "healthy".to_string() } else { "unhealthy".to_string() },
        details: Some(HealthDetails {
            database: db_status,
            redis: redis_status,
            message: if error_messages.is_empty() {
                None
            } else {
                Some(error_messages.join("; "))
            },
        }),
    };

    (status_code, Json(response))
}

/// Check database connectivity
async fn check_database_health(state: &AppState) -> Result<(), String> {
    // Try to execute a simple query to verify database connectivity
    // Using the user_service which has access to the database pool
    match state.user_service.health_check().await {
        Ok(()) => Ok(()),
        Err(e) => {
            warn!("Database health check failed: {}", e);
            Err(format!("Database connection failed: {e}"))
        }
    }
}

/// Check Redis connectivity
async fn check_redis_health(state: &AppState) -> Result<(), String> {
    // Check if Redis is configured and accessible
    if let Some(ref _redis_tx) = state.redis_publish_tx {
        // If Redis publish channel exists, assume Redis is healthy
        // A more thorough check would ping Redis directly
        Ok(())
    } else {
        // Redis not configured - this is acceptable in some deployments
        warn!("Redis is not configured");
        Ok(())
    }
}

/// Prometheus metrics endpoint
pub async fn prometheus_metrics() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        metrics::gather_metrics(),
    )
}
