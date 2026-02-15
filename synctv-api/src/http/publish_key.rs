//! Publish key API endpoints
//!
//! HTTP endpoints for generating RTMP publish keys for live streaming.
//! Streaming is scoped to individual media items, not rooms.
//!
//! Uses proto-generated types for response to ensure type consistency
//! with gRPC handlers.

use axum::{
    extract::{Path, State},
    response::Json,
    routing::post,
    Router,
};

use crate::http::{AppState, AppError, AppResult, middleware::AuthUser};
use crate::proto::client::CreatePublishKeyResponse;

/// Create publish key routes
pub fn create_publish_key_router() -> Router<AppState> {
    Router::new().route(
        "/rooms/:room_id/movies/:media_id/live/publish-key",
        post(generate_publish_key),
    )
}

/// Generate a publish key for RTMP streaming
///
/// POST /rooms/:room_id/movies/:media_id/live/publish-key
/// Requires authentication
///
/// Generates a JWT token for a specific media item.
/// Stream name format: {`room_id}/{media_id`}
///
/// Based on synctv-go implementation:
/// - Endpoint: POST /api/room/movie/:movieId/live/publishKey
/// - Multiple concurrent streams per room (one per media item)
/// - Each media item can have independent RTMP stream
#[axum::debug_handler]
pub async fn generate_publish_key(
    State(state): State<AppState>,
    Path((room_id, media_id)): Path<(String, String)>,
    auth_user: AuthUser,
) -> AppResult<Json<CreatePublishKeyResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    // Delegate to shared ClientApiImpl (handles permission check, key generation, RTMP URL)
    let req = crate::proto::client::CreatePublishKeyRequest {
        id: media_id,
    };
    let resp = state
        .client_api
        .create_publish_key(&user_id_str, &room_id, req)
        .await
        .map_err(|e| {
            if e.contains("Permission denied") {
                AppError::forbidden(e)
            } else if e.contains("not found") {
                AppError::not_found(e)
            } else {
                AppError::internal_server_error(e)
            }
        })?;

    Ok(Json(resp))
}
