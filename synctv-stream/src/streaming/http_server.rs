// Unified HTTP server for FLV and HLS streaming
//
// Provides both FLV and HLS endpoints on a single axum server:
// - GET /live/{media_id}.flv - HTTP-FLV lazy-load streaming
// - GET /hls/{media_id}.m3u8 - HLS master playlist
// - GET /hls/{media_id}/{segment}.ts - HLS segments
//
// Architecture benefits:
// - Single HTTP port (default: 8080)
// - Shared axum infrastructure
// - Unified middleware (CORS, logging, metrics)
// - Lower resource usage

use crate::{
    relay::registry::StreamRegistry,
    streaming::pull_manager::PullStreamManager,
};
use axum::{
    Router,
    routing::get,
    extract::{Path, State},
    response::{Response, IntoResponse},
    http::{StatusCode, header},
};
use tracing as log;
use std::sync::Arc;

#[derive(Clone)]
pub struct StreamingHttpState {
    pub registry: StreamRegistry,
    pub pull_manager: Arc<PullStreamManager>,
}

/// Create unified HTTP router for FLV and HLS
pub fn create_streaming_router(state: StreamingHttpState) -> Router {
    Router::new()
        // HTTP-FLV endpoints (lazy-load)
        // URL structure: /live/{room_id}/{media_id}.flv
        .route("/live/:room_id/:media_id.flv", get(handle_flv_request))
        // HLS endpoints
        // URL structure: /hls/{room_id}/{media_id}.m3u8 and /hls/{room_id}/{media_id}/{segment}.ts
        .route("/hls/:room_id/:media_id.m3u8", get(handle_hls_playlist))
        .route("/hls/:room_id/:media_id/:segment", get(handle_hls_segment))
        // Health check
        .route("/health", get(health_check))
        .with_state(state)
}

/// HTTP-FLV lazy-load handler
async fn handle_flv_request(
    Path((room_id, media_id)): Path<(String, String)>,
    State(mut state): State<StreamingHttpState>,
) -> Result<Response, StatusCode> {
    log::info!("FLV request for room: {}, media: {}", room_id, media_id);

    // 1. Query Redis for publisher node
    let publisher_info = match state.registry.get_publisher(&room_id, &media_id).await {
        Ok(Some(info)) => info,
        Ok(None) => {
            log::warn!("No publisher found for room {} / media {}", room_id, media_id);
            return Err(StatusCode::NOT_FOUND);
        }
        Err(e) => {
            log::error!("Failed to query publisher for room {} / media {}: {}", room_id, media_id, e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    // 2. Lazy-load: Get or create pull stream
    let _pull_stream = match state.pull_manager
        .get_or_create_pull_stream(&media_id, &publisher_info.node_id)
        .await
    {
        Ok(stream) => stream,
        Err(e) => {
            log::error!("Failed to create pull stream for media {}: {}", media_id, e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    log::info!("Serving FLV stream for media: {}", media_id);

    // 3. Return streaming response
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "video/x-flv")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("X-Pull-Mode", "lazy-load")
        .header("X-Publisher-Node", publisher_info.node_id.as_str())
        .body("FLV stream - TODO: implement subscribe_flv()".to_string())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(response.into_response())
}

/// HLS master playlist handler
async fn handle_hls_playlist(
    Path((room_id, media_id)): Path<(String, String)>,
    State(mut state): State<StreamingHttpState>,
) -> Result<Response, StatusCode> {
    log::info!("HLS playlist request for room: {}, media: {}", room_id, media_id);

    // 1. Query Redis for publisher
    let publisher_info = match state.registry.get_publisher(&room_id, &media_id).await {
        Ok(Some(info)) => info,
        Ok(None) => return Err(StatusCode::NOT_FOUND),
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    // 2. Lazy-load pull stream
    let _pull_stream = match state.pull_manager
        .get_or_create_pull_stream(&media_id, &publisher_info.node_id)
        .await
    {
        Ok(stream) => stream,
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    // 3. Generate HLS playlist
    let playlist = "#EXTM3U\n\
         #EXT-X-VERSION:3\n\
         #EXT-X-TARGETDURATION:10\n\
         #EXT-X-MEDIA-SEQUENCE:0\n\
         # TODO: Implement HLS segment generation\n".to_string();

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(playlist)
        .unwrap()
        .into_response())
}

/// HLS segment handler
async fn handle_hls_segment(
    Path((room_id, media_id, segment)): Path<(String, String, String)>,
) -> Result<Response, StatusCode> {
    log::info!("HLS segment request: room {} / media {} / segment {}", room_id, media_id, segment);

    // TODO: Implement segment retrieval from storage
    Err(StatusCode::NOT_IMPLEMENTED)
}

/// Health check endpoint
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}
