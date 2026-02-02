//! Publish key API endpoints
//!
//! HTTP endpoints for generating RTMP publish keys for live streaming.

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Json},
    routing::post,
    Router,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::http::{AppState, AppError, AppResult};
use synctv_core::models::{MediaId, RoomId};

/// Request to generate a publish key
#[derive(Debug, Deserialize)]
pub struct GeneratePublishKeyRequest {
    /// Optional media/stream ID
    pub media_id: Option<String>,
}

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
    Router::new().route("/api/rooms/:room_id/publish-key", post(generate_publish_key))
}

/// Generate a publish key for RTMP streaming
///
/// POST /api/rooms/:room_id/publish-key
/// Requires authentication
///
/// Generates a JWT token that can be used to authenticate RTMP push.
/// The token includes room_id, media_id, user_id, and expiration time.
pub async fn generate_publish_key(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<GeneratePublishKeyRequest>,
) -> AppResult<Json<PublishKeyResponse>> {
    // Get user ID from JWT (would normally come from middleware)
    let user_id = "user_id_from_jwt".to_string(); // TODO: Get from auth middleware

    let room_id = RoomId::from_string(room_id);
    let user_id = synctv_core::models::UserId::from_string(user_id);

    // Get PublishKeyService from state
    let publish_key_service = state.publish_key_service.as_ref()
        .ok_or_else(|| AppError::internal_server_error("Publish key service not configured"))?;

    // Use media_id from request or generate a default one
    let media_id = req.media_id
        .unwrap_or_else(|| format!("stream_{}", nanoid::nanoid!(8)));
    let media_id = MediaId::from_string(media_id);

    // Generate publish key
    let publish_key = publish_key_service
        .generate_publish_key(room_id.clone(), media_id.clone(), user_id.clone())
        .await
        .map_err(|e| AppError::internal_server_error(&format!("Failed to generate publish key: {}", e)))?;

    // Construct RTMP URL and stream key
    let rtmp_url = format!("rtmp://localhost:1935/live/{}", publish_key.room_id);
    let stream_key = publish_key.token.clone();

    info!(
        room_id = publish_key.room_id,
        user_id = publish_key.user_id,
        "Generated publish key"
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
