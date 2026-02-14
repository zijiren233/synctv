//! Media/Playlist management HTTP API
//!
//! Handles playlist browsing operations.
//!
//! Uses gRPC proto types for all requests/responses to maintain consistency.

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Json},
};

use crate::http::{AppState, AppResult, middleware::AuthUser};
use crate::proto::client::{Media, ListPlaylistResponse};
use crate::impls::client::media_to_proto;
use synctv_core::models::RoomId;

/// Get current media in playlist
#[axum::debug_handler]
pub async fn get_playing_media(
    _auth: AuthUser,
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
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Query(params): Query<PlaylistQuery>,
) -> AppResult<impl IntoResponse> {
    let room_id = RoomId::from_string(room_id);

    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(50).clamp(1, 100);

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

    Ok(Json(ListPlaylistResponse {
        playlist: Some(playlist_proto),
        media: media_list,
        total: total as i32,
    }))
}

/// List dynamic playlist items
///
/// GET /`api/rooms/:room_id/playlists/:playlist_id/items`
#[axum::debug_handler]
pub async fn list_playlist_items(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((room_id, playlist_id)): Path<(String, String)>,
    Query(params): Query<ListPlaylistItemsQuery>,
) -> AppResult<impl IntoResponse> {
    let req = crate::proto::client::ListPlaylistItemsRequest {
        playlist_id: playlist_id.clone(),
        relative_path: params.relative_path.unwrap_or_default(),
        page: params.page.unwrap_or(0).max(0),
        page_size: params.page_size.unwrap_or(50).clamp(1, 100),
    };

    let response = state
        .client_api
        .list_playlist_items(auth.user_id.as_str(), &room_id, req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Set current playing media
#[axum::debug_handler]
pub async fn set_playing_media(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((room_id, media_id)): Path<(String, String)>,
) -> AppResult<impl IntoResponse> {
    let room_id = RoomId::from_string(room_id);
    let media_id = synctv_core::models::MediaId::from_string(media_id);

    let state = state
        .room_service
        .set_playing_media(room_id, auth.user_id, media_id)
        .await?;

    // Convert to proto PlaybackState
    let proto_state = crate::proto::client::PlaybackState {
        room_id: state.room_id.as_str().to_string(),
        playing_media_id: state.playing_media_id.map(|id| id.as_str().to_string()).unwrap_or_default(),
        current_time: state.current_time,
        speed: state.speed,
        is_playing: state.is_playing,
        updated_at: state.updated_at.timestamp(),
        version: 0,
        playing_playlist_id: state.playing_playlist_id.map(|id| id.as_str().to_string()).unwrap_or_default(),
        relative_path: state.relative_path,
    };

    Ok(Json(crate::proto::client::GetPlaybackStateResponse {
        playback_state: Some(proto_state),
    }))
}

// ============== Request/Response Types ==============

#[derive(Debug, serde::Deserialize)]
pub struct PlaylistQuery {
    pub page: Option<i32>,
    pub page_size: Option<i32>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ListPlaylistItemsQuery {
    pub relative_path: Option<String>,
    pub page: Option<i32>,
    pub page_size: Option<i32>,
}
