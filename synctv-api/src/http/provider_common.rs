//! Common Provider Route Utilities
//!
//! Shared functionality across all provider routes

use std::collections::HashMap;

use axum::{extract::{Path, State}, routing::get, Json, Router, response::IntoResponse};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::json;
use synctv_core::models::{Media, MediaId, RoomId};
use synctv_core::provider::{MediaProvider, PlaybackResult as ProviderPlaybackResult, ProviderContext, ProviderError};

use super::AppState;
use super::error::AppError;
use super::middleware::AuthUser;

/// Register common provider routes
///
/// Routes:
/// - GET /instances - List all available provider instances
/// - GET /`backends/:provider_type` - List available backends for a provider type
pub fn register_common_routes() -> Router<AppState> {
    Router::new()
        .route("/instances", get(list_instances))
        .route("/backends/:provider_type", get(list_backends))
}

/// List all available provider instances
async fn list_instances(_auth: AuthUser, State(state): State<AppState>) -> impl IntoResponse {
    let instances = state.provider_instance_manager.list().await;

    Json(json!({
        "instances": instances
    }))
}

/// List available backends for a given provider type (bilibili/alist/emby)
async fn list_backends(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(provider_type): Path<String>,
) -> impl IntoResponse {
    let instances = match state.provider_instance_manager.get_all_instances().await {
        Ok(all) => all
            .into_iter()
            .filter(|i| i.enabled && i.providers.iter().any(|p| p == &provider_type))
            .map(|i| i.name)
            .collect::<Vec<_>>(),
        Err(_) => vec![],
    };

    Json(json!({
        "backends": instances
    }))
}

/// Convert `ProviderError` to HTTP response
pub fn error_response(e: ProviderError) -> (StatusCode, Json<serde_json::Value>) {
    let (status, message, details) = match &e {
        ProviderError::NetworkError(msg) => (StatusCode::BAD_GATEWAY, msg.clone(), msg.clone()),
        ProviderError::ApiError(msg) => (StatusCode::BAD_GATEWAY, msg.clone(), msg.clone()),
        ProviderError::ParseError(msg) => (StatusCode::BAD_REQUEST, msg.clone(), msg.clone()),
        ProviderError::InvalidConfig(msg) => (StatusCode::BAD_REQUEST, msg.clone(), msg.clone()),
        ProviderError::NotFound => (StatusCode::NOT_FOUND, "Resource not found".to_string(), "Resource not found".to_string()),
        ProviderError::InstanceNotFound(msg) => (StatusCode::NOT_FOUND, msg.clone(), msg.clone()),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "Provider error".to_string(), e.to_string()),
    };

    let body = json!({
        "error": message,
        "details": details
    });

    (status, Json(body))
}

/// Convert String error to HTTP response (for implementation layer errors)
#[must_use] 
pub fn parse_provider_error(error_msg: &str) -> ProviderError {
    // Parse common error patterns and convert to ProviderError
    let lower = error_msg.to_lowercase();

    if lower.contains("network") || lower.contains("connection") {
        ProviderError::NetworkError(error_msg.to_string())
    } else if lower.contains("not found") {
        ProviderError::NotFound
    } else if lower.contains("parse") || lower.contains("invalid") {
        ProviderError::ParseError(error_msg.to_string())
    } else {
        // Unauthorized, authentication, or any other error
        ProviderError::ApiError(error_msg.to_string())
    }
}

/// Extract `instance_name` from query parameter
#[derive(Debug, Deserialize)]
pub struct InstanceQuery {
    #[serde(default)]
    pub instance_name: Option<String>,
}

impl InstanceQuery {
    #[must_use]
    pub fn as_deref(&self) -> Option<&str> {
        self.instance_name.as_deref()
    }
}

// ------------------------------------------------------------------
// Shared playback resolution helpers
// ------------------------------------------------------------------

/// Verify room membership, fetch the playlist, and find a specific media item.
///
/// This is the common first phase shared by all provider proxy handlers.
pub async fn resolve_media_from_playlist(
    auth: &AuthUser,
    room_id: &RoomId,
    media_id: &MediaId,
    state: &AppState,
) -> Result<Media, AppError> {
    state
        .room_service
        .check_membership(room_id, &auth.user_id)
        .await
        .map_err(|_| AppError::forbidden("Not a member of this room"))?;

    let playlist = state
        .room_service
        .get_playlist(room_id)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get playlist: {e}"))?;

    let media = playlist
        .into_iter()
        .find(|m| m.id == *media_id)
        .ok_or_else(|| anyhow::anyhow!("Media not found in playlist"))?;

    Ok(media)
}

/// Resolve a playback URL and headers from a `MediaProvider`.
///
/// Performs the full flow: membership check -> playlist lookup -> find media ->
/// `generate_playback` -> extract first URL + headers from the default mode.
///
/// Used by alist and emby proxy handlers.
pub async fn resolve_provider_playback_url(
    auth: &AuthUser,
    room_id: &RoomId,
    media_id: &MediaId,
    state: &AppState,
    provider: &dyn MediaProvider,
) -> Result<(String, HashMap<String, String>), AppError> {
    let media = resolve_media_from_playlist(auth, room_id, media_id, state).await?;

    let ctx = ProviderContext::new("synctv")
        .with_user_id(auth.user_id.as_str())
        .with_room_id(room_id.as_str());

    let playback_result = provider
        .generate_playback(&ctx, &media.source_config)
        .await
        .map_err(|e| anyhow::anyhow!("{} generate_playback failed: {e}", provider.name()))?;

    let default_mode = &playback_result.default_mode;
    let playback_info = playback_result
        .playback_infos
        .get(default_mode)
        .ok_or_else(|| anyhow::anyhow!("Default playback mode not found"))?;

    let url = playback_info
        .urls
        .first()
        .ok_or_else(|| anyhow::anyhow!("No URLs in playback info"))?;

    Ok((url.clone(), playback_info.headers.clone()))
}

/// Resolve the full `PlaybackResult` from a `MediaProvider`.
///
/// Performs the full flow: membership check -> playlist lookup -> find media ->
/// `generate_playback`.
///
/// Used by bilibili proxy handlers that need access to the complete result
/// (DASH data, multiple modes, subtitles).
pub async fn resolve_provider_playback_result(
    auth: &AuthUser,
    room_id: &RoomId,
    media_id: &MediaId,
    state: &AppState,
    provider: &dyn MediaProvider,
) -> Result<ProviderPlaybackResult, AppError> {
    let media = resolve_media_from_playlist(auth, room_id, media_id, state).await?;

    let ctx = ProviderContext::new("synctv")
        .with_user_id(auth.user_id.as_str())
        .with_room_id(room_id.as_str());

    provider
        .generate_playback(&ctx, &media.source_config)
        .await
        .map_err(|e| anyhow::anyhow!("{} generate_playback failed: {e}", provider.name()).into())
}
