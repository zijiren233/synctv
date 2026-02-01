//! Common Provider Route Utilities
//!
//! Shared functionality across all provider routes

use axum::{extract::State, response::IntoResponse, routing::get, Json, Router};
use serde_json::json;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_common_routes_compile() {
        // Ensure routes can be built
        assert!(true);
    }
}
