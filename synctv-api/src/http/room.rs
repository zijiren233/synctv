// Room management HTTP handlers

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use synctv_core::models::{
    id::{RoomId, MediaId},
    room::RoomSettings,
    permission::PermissionBits,
    media::ProviderType,
};

use super::{AppState, AppResult, middleware::AuthUser};

/// Create room request
#[derive(Debug, Deserialize)]
pub struct CreateRoomRequest {
    pub name: String,
    pub password: Option<String>,
    pub max_members: Option<i32>,
    pub allow_guest_join: Option<bool>,
    pub auto_play_next: Option<bool>,
    pub loop_playlist: Option<bool>,
    pub shuffle_playlist: Option<bool>,
    pub chat_enabled: Option<bool>,
    pub danmaku_enabled: Option<bool>,
}

/// Join room request
#[derive(Debug, Deserialize)]
pub struct JoinRoomRequest {
    pub password: Option<String>,
}

/// Add media request
#[derive(Debug, Deserialize)]
pub struct AddMediaRequest {
    pub title: String,
    pub url: String,
    pub provider: String,
}

/// Playback control requests
#[derive(Debug, Deserialize)]
pub struct PlayRequest {
    pub media_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SeekRequest {
    pub position: f64,
}

#[derive(Debug, Deserialize)]
pub struct ChangeSpeedRequest {
    pub speed: f64,
}

#[derive(Debug, Deserialize)]
pub struct SwitchMediaRequest {
    pub media_id: String,
}

/// Room response
#[derive(Debug, Serialize)]
pub struct RoomResponse {
    pub id: String,
    pub name: String,
    pub created_by: String,
    pub status: String,
    pub settings: serde_json::Value,
    pub created_at: String,
}

/// Media response
#[derive(Debug, Serialize)]
pub struct MediaResponse {
    pub id: String,
    pub title: String,
    pub url: String,
    pub provider: String,
    pub position: i32,
    pub metadata: serde_json::Value,
    pub added_at: String,
    pub added_by: String,
}

/// Playback state response
#[derive(Debug, Serialize)]
pub struct PlaybackStateResponse {
    pub is_playing: bool,
    pub current_media_id: Option<String>,
    pub position: f64,
    pub speed: f64,
    pub updated_at: String,
}

/// Create a new room
pub async fn create_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateRoomRequest>,
) -> AppResult<Json<RoomResponse>> {
    // Validate input
    if req.name.is_empty() {
        return Err(super::AppError::bad_request("Room name cannot be empty"));
    }

    // Build room settings
    let settings = RoomSettings {
        require_password: req.password.is_some(),
        auto_play_next: req.auto_play_next.unwrap_or(true),
        loop_playlist: req.loop_playlist.unwrap_or(false),
        shuffle_playlist: req.shuffle_playlist.unwrap_or(false),
        allow_guest_join: req.allow_guest_join.unwrap_or(false),
        max_members: req.max_members,
        chat_enabled: req.chat_enabled.unwrap_or(true),
        danmaku_enabled: req.danmaku_enabled.unwrap_or(true),
    };

    // Create room
    let (room, _member) = state.room_service.create_room(
        req.name,
        auth.user_id.clone(),
        req.password,
        Some(settings),
    ).await?;

    Ok(Json(RoomResponse {
        id: room.id.as_str().to_string(),
        name: room.name,
        created_by: room.created_by.as_str().to_string(),
        status: room.status.as_str().to_string(),
        settings: room.settings.clone(),
        created_at: room.created_at.to_rfc3339(),
    }))
}

/// Get room information
pub async fn get_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<RoomResponse>> {
    let room_id = RoomId::from_string(room_id);

    // Check if user is a member (will fail if not)
    state.room_service.check_permission(&room_id, &auth.user_id, PermissionBits::SEND_CHAT).await?;

    // Get room
    let room = state.room_service.get_room(&room_id).await?;

    Ok(Json(RoomResponse {
        id: room.id.as_str().to_string(),
        name: room.name,
        created_by: room.created_by.as_str().to_string(),
        status: room.status.as_str().to_string(),
        settings: room.settings.clone(),
        created_at: room.created_at.to_rfc3339(),
    }))
}

/// Join a room
pub async fn join_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<JoinRoomRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);

    // Join room
    state.room_service.join_room(room_id, auth.user_id, req.password).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Joined room successfully"
    })))
}

/// Leave a room
pub async fn leave_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);

    // Leave room
    state.room_service.leave_room(room_id, auth.user_id).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Left room successfully"
    })))
}

