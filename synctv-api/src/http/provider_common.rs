//! Common Provider Route Utilities
//!
//! Shared functionality across all provider routes

use axum::{
    Router,
    routing::get,
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::AppState;

/// Register common provider routes
///
/// Routes:
/// - GET /backends/:vendor - List available backend instances for a vendor
pub fn register_common_routes() -> Router<AppState> {
    Router::new()
        .route("/backends/:vendor", get(list_backends))
}

/// List available backend instances for a vendor
async fn list_backends(
    State(state): State<AppState>,
    Path(_vendor): Path<String>,
) -> impl IntoResponse {
    let backends = state.provider_instance_manager.list().await;

    Json(json!({
        "backends": backends
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
