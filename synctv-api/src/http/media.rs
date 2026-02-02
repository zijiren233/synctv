//! Media/Playlist management HTTP API
//!
//! Handles all playlist operations including adding, editing, removing,
//! reordering, and retrieving media items.

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::http::{AppState, AppResult};
use synctv_core::{
    models::{MediaId, RoomId, UserId},
};

/// Get current media in playlist
#[axum::debug_handler]
pub async fn get_playing_media(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let room_id = RoomId::from_string(room_id);
    let media = state
        .room_service
        .get_playing_media(&room_id)
        .await?;

    let response = media.map(|m| media_to_response(&m));
    Ok(Json(response))
}

/// Get playlist for a room
#[axum::debug_handler]
pub async fn get_playlist(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Query(params): Query<PlaylistQuery>,
) -> AppResult<impl IntoResponse> {
    let room_id = RoomId::from_string(room_id);

    let page = params.page.unwrap_or(1);
    let page_size = params.page_size.unwrap_or(50);

    let (playlist, total) = state
        .room_service
        .get_playlist_paginated(&room_id, page, page_size)
        .await?;

    Ok(Json(PlaylistResponse {
        items: playlist.into_iter().map(|m| media_to_response(&m)).collect(),
        total,
        page,
        page_size,
    }))
}

/// Add a single media item to playlist
#[axum::debug_handler]
pub async fn add_media(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<AddMediaHttpRequest>,
) -> AppResult<impl IntoResponse> {
    let room_id = RoomId::from_string(room_id);
    let user_id = UserId::from_string(req.user_id);

    // Build source_config from URL
    let source_config = serde_json::json!({
        "url": req.url
    });

    // Determine provider instance name (empty for direct URL)
    let provider_instance_name = if req.provider.is_empty() {
        String::new()
    } else {
        req.provider.clone()
    };

    // Extract title from URL or use provided title
    let title = if req.title.is_empty() {
        req.url.split('/').last().unwrap_or("Unknown").to_string()
    } else {
        req.title
    };

    let media = state
        .room_service
        .add_media(room_id, user_id, provider_instance_name, source_config, title)
        .await?;

    Ok(Json(media_to_response(&media)))
}

/// Add multiple media items to playlist
#[axum::debug_handler]
pub async fn add_media_batch(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<AddMediaBatchHttpRequest>,
) -> AppResult<impl IntoResponse> {
    let room_id = RoomId::from_string(room_id);
    let user_id = UserId::from_string(req.user_id);

    let mut media_items = Vec::new();
    for item in req.items {
        // Build source_config from URL
        let source_config = serde_json::json!({
            "url": item.url
        });

        // Determine provider instance name (empty for direct URL)
        let provider_instance_name = if item.provider.is_empty() {
            String::new()
        } else {
            item.provider.clone()
        };

        // Extract title from URL or use provided title
        let title = if item.title.is_empty() {
            item.url.split('/').last().unwrap_or("Unknown").to_string()
        } else {
            item.title
        };

        let media = state
            .room_service
            .add_media(room_id.clone(), user_id.clone(), provider_instance_name, source_config, title)
            .await?;

        media_items.push(media);
    }

    Ok(Json(media_items.into_iter().map(|m| media_to_response(&m)).collect::<Vec<_>>()))
}

/// Edit media item
#[axum::debug_handler]
pub async fn edit_media(
    State(state): State<AppState>,
    Path((room_id, media_id)): Path<(String, String)>,
    Json(req): Json<EditMediaHttpRequest>,
) -> AppResult<impl IntoResponse> {
    let room_id = RoomId::from_string(room_id);
    let media_id = MediaId::from_string(media_id);
    let user_id = UserId::from_string(req.user_id);

    let media = state
        .room_service
        .edit_media(room_id, user_id, media_id, req.title, req.metadata)
        .await?;

    Ok(Json(media_to_response(&media)))
}

