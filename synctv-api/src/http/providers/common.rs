//! Common Provider HTTP Endpoints

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    http::StatusCode,
    Json,
};
use serde_json::json;
use crate::http::AppState;

/// Get all backend instances for a specific vendor/provider
///
/// GET /api/vendor/backends/:vendor
pub async fn backends(
    State(state): State<AppState>,
    Path(vendor): Path<String>,
) -> impl IntoResponse {
    tracing::info!("Get backends for vendor: {}", vendor);

    // Get all provider instances from instance manager
    let instances = match state.provider_instance_manager.get_all_instances().await {
        Ok(instances) => instances,
        Err(e) => {
            tracing::error!("Failed to get provider instances: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to get provider instances",
                    "message": e.to_string()
                }))
            ).into_response();
        }
    };

    // Filter by vendor type (check if providers array contains the vendor)
    let vendor_instances: Vec<_> = instances.into_iter()
        .filter(|instance| instance.providers.contains(&vendor))
        .map(|instance| json!({
            "name": instance.name,
            "providers": instance.providers,
            "enabled": instance.enabled,
            "created_at": instance.created_at,
        }))
        .collect();

    (
        StatusCode::OK,
        Json(json!({
            "backends": vendor_instances
        }))
    ).into_response()
}
