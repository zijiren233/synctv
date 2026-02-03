// Live streaming HTTP endpoints
//!
//! Provides FLV and HLS streaming endpoints for live video.
//!
//! Architecture:
//! - Uses synctv-stream's LiveStreamingInfrastructure via AppState
//! - Implements lazy-load FLV streaming
//! - Implements HLS playlist generation and segment serving
//!
//! Endpoints (matching synctv-go paths):
//! - GET /api/room/movie/live/flv/:media_id - FLV streaming
//! - GET /api/room/movie/live/hls/list/:media_id - HLS playlist
//! - GET /api/room/movie/live/hls/data/:room_id/:media_id/:segment.ts - HLS segment

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use bytes::Bytes;
use serde::Deserialize;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{debug, error, info, warn};

use crate::http::{AppState, AppResult};
use synctv_stream::api::{FlvStreamingApi, HlsStreamingApi};

/// Query parameters for live streaming endpoints
#[derive(Debug, Deserialize)]
pub struct LiveQuery {
    /// Room ID (required for most endpoints)
    room_id: Option<String>,
    /// Authentication token
    token: Option<String>,
}

/// Create live streaming router
///
/// Uses AppState for state (live_streaming_infrastructure must be configured)
///
/// Routes match synctv-go path patterns:
/// - /api/room/movie/live/flv/:media_id
/// - /api/room/movie/live/hls/list/:media_id
/// - /api/room/movie/live/hls/data/:room_id/:media_id/:segment.ts
pub fn create_live_router() -> Router<AppState> {
    Router::new()
        // FLV streaming endpoint
        .route("/flv/:media_id.flv", get(handle_flv_stream))
        // HLS playlist endpoint
        .route("/hls/list/:media_id", get(handle_hls_playlist))
        // HLS playlist with .m3u8 extension
        .route("/hls/list/:media_id.m3u8", get(handle_hls_playlist))
        // HLS segment endpoint
        .route(
            "/hls/data/:room_id/:media_id/:segment.ts",
            get(handle_hls_segment),
        )
        // HLS segment with .png extension (disguised mode)
        .route(
            "/hls/data/:room_id/:media_id/:segment.png",
            get(handle_hls_segment_disguised),
        )
}

/// Handle FLV streaming request
///
/// GET /api/room/movie/live/flv/:media_id?roomId=:room_id&token=:token
///
/// Streaming endpoint for HTTP-FLV live streaming.
/// Creates a lazy-load pull stream on first request.
///
/// # Response
/// Returns streaming FLV data with `video/x-flv` content type.
async fn handle_flv_stream(
    Path(media_id): Path<String>,
    Query(params): Query<LiveQuery>,
    State(state): State<AppState>,
) -> AppResult<Response> {
    let room_id = params
        .room_id
        .ok_or_else(|| AppError::bad_request("roomId query parameter is required"))?;

    info!(room_id = %room_id, media_id = %media_id, "FLV streaming request");

    // Get live streaming infrastructure
    let infrastructure = state
        .live_streaming_infrastructure
        .as_ref()
        .ok_or_else(|| AppError::internal_server_error("Live streaming not configured"))?;

    // Create FLV streaming session with lazy-load pull
    let rx = FlvStreamingApi::create_session_with_pull(infrastructure, &room_id, &media_id)
        .await
        .map_err(|e| AppError::internal_server_error(format!("Failed to create FLV session: {}", e)))?;

    // Convert to streaming response
    let body = Body::from_stream(UnboundedReceiverStream::new(rx));

    info!(room_id = %room_id, media_id = %media_id, "FLV streaming started");

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "video/x-flv")
        .header(header::CACHE_CONTROL, "no-cache, no-store")
        .header("X-Accel-Buffering", "no")
        .header(header::CONNECTION, "keep-alive")
        .body(body)
        .map_err(|_| AppError::internal_server_error("Failed to build response"))?
        .into_response())
}

/// Handle HLS playlist request
///
/// GET /api/room/movie/live/hls/list/:media_id?roomId=:room_id&token=:token
///
/// Returns M3U8 playlist with references to TS segments.
/// Creates a lazy-load pull stream on first request.
///
/// # Response
/// Returns M3U8 playlist with `application/vnd.apple.mpegurl` content type.
async fn handle_hls_playlist(
    Path(media_id): Path<String>,
    Query(params): Query<LiveQuery>,
    State(state): State<AppState>,
) -> AppResult<Response> {
    let room_id = params
        .room_id
        .ok_or_else(|| AppError::bad_request("roomId query parameter is required"))?;

    info!(room_id = %room_id, media_id = %media_id, "HLS playlist request");

    let infrastructure = state
        .live_streaming_infrastructure
        .as_ref()
        .ok_or_else(|| AppError::internal_server_error("Live streaming not configured"))?;

    // Build segment URL base following synctv-go pattern
    // TS segments are at: /api/room/movie/live/hls/data/{roomId}/{movieId}/
    let segment_url_base = format!("/api/room/movie/live/hls/data/{}/{}/", room_id, media_id);

    // Generate HLS playlist with simple URL format
    let playlist = HlsStreamingApi::generate_playlist_with_pull_simple(
        infrastructure,
        &room_id,
        &media_id,
        &segment_url_base,
    )
    .await
    .map_err(|e| AppError::internal_server_error(format!("Failed to generate HLS playlist: {}", e)))?;

    debug!(
        room_id = %room_id,
        media_id = %media_id,
        "Generated HLS playlist"
    );

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
        .header(header::CACHE_CONTROL, "no-cache, no-store")
        .body(playlist)
        .map_err(|_| AppError::internal_server_error("Failed to build response"))?
        .into_response())
}

