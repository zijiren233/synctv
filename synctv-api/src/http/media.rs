//! Media/Playlist management HTTP API
//!
//! Handles all playlist operations including adding, editing, removing,
//! reordering, and retrieving media items.
//!
//! Uses gRPC proto types for all requests/responses to maintain consistency.

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Json},
};
use serde::Deserialize;

use crate::http::{AppState, AppResult};
use crate::proto::client::{Media, GetPlaylistResponse};
use crate::impls::client::media_to_proto;
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

    let response = media.map(|m| media_to_proto(&m));
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

    // Get root playlist info
    let root_playlist = state
        .room_service
        .playlist_service()
        .get_root_playlist(&room_id)
        .await?;

    // Get paginated media from playlist
    let (media_list_db, total) = state
        .room_service
        .get_playlist_paginated(&room_id, page, page_size)
        .await?;

    // Convert to proto format
    let media_list: Vec<Media> = media_list_db.into_iter().map(|m| media_to_proto(&m)).collect();

    // Convert playlist to proto
    let playlist_proto = crate::proto::client::Playlist {
        id: root_playlist.id.as_str().to_string(),
        room_id: root_playlist.room_id.as_str().to_string(),
        name: root_playlist.name.clone(),
        parent_id: root_playlist.parent_id.as_ref().map(|id| id.as_str().to_string()).unwrap_or_default(),
        position: root_playlist.position,
        is_folder: true, // Root playlist is always a folder
        is_dynamic: root_playlist.source_provider.is_some(),
        item_count: total as i32,
        created_at: root_playlist.created_at.timestamp(),
        updated_at: root_playlist.updated_at.timestamp(),
    };

    Ok(Json(GetPlaylistResponse {
        playlist: Some(playlist_proto),
        media: media_list,
        total: total as i32,
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
        req.url.split('/').next_back().unwrap_or("Unknown").to_string()
    } else {
        req.title
    };

    let media = state
        .room_service
        .add_media(room_id, user_id, provider_instance_name, source_config, title)
        .await?;

    Ok(Json(media_to_proto(&media)))
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
            item.url.split('/').next_back().unwrap_or("Unknown").to_string()
        } else {
            item.title
        };

        let media = state
            .room_service
            .add_media(room_id.clone(), user_id.clone(), provider_instance_name, source_config, title)
            .await?;

        media_items.push(media);
    }

    Ok(Json(media_items.into_iter().map(|m| media_to_proto(&m)).collect::<Vec<_>>()))
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
        .edit_media(room_id, user_id, media_id, req.title)
        .await?;

    Ok(Json(media_to_proto(&media)))
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

    // Convert to proto PlaybackState
    let proto_state = crate::proto::client::PlaybackState {
        room_id: state.room_id.as_str().to_string(),
        playing_media_id: state.playing_media_id.map(|id| id.as_str().to_string()).unwrap_or_default(),
        position: state.position,
        speed: state.speed,
        is_playing: state.is_playing,
        updated_at: state.updated_at.timestamp(),
        version: 0,
    };

    Ok(Json(crate::proto::client::GetPlaybackStateResponse {
        playback_state: Some(proto_state),
    }))
}

// ============== Request/Response Types ==============

#[derive(Debug, Deserialize)]
pub struct PlaylistQuery {
    pub page: Option<i32>,
    pub page_size: Option<i32>,
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