/// Delete a room
pub async fn delete_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);

    // Delete room (only creator can delete)
    state.room_service.delete_room(room_id, auth.user_id).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Room deleted successfully"
    })))
}

/// Add media to playlist
pub async fn add_media(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<AddMediaRequest>,
) -> AppResult<Json<MediaResponse>> {
    let room_id = RoomId::from_string(room_id);

    // Parse provider type
    let provider = ProviderType::from_str(&req.provider)
        .ok_or_else(|| super::AppError::bad_request(format!("Invalid provider: {}", req.provider)))?;

    // Add media (permission check is done inside service)
    let media = state.room_service.add_media(
        room_id,
        auth.user_id,
        req.url,
        provider,
        req.title,
    ).await?;

    Ok(Json(MediaResponse {
        id: media.id.as_str().to_string(),
        title: media.title,
        url: media.url,
        provider: media.provider.as_str().to_string(),
        position: media.position,
        metadata: media.metadata.clone(),
        added_at: media.added_at.to_rfc3339(),
        added_by: media.added_by.as_str().to_string(),
    }))
}

/// Remove media from playlist
pub async fn remove_media(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((room_id, media_id)): Path<(String, String)>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);
    let media_id = MediaId::from_string(media_id);

    // Remove media (permission check is done inside service)
    state.room_service.remove_media(room_id, auth.user_id, media_id).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Media removed successfully"
    })))
}

/// Get playlist
pub async fn get_playlist(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<Vec<MediaResponse>>> {
    let room_id = RoomId::from_string(room_id);

    // Check if user is a member
    state.room_service.check_permission(&room_id, &auth.user_id, PermissionBits::SEND_CHAT).await?;

    // Get playlist
    let media = state.room_service.get_playlist(room_id).await?;

    let response = media.into_iter().map(|m| MediaResponse {
        id: m.id.as_str().to_string(),
        title: m.title,
        url: m.url,
        provider: m.provider.as_str().to_string(),
        position: m.position,
        metadata: m.metadata.clone(),
        added_at: m.added_at.to_rfc3339(),
        added_by: m.added_by.as_str().to_string(),
    }).collect();

    Ok(Json(response))
}

/// Play (resume playback)
pub async fn play(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<PlayRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);

    // Update playback state
    let media_id_opt = req.media_id.as_ref().map(|s| MediaId::from_string(s.clone()));

    state.room_service.update_playback(
        room_id,
        auth.user_id,
        |state| {
            if let Some(mid) = media_id_opt {
                state.switch_media(mid);
            }
            state.play();
        },
        PermissionBits::PLAY_PAUSE,
    ).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Playback started"
    })))
}

/// Pause playback
pub async fn pause(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);

    // Update playback state
    state.room_service.update_playback(
        room_id,
        auth.user_id,
        |state| state.pause(),
        PermissionBits::PLAY_PAUSE,
    ).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Playback paused"
    })))
}

/// Seek to position
pub async fn seek(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<SeekRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);

    // Update playback state
    let position = req.position;
    state.room_service.update_playback(
        room_id,
        auth.user_id,
        move |state| state.seek(position),
        PermissionBits::SEEK,
    ).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Position updated"
    })))
}

/// Change playback speed
pub async fn change_speed(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<ChangeSpeedRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);

    // Update playback state
    let speed = req.speed;
    state.room_service.update_playback(
        room_id,
        auth.user_id,
        move |state| state.change_speed(speed),
        PermissionBits::CHANGE_SPEED,
    ).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Playback speed updated"
    })))
}

/// Switch to a different media
pub async fn switch_media(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<SwitchMediaRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);
    let media_id = MediaId::from_string(req.media_id);

    // Update playback state
    state.room_service.update_playback(
        room_id,
        auth.user_id,
        move |state| state.switch_media(media_id),
        PermissionBits::SWITCH_MEDIA,
    ).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Switched to new media"
    })))
}

/// Get playback state
pub async fn get_playback_state(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<PlaybackStateResponse>> {
    let room_id = RoomId::from_string(room_id);

    // Check if user is a member
    state.room_service.check_permission(&room_id, &auth.user_id, PermissionBits::SEND_CHAT).await?;

    // Get playback state
    let state_data = state.room_service.get_playback_state(&room_id).await?;

    Ok(Json(PlaybackStateResponse {
        is_playing: state_data.is_playing,
        current_media_id: state_data.current_media_id.map(|id| id.as_str().to_string()),
        position: state_data.position,
        speed: state_data.speed,
        updated_at: state_data.updated_at.to_rfc3339(),
    }))
}
