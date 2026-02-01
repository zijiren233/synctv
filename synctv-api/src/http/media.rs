//! Media/Playlist management HTTP API
//!
//! Handles all playlist operations including adding, editing, removing,
//! reordering, and retrieving media items.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::http::{AppState, AppResult};
use synctv_core::{
    models::{MediaId, ProviderType, RoomId},
    service::media::{AddMediaRequest, EditMediaRequest, MediaService},
};

/// Get current media in playlist
#[axum::debug_handler]
pub async fn get_current_media(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let room_id = RoomId::from_string(room_id);
    let media = state
        .room_service
        .media_service()
        .get_current_media(&room_id)
        .await?;

    Ok(Json(media))
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
        .media_service()
        .get_playlist_paginated(&room_id, page, page_size)
        .await?;

    Ok(Json(PlaylistResponse {
        items: playlist,
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
    let user_id = synctv_core::models::UserId::from_string(req.user_id);

    let media = state
        .room_service
        .media_service()
        .add_media(
            room_id,
            user_id,
            req.url,
            req.provider,
            req.title,
        )
        .await?;

    Ok(Json(media))
}

/// Add multiple media items to playlist
#[axum::debug_handler]
pub async fn add_media_batch(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<AddMediaBatchHttpRequest>,
) -> AppResult<impl IntoResponse> {
    let room_id = RoomId::from_string(room_id);

    let items: Vec<AddMediaRequest> = req
        .items
        .into_iter()
        .map(|item| AddMediaRequest {
            url: item.url,
            provider: item.provider,
            title: item.title,
            metadata: item.metadata,
        })
        .collect();

    let media_items = state
        .room_service
        .media_service()
        .add_media_batch(room_id, synctv_core::models::UserId::from_string(req.user_id), items)
        .await?;

    Ok(Json(media_items))
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

    let edit_req = EditMediaRequest {
        media_id,
        url: req.url,
        title: req.title,
        metadata: req.metadata,
    };

    let media = state
        .room_service
        .media_service()
        .edit_media(room_id, synctv_core::models::UserId::from_string(req.user_id), edit_req)
        .await?;

    Ok(Json(media))
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

    state
        .room_service
        .media_service()
        .remove_media(room_id, synctv_core::models::UserId::from_string(req.user_id), media_id)
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

    state
        .room_service
        .media_service()
        .swap_media(room_id, synctv_core::models::UserId::from_string(req.user_id), media_id1, media_id2)
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

    let count = state
        .room_service
        .media_service()
        .clear_playlist(room_id, synctv_core::models::UserId::from_string(req.user_id))
        .await?;

    Ok(Json(serde_json::json!({"success": true, "count": count})))
}

/// Set current playing media
#[axum::debug_handler]
pub async fn set_current_media(
    State(state): State<AppState>,
    Path((room_id, media_id)): Path<(String, String)>,
    Json(req): Json<UserIdRequest>,
) -> AppResult<impl IntoResponse> {
    let room_id = RoomId::from_string(room_id);
    let media_id = MediaId::from_string(media_id);

    let media = state
        .room_service
        .media_service()
        .set_current_media(room_id, synctv_core::models::UserId::from_string(req.user_id), media_id)
        .await?;

    Ok(Json(media))
}

// ============== Request/Response Types ==============

#[derive(Debug, Deserialize)]
pub struct PlaylistQuery {
    pub page: Option<i32>,
    pub page_size: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct PlaylistResponse {
    pub items: Vec<synctv_core::models::Media>,
    pub total: i64,
    pub page: i32,
    pub page_size: i32,
}

#[derive(Debug, Deserialize)]
pub struct AddMediaHttpRequest {
    pub user_id: String,
    pub url: String,
    #[serde(deserialize_with = "deserialize_provider")]
    pub provider: ProviderType,
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
    #[serde(deserialize_with = "deserialize_provider")]
    pub provider: ProviderType,
    pub title: String,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct EditMediaHttpRequest {
    pub user_id: String,
    pub url: Option<String>,
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

// ============== Helper Functions ==============

fn deserialize_provider<'de, D>(deserializer: D) -> Result<ProviderType, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    ProviderType::from_str(&s).ok_or_else(|| serde::de::Error::custom("invalid provider type"))
}
