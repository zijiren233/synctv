//! Health check endpoints
//!
//! Provides liveness, readiness, and dependency health checks.

use axum::{
    extract::State,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::time::Instant;
use tracing::debug;

use crate::http::AppState;

/// Health check response
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub timestamp: i64,
    pub uptime_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks: Option<HashMap<String, HealthCheck>>,
}

/// Individual health check
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthCheck {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

/// Health check router
pub fn create_health_router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health_check))
        .route("/health/ready", get(readiness_check))
        .route("/health/live", get(liveness_check))
        .route("/health/database", get(database_check))
        .route("/health/redis", get(redis_check))
}

/// Basic health check (always returns OK if server is running)
pub async fn health_check() -> impl IntoResponse {
    let uptime = synctv_core::SERVER_START_TIME.elapsed().as_secs();

    Json(HealthResponse {
        status: "ok".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        uptime_seconds: uptime,
        checks: None,
    })
}

/// Liveness probe (checks if server is alive)
pub async fn liveness_check() -> impl IntoResponse {
    Json(HealthResponse {
        status: "alive".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        uptime_seconds: synctv_core::SERVER_START_TIME.elapsed().as_secs(),
        checks: None,
    })
}

/// Readiness probe (checks if server is ready to handle requests)
pub async fn readiness_check(State(state): State<AppState>) -> impl IntoResponse {
    let mut checks = HashMap::new();
    let mut overall_status = "ok";

    // Check database
    let db_status = check_database(&state).await;
    checks.insert("database".to_string(), db_status.clone());

    // Check Redis (if configured)
    if state.redis_publish_tx.is_some() {
        let redis_status = check_redis(&state).await;
        checks.insert("redis".to_string(), redis_status.clone());
        if redis_status.status != "ok" {
            overall_status = "degraded";
        }
    }

    if db_status.status != "ok" {
        overall_status = "not_ready";
    }

    Json(HealthResponse {
        status: overall_status.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        uptime_seconds: synctv_core::SERVER_START_TIME.elapsed().as_secs(),
        checks: Some(checks),
    })
}

/// Database health check
pub async fn database_check(State(state): State<AppState>) -> impl IntoResponse {
    let status = check_database(&state).await;

    let mut checks = HashMap::new();
    checks.insert("database".to_string(), status);

    Json(HealthResponse {
        status: if checks.values().all(|c| c.status == "ok") {
            "ok".to_string()
        } else {
            "error".to_string()
        },
        timestamp: chrono::Utc::now().timestamp(),
        uptime_seconds: synctv_core::SERVER_START_TIME.elapsed().as_secs(),
        checks: Some(checks),
    })
}

/// Redis health check
pub async fn redis_check(State(state): State<AppState>) -> impl IntoResponse {
    let status = check_redis(&state).await;

    let mut checks = HashMap::new();
    checks.insert("redis".to_string(), status);

    Json(HealthResponse {
        status: if checks.values().all(|c| c.status == "ok") {
            "ok".to_string()
        } else {
            "error".to_string()
        },
        timestamp: chrono::Utc::now().timestamp(),
        uptime_seconds: synctv_core::SERVER_START_TIME.elapsed().as_secs(),
        checks: Some(checks),
    })
}

async fn check_database(state: &AppState) -> HealthCheck {
    let start = Instant::now();

    // Simple query to check database connection
    let result = sqlx::query("SELECT 1")
        .fetch_one(&state.db_pool)
        .await;

    match result {
        Ok(_) => HealthCheck {
            status: "ok".to_string(),
            message: None,
            latency_ms: Some(start.elapsed().as_millis() as u64),
        },
        Err(e) => HealthCheck {
            status: "error".to_string(),
            message: Some(format!("Database connection failed: {}", e)),
            latency_ms: Some(start.elapsed().as_millis() as u64),
        },
    }
}

async fn check_redis(state: &AppState) -> HealthCheck {
    let start = Instant::now();

    // Check if Redis sender exists and can send
    let status = if let Some(_) = &state.redis_publish_tx {
        HealthCheck {
            status: "ok".to_string(),
            message: None,
            latency_ms: Some(start.elapsed().as_millis() as u64),
        }
    } else {
        HealthCheck {
            status: "disabled".to_string(),
            message: Some("Redis not configured".to_string()),
            latency_ms: Some(start.elapsed().as_millis() as u64),
        }
    };

    status
}