/// Delete media item
#[axum::debug_handler]
pub async fn delete_media(
    State(state): State<AppState>,
    Path((room_id, media_id)): Path<(String, String)>,
    Json(req): Json<UserIdRequest>,
) -> AppResult<impl IntoResponse> {
    let room_id = RoomId::from_string(room_id);
    let media_id = MediaId::from_string(media_id);
    let user_id = UserId::from_string(req.user_id);

    state
        .room_service
        .remove_media(room_id, user_id, media_id)
        .await?;

    Ok(Json(serde_json::json!({"success": true})))
}

/// Swap positions of two media items
#[axum::debug_handler]
pub async fn swap_media(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<SwapMediaRequest>,
) -> AppResult<impl IntoResponse> {
    let room_id = RoomId::from_string(room_id);
    let media_id1 = MediaId::from_string(req.media_id1);
    let media_id2 = MediaId::from_string(req.media_id2);
    let user_id = UserId::from_string(req.user_id);

    state
        .room_service
        .swap_media(room_id, user_id, media_id1, media_id2)
        .await?;

    Ok(Json(serde_json::json!({"success": true})))
}

/// Clear entire playlist
#[axum::debug_handler]
pub async fn clear_playlist(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<UserIdRequest>,
) -> AppResult<impl IntoResponse> {
    let room_id = RoomId::from_string(room_id);
    let user_id = UserId::from_string(req.user_id);

    let count = state
        .room_service
        .clear_playlist(room_id, user_id)
        .await?;

    Ok(Json(serde_json::json!({"success": true, "count": count})))
}

/// Set current playing media
#[axum::debug_handler]
pub async fn set_playing_media(
    State(state): State<AppState>,
    Path((room_id, media_id)): Path<(String, String)>,
    Json(req): Json<UserIdRequest>,
) -> AppResult<impl IntoResponse> {
    let room_id = RoomId::from_string(room_id);
    let media_id = MediaId::from_string(media_id);
    let user_id = UserId::from_string(req.user_id);

    let state = state
        .room_service
        .set_playing_media(room_id, user_id, media_id)
        .await?;

    Ok(Json(playback_state_to_response(&state)))
}

// ============== Request/Response Types ==============

#[derive(Debug, Deserialize)]
pub struct PlaylistQuery {
    pub page: Option<i32>,
    pub page_size: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct PlaylistResponse {
    pub items: Vec<MediaResponse>,
    pub total: i64,
    pub page: i32,
    pub page_size: i32,
}

#[derive(Debug, Deserialize)]
pub struct AddMediaHttpRequest {
    pub user_id: String,
    pub url: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct AddMediaBatchHttpRequest {
    pub user_id: String,
    pub items: Vec<AddMediaItem>,
}

#[derive(Debug, Deserialize)]
pub struct AddMediaItem {
    pub url: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub title: String,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct EditMediaHttpRequest {
    pub user_id: String,
    pub title: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UserIdRequest {
    pub user_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SwapMediaRequest {
    pub user_id: String,
    pub media_id1: String,
    pub media_id2: String,
}

#[derive(Debug, Serialize)]
pub struct MediaResponse {
    pub id: String,
    pub room_id: String,
    pub name: String,
    pub url: String,
    pub provider: String,
    pub position: i32,
    pub added_at: i64,
    pub added_by: String,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct PlaybackStateResponse {
    pub is_playing: bool,
    pub current_time: f64,
    pub playback_rate: f64,
    pub playing_media_id: Option<String>,
}

// ============== Helper Functions ==============

fn media_to_response(media: &synctv_core::models::Media) -> MediaResponse {
    // Try to extract URL from source_config
    let url = media.source_config.get("url")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| String::new());

    MediaResponse {
        id: media.id.as_str().to_string(),
        room_id: media.room_id.as_str().to_string(),
        name: media.name.clone(),
        url,
        provider: media.source_provider.clone(),
        position: media.position,
        added_at: media.added_at.timestamp(),
        added_by: media.creator_id.as_str().to_string(),
        metadata: media.metadata.clone(),
    }
}

fn playback_state_to_response(state: &synctv_core::models::RoomPlaybackState) -> PlaybackStateResponse {
    PlaybackStateResponse {
        is_playing: state.is_playing,
        current_time: state.current_time,
        playback_rate: state.playback_rate,
        playing_media_id: state.playing_media_id.map(|id| id.as_str().to_string()),
    }
}
