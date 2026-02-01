// Live streaming router for synctv-api integration
// Provides FLV, HLS, and publish key endpoints

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use streamhub::define::StreamHubEventSender;
use tracing::{error, info, warn};

use crate::relay::StreamRegistry;

/// State for live streaming endpoints
#[derive(Clone)]
pub struct LiveStreamingState {
    registry: Arc<StreamRegistry>,
    stream_hub_event_sender: StreamHubEventSender,
    // TODO: Add JWT service for publish key generation
}

impl LiveStreamingState {
    pub fn new(
        registry: Arc<StreamRegistry>,
        stream_hub_event_sender: StreamHubEventSender,
    ) -> Self {
        Self {
            registry,
            stream_hub_event_sender,
        }
    }
}

/// Create live streaming router
/// Routes (all under /api/room/movie/live):
/// - POST   /publishKey        - Generate RTMP publish key (requires room auth)
/// - GET    /flv/:media_id     - FLV streaming (requires room auth)
/// - GET    /hls/list/:media_id - HLS playlist (requires room auth)
/// - GET    /hls/data/:room_id/:media_id/:segment - HLS segment data (no auth)
pub fn create_live_router(state: LiveStreamingState) -> Router {
    Router::new()
        // POST /live/publishKey - Generate publish key (requires auth)
        .route("/publishKey", post(handle_new_publish_key))
        // GET /live/flv/:media_id - FLV streaming (requires auth)
        .route("/flv/:media_id", get(handle_flv_stream))
        // GET /live/hls/list/:media_id - HLS playlist (requires auth)
        .route("/hls/list/:media_id", get(handle_hls_list))
        // GET /live/hls/data/:room_id/:media_id/:segment - HLS data (no auth required)
        .route(
            "/hls/data/:room_id/:media_id/:segment",
            get(handle_hls_data),
        )
        .with_state(state)
}

/// Request for generating publish key
#[derive(Deserialize)]
struct NewPublishKeyRequest {
    id: String, // media_id
}

/// Response for publish key
#[derive(Serialize)]
struct PublishKeyResponse {
    publish_key: String,
}

/// Handle POST /live/publishKey
/// Generate RTMP publish key (JWT token)
/// Requires: Extension<RoomId> and Extension<UserId> from auth middleware
async fn handle_new_publish_key(
    Extension(room_id): Extension<String>,
    Extension(user_id): Extension<String>,
    State(_state): State<LiveStreamingState>,
    Json(req): Json<NewPublishKeyRequest>,
) -> Result<Json<PublishKeyResponse>, StatusCode> {
    info!(
        room_id = %room_id,
        user_id = %user_id,
        media_id = %req.id,
        "Generate publish key request"
    );

    // TODO: Verify user is media creator
    // TODO: Generate JWT token with room_id + media_id
    // For now, return placeholder

    let publish_key = format!("{}:{}:{}", room_id, req.id, user_id);

    Ok(Json(PublishKeyResponse { publish_key }))
}

/// Handle GET /live/flv/:media_id
/// FLV streaming endpoint
/// Requires: Extension<RoomId> from auth middleware
async fn handle_flv_stream(
    Path(media_id): Path<String>,
    Extension(room_id): Extension<String>,
    State(state): State<LiveStreamingState>,
) -> Result<Response, StatusCode> {
    // Remove .flv suffix if present
    let media_id = media_id.trim_end_matches(".flv");

    info!(
        room_id = %room_id,
        media_id = %media_id,
        "FLV streaming request"
    );

    // Check if stream exists (publisher registered)
    let mut registry = (*state.registry).clone();
    match registry.get_publisher(&room_id, media_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            warn!("No publisher for room {} / media {}", room_id, media_id);
            return Err(StatusCode::NOT_FOUND);
        }
        Err(e) => {
            error!("Failed to query publisher: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    // Create channel for HTTP response data
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<bytes::Bytes, std::io::Error>>();

    // Spawn FLV session
    let stream_name = format!("{}/{}", room_id, media_id);
    let mut flv_session = super::httpflv::HttpFlvSession::new(
        "live".to_string(),
        stream_name,
        state.stream_hub_event_sender,
        tx,
    );

    tokio::spawn(async move {
        if let Err(e) = flv_session.run().await {
            error!("FLV session error: {}", e);
        }
    });

    // Return streaming response
    let body = Body::from_stream(tokio_stream::wrappers::UnboundedReceiverStream::new(rx));

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "video/x-flv")
        .header(header::CACHE_CONTROL, "no-cache, no-store")
        .header(header::CONNECTION, "close")
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_response())
}

/// Handle GET /live/hls/list/:media_id
/// HLS playlist endpoint (returns m3u8)
/// Requires: Extension<RoomId> from auth middleware
async fn handle_hls_list(
    Path(media_id): Path<String>,
    Extension(room_id): Extension<String>,
    State(_state): State<LiveStreamingState>,
) -> Result<Response, StatusCode> {
    // Remove .m3u8 suffix if present
    let media_id = media_id.trim_end_matches(".m3u8");

    info!(
        room_id = %room_id,
        media_id = %media_id,
        "HLS list request"
    );

    // TODO: Check if media is live
    // TODO: Get HLS channel and generate M3U8
    // For now, return placeholder

    let m3u8_content = format!(
        "#EXTM3U\n\
         #EXT-X-VERSION:3\n\
         #EXT-X-TARGETDURATION:10\n\
         #EXT-X-MEDIA-SEQUENCE:0\n\
         #EXTINF:10.0,\n\
         /api/room/movie/live/hls/data/{}/{}/segment0.ts\n",
        room_id, media_id
    );

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
        .header(header::CACHE_CONTROL, "no-cache, no-store")
        .body(Body::from(m3u8_content))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_response())
}

/// Handle GET /live/hls/data/:room_id/:media_id/:segment
/// HLS segment data endpoint (returns .ts file)
/// No authentication required (public access)
async fn handle_hls_data(
    Path((room_id, media_id, segment)): Path<(String, String, String)>,
    State(_state): State<LiveStreamingState>,
) -> Result<Response, StatusCode> {
    info!(
        room_id = %room_id,
        media_id = %media_id,
        segment = %segment,
        "HLS data request"
    );

    // TODO: Load room by ID (no auth required)
    // TODO: Get media by ID
    // TODO: Get HLS channel
    // TODO: Read TS segment data
    // For now, return not implemented

    Err(StatusCode::NOT_IMPLEMENTED)
}
