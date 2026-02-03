//! Publish key API endpoints
//!
//! HTTP endpoints for generating RTMP publish keys for live streaming.
//! Streaming is scoped to individual media items, not rooms.

use axum::{
    extract::{Path, State},
    response::Json,
    routing::post,
    Router,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::http::{AppState, AppError, AppResult, middleware::AuthUser};
use synctv_core::models::{MediaId, RoomId};

/// Publish key response
#[derive(Debug, Serialize)]
pub struct PublishKeyResponse {
    /// JWT token for RTMP authentication
    pub token: String,
    /// Room ID
    pub room_id: String,
    /// Media/Stream ID
    pub media_id: String,
    /// User ID who requested the key
    pub user_id: String,
    /// Expiration timestamp (Unix)
    pub expires_at: i64,
    /// RTMP URL with stream key
    pub rtmp_url: String,
    /// Stream key
    pub stream_key: String,
}

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
/// Stream name format: {room_id}/{media_id}
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
) -> AppResult<Json<PublishKeyResponse>> {
    let room_id = RoomId::from_string(room_id);
    let media_id = MediaId::from_string(media_id);
    let user_id = auth_user.user_id;

    // Get PublishKeyService from state
    let publish_key_service = state.publish_key_service.as_ref()
        .ok_or_else(|| AppError::internal_server_error("Publish key service not configured"))?;

    // Check permission to start live stream
    state
        .room_service
        .check_permission(&room_id, &user_id, synctv_core::models::PermissionBits::START_LIVE)
        .await
        .map_err(|e| AppError::forbidden(format!("Permission denied: {}", e)))?;

    // Generate publish key for this specific media item
    let publish_key = publish_key_service
        .generate_publish_key(room_id.clone(), media_id.clone(), user_id.clone())
        .await
        .map_err(|e| AppError::internal_server_error(format!("Failed to generate publish key: {}", e)))?;

    // Construct RTMP URL and stream key
    // Stream name format: {room_id}/{media_id}
    let rtmp_url = format!("rtmp://localhost:1935/live/{}", room_id.as_str());
    let stream_key = publish_key.token.clone();

    info!(
        room_id = publish_key.room_id,
        media_id = publish_key.media_id,
        user_id = publish_key.user_id,
        "Generated publish key for media stream"
    );

    Ok(Json(PublishKeyResponse {
        token: publish_key.token,
        room_id: publish_key.room_id,
        media_id: publish_key.media_id,
        user_id: publish_key.user_id,
        expires_at: publish_key.expires_at,
        rtmp_url,
        stream_key,
    }))
}