/// Handle HLS segment request
///
/// GET /api/room/movie/live/hls/data/:room_id/:media_id/:segment.ts
///
/// Serves individual HLS TS segments.
///
/// # Response
/// Returns TS segment data with `video/mp2t` content type.
async fn handle_hls_segment(
    Path((room_id, media_id, segment)): Path<(String, String, String)>,
    State(state): State<AppState>,
) -> AppResult<Response> {
    // Remove .ts suffix if present
    let segment_name = segment.trim_end_matches(".ts");

    debug!(
        room_id = %room_id,
        media_id = %media_id,
        segment = %segment_name,
        "HLS segment request"
    );

    let infrastructure = state
        .live_streaming_infrastructure
        .as_ref()
        .ok_or_else(|| AppError::internal_server_error("Live streaming not configured"))?;

    // Get segment data
    match HlsStreamingApi::get_segment(infrastructure, &room_id, &media_id, segment_name).await {
        Ok(data) => {
            debug!(
                room_id = %room_id,
                media_id = %media_id,
                segment = %segment_name,
                size = data.len(),
                "Serving HLS segment"
            );

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "video/mp2t")
                .header(header::CACHE_CONTROL, "public, max-age=90")
                .header("X-Accel-Buffering", "no")
                .body(Body::from(data))
                .map_err(|_| AppError::internal_server_error("Failed to build response"))?
                .into_response())
        }
        Err(e) => {
            warn!(
                room_id = %room_id,
                media_id = %media_id,
                segment = %segment_name,
                error = %e,
                "HLS segment not found"
            );
            Err(AppError::not_found("HLS segment not found"))
        }
    }
}

/// Handle HLS segment request (disguised as PNG)
///
/// GET /api/room/movie/live/hls/data/:room_id/:media_id/:segment.png
///
/// Serves TS segments disguised as PNG images (TSDisguisedAsPng feature).
/// Adds a PNG header to TS data to bypass certain filters.
///
/// # Response
/// Returns TS segment data with PNG header, `image/png` content type.
async fn handle_hls_segment_disguised(
    Path((room_id, media_id, segment)): Path<(String, String, String)>,
    State(state): State<AppState>,
) -> AppResult<Response> {
    // Remove .png suffix
    let segment_name = segment.trim_end_matches(".png");

    debug!(
        room_id = %room_id,
        media_id = %media_id,
        segment = %segment_name,
        "HLS segment request (disguised as PNG)"
    );

    let infrastructure = state
        .live_streaming_infrastructure
        .as_ref()
        .ok_or_else(|| AppError::internal_server_error("Live streaming not configured"))?;

    // Get segment data
    match HlsStreamingApi::get_segment(infrastructure, &room_id, &media_id, segment_name).await {
        Ok(ts_data) => {
            // PNG header (89 50 4E 47 0D 0A 1A 0A + IHDR chunk)
            // Minimal PNG: 8x8 pixel image
            let png_header = [
                0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
                0x00, 0x00, 0x00, 0x0D, // Length: 13
                0x49, 0x48, 0x44, 0x52, // IHDR
                0x00, 0x00, 0x00, 0x08, // Width: 8
                0x00, 0x00, 0x00, 0x08, // Height: 8
                0x08, 0x02, 0x00, 0x00, 0x00, // Bit depth: 8, Color type: 2 (RGB), etc.
                0x90, 0x77, 0x53, // CRC
                0xDE, // Start of IDAT chunk (followed by actual TS data)
            ];

            let mut disguised_data = Vec::with_capacity(png_header.len() + ts_data.len());
            disguised_data.extend_from_slice(&png_header);
            disguised_data.extend_from_slice(&ts_data);

            debug!(
                room_id = %room_id,
                media_id = %media_id,
                segment = %segment_name,
                original_size = ts_data.len(),
                disguised_size = disguised_data.len(),
                "Serving HLS segment (disguised as PNG)"
            );

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/png")
                .header(header::CACHE_CONTROL, "public, max-age=90")
                .header("X-Accel-Buffering", "no")
                .body(Body::from(disguised_data))
                .map_err(|_| AppError::internal_server_error("Failed to build response"))?
                .into_response())
        }
        Err(e) => {
            warn!(
                room_id = %room_id,
                media_id = %media_id,
                segment = %segment_name,
                error = %e,
                "HLS segment not found"
            );
            Err(AppError::not_found("HLS segment not found"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_live_router() {
        let router = create_live_router();
        // Router should be created successfully
        assert_eq!(router.routes().len(), 5);
    }
}
