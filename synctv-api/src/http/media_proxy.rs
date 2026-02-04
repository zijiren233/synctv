//! Media proxy HTTP endpoint
//!
//! Proxies video streams from external providers (Bilibili, Alist, Emby)
//! to avoid CORS issues and hide source URLs from clients

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use crate::http::error::AppResult;
use crate::http::middleware::AuthUser;
use crate::http::AppState;
use synctv_core::models::{MediaId, RoomId};

/// GET /`api/rooms/:room_id/media/:media_id/proxy` - Proxy video stream from provider
pub async fn proxy_media_stream(
    _auth: AuthUser,
    Path((room_id, media_id)): Path<(String, String)>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let room_id = RoomId::from_string(room_id);
    let media_id = MediaId::from_string(media_id);

    // Get the playlist to find the media
    let playlist = state
        .room_service
        .get_playlist(&room_id)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get playlist: {e}"))?;

    // Find the media in the playlist
    let media = playlist
        .iter()
        .find(|m| m.id == media_id)
        .ok_or_else(|| anyhow::anyhow!("Media not found in playlist"))?;

    // Extract the original URL from metadata
    let original_url = media
        .metadata
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Media URL not found in metadata"))?;

    tracing::info!("Proxying media request for URL: {}", original_url);

    // Use reqwest to fetch the content
    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {e}"))?;

    // Build the proxy request
    let mut request = client.get(original_url);

    // Copy relevant headers from the original request
    for (name, value) in &headers {
        // Skip headers that shouldn't be proxied
        if matches!(
            name.as_str(),
            "host" | "connection" | "accept-encoding" | "content-length" | "transfer-encoding"
        ) {
            continue;
        }

        if let Ok(value) = value.to_str() {
            request = request.header(name.as_str(), value);
        }
    }

    // Set User-Agent to mimic a browser
    request = request.header(
        "User-Agent",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
    );

    // Set Referer if not present (some providers require this)
    if let Ok(parsed_url) = url::Url::parse(original_url) {
        let referer = format!(
            "{}://{}{}",
            parsed_url.scheme(),
            parsed_url.host_str().unwrap_or(""),
            parsed_url.path()
        );
        request = request.header("Referer", referer);
    }

    // Execute the proxy request
    let proxy_response = request
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Proxy request failed: {e}"))?;

    let status = proxy_response.status();
    let response_headers = proxy_response.headers().clone();

    // Get the response body bytes
    let body_bytes = proxy_response
        .bytes()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read response body: {e}"))?;

    // Build the response
    let mut builder = Response::builder().status(status);

    // Copy relevant headers from the proxy response
    for (name, value) in &response_headers {
        // Skip headers that shouldn't be forwarded
        if matches!(
            name.as_str(),
            "connection" | "transfer-encoding" | "content-encoding" | "content-length"
        ) {
            continue;
        }

        if let Ok(value_str) = value.to_str() {
            builder = builder.header(name.as_str(), value_str);
        }
    }

    // Set CORS headers
    builder = builder.header("Access-Control-Allow-Origin", "*");
    builder = builder.header("Access-Control-Allow-Methods", "GET, HEAD, OPTIONS");
    builder = builder.header("Access-Control-Allow-Headers", "*");
    builder = builder.header("Access-Control-Expose-Headers", "*");

    // Set cache control for streaming
    builder = builder.header("Cache-Control", "no-cache");
    builder = builder.header("Pragma", "no-cache");

    let response = builder
        .body(Body::from(body_bytes))
        .map_err(|e| anyhow::anyhow!("Failed to build response: {e}"))?;

    Ok(response)
}

/// OPTIONS /`api/rooms/:room_id/media/:media_id/proxy` - CORS preflight
pub async fn proxy_media_stream_options() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            ("Access-Control-Allow-Origin", "*"),
            ("Access-Control-Allow-Methods", "GET, HEAD, OPTIONS"),
            ("Access-Control-Allow-Headers", "*"),
            ("Access-Control-Max-Age", "86400"),
        ],
    )
}

/// Create the media proxy router
pub fn create_media_proxy_router() -> axum::Router<AppState> {
    axum::Router::new()
        .route(
            "/api/rooms/:room_id/media/:media_id/proxy",
            axum::routing::get(proxy_media_stream).options(proxy_media_stream_options),
        )
}
