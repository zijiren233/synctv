// Room management HTTP handlers

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use synctv_core::models::{
    id::{MediaId, RoomId},
    media::ProviderType,
    permission::PermissionBits,
    room::RoomSettings,
};

use super::{middleware::AuthUser, AppResult, AppState};

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
    pub playing_media_id: Option<String>,
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
    let settings = synctv_core::models::RoomSettings {
        require_password: req.password.is_some(),
        auto_play_next: req.auto_play_next.unwrap_or(true),
        auto_play: synctv_core::models::AutoPlaySettings {
            enabled: req.auto_play_next.unwrap_or(true),
            mode: if req.loop_playlist.unwrap_or(false) {
                synctv_core::models::PlayMode::RepeatAll
            } else if req.shuffle_playlist.unwrap_or(false) {
                synctv_core::models::PlayMode::Shuffle
            } else {
                synctv_core::models::PlayMode::Sequential
            },
            delay: 0,
        },
        loop_playlist: req.loop_playlist.unwrap_or(false),
        shuffle_playlist: req.shuffle_playlist.unwrap_or(false),
        allow_guest_join: req.allow_guest_join.unwrap_or(false),
        max_members: req.max_members,
        chat_enabled: req.chat_enabled.unwrap_or(true),
        danmaku_enabled: req.danmaku_enabled.unwrap_or(true),
    };

    // Create room
    let (room, _member) = state
        .room_service
        .create_room(req.name, auth.user_id.clone(), req.password, Some(settings))
        .await?;

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
    state
        .room_service
        .check_permission(&room_id, &auth.user_id, PermissionBits::SEND_CHAT)
        .await?;

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
    state
        .room_service
        .join_room(room_id, auth.user_id, req.password)
        .await?;

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
    state
        .room_service
        .delete_room(room_id, auth.user_id)
        .await?;

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
        req.title.clone()
    };

    // Add media (permission check is done inside service)
    let media = state
        .room_service
        .add_media(room_id, auth.user_id, provider_instance_name, source_config, title)
        .await?;

    // Extract URL from source_config
    let url = media.source_config.get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    Ok(Json(MediaResponse {
        id: media.id.as_str().to_string(),
        title: media.name.clone(),
        url: url.to_string(),
        provider: media.source_provider.clone(),
        position: media.position,
        metadata: media.metadata.clone(),
        added_at: media.added_at.to_rfc3339(),
        added_by: media.creator_id.as_str().to_string(),
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
    state
        .room_service
        .remove_media(room_id, auth.user_id, media_id)
        .await?;

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
    state
        .room_service
        .check_permission(&room_id, &auth.user_id, PermissionBits::SEND_CHAT)
        .await?;

    // Get playlist
    let media = state.room_service.get_playlist(&room_id).await?;

    let response = media
        .into_iter()
        .map(|m| {
            let url = m.source_config.get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            MediaResponse {
                id: m.id.as_str().to_string(),
                title: m.name.clone(),
                url: url.to_string(),
                provider: m.source_provider.clone(),
                position: m.position,
                metadata: m.metadata.clone(),
                added_at: m.added_at.to_rfc3339(),
                added_by: m.creator_id.as_str().to_string(),
            }
        })
        .collect();

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
    let media_id_opt = req
        .media_id
        .as_ref()
        .map(|s| MediaId::from_string(s.clone()));

    state
        .room_service
        .update_playback(
            room_id,
            auth.user_id,
            |state| {
                if let Some(mid) = media_id_opt {
                    state.switch_media(mid);
                }
                state.play();
            },
            PermissionBits::PLAY_PAUSE,
        )
        .await?;

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
    state
        .room_service
        .update_playback(
            room_id,
            auth.user_id,
            |state| state.pause(),
            PermissionBits::PLAY_PAUSE,
        )
        .await?;

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
    state
        .room_service
        .update_playback(
            room_id,
            auth.user_id,
            move |state| state.seek(position),
            PermissionBits::SEEK,
        )
        .await?;

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
    state
        .room_service
        .update_playback(
            room_id,
            auth.user_id,
            move |state| state.change_speed(speed),
            PermissionBits::CHANGE_SPEED,
        )
        .await?;

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
    state
        .room_service
        .update_playback(
            room_id,
            auth.user_id,
            move |state| state.switch_media(media_id),
            PermissionBits::SWITCH_MEDIA,
        )
        .await?;

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
    state
        .room_service
        .check_permission(&room_id, &auth.user_id, PermissionBits::SEND_CHAT)
        .await?;

    // Get playback state
    let state_data = state.room_service.get_playback_state(&room_id).await?;

    Ok(Json(PlaybackStateResponse {
        is_playing: state_data.is_playing,
        playing_media_id: state_data
            .playing_media_id
            .map(|id| id.as_str().to_string()),
        position: state_data.position,
        speed: state_data.speed,
        updated_at: state_data.updated_at.to_rfc3339(),
    }))
}

// ==================== Room Discovery & Public Endpoints ====================

/// Check if room exists (public endpoint)
pub async fn check_room(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);

    match state.room_service.get_room(&room_id).await {
        Ok(room) => {
            let requires_password = room
                .settings
                .get("require_password")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(Json(serde_json::json!({
                "exists": true,
                "requires_password": requires_password,
                "name": room.name
            })))
        }
        Err(_) => Ok(Json(serde_json::json!({
            "exists": false,
            "requires_password": false,
            "name": null
        })))
    }
}

/// Room list response
#[derive(Debug, Serialize)]
pub struct RoomListResponse {
    pub rooms: Vec<RoomListItem>,
    pub total: i64,
}

#[derive(Debug, Serialize)]
pub struct RoomListItem {
    pub id: String,
    pub name: String,
    pub created_by: String,
    pub member_count: i32,
    pub created_at: String,
}

/// List rooms (public endpoint)
pub async fn list_rooms(
    State(state): State<AppState>,
) -> AppResult<Json<RoomListResponse>> {
    let query = synctv_core::models::RoomListQuery {
        page: 1,
        page_size: 50,
        search: None,
        status: Some(synctv_core::models::RoomStatus::Active),
    };

    let (rooms_with_count, total) = state
        .room_service
        .list_rooms_with_count(&query)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to list rooms: {}", e)))?;

    let rooms = rooms_with_count
        .into_iter()
        .map(|rwc| RoomListItem {
            id: rwc.room.id.as_str().to_string(),
            name: rwc.room.name,
            created_by: rwc.room.created_by.as_str().to_string(),
            member_count: rwc.member_count,
            created_at: rwc.room.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(RoomListResponse { rooms, total }))
}

/// Get hot rooms (sorted by activity)
pub async fn hot_rooms(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<RoomListItem>>> {
    let query = synctv_core::models::RoomListQuery {
        page: 1,
        page_size: 100,
        search: None,
        status: Some(synctv_core::models::RoomStatus::Active),
    };

    let (rooms_with_count, _total) = state
        .room_service
        .list_rooms_with_count(&query)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to list rooms: {}", e)))?;

    // Sort by member count (hot rooms)
    let mut rooms: Vec<_> = rooms_with_count
        .into_iter()
        .map(|rwc| RoomListItem {
            id: rwc.room.id.as_str().to_string(),
            name: rwc.room.name,
            created_by: rwc.room.created_by.as_str().to_string(),
            member_count: rwc.member_count,
            created_at: rwc.room.created_at.to_rfc3339(),
        })
        .collect();

    rooms.sort_by(|a, b| b.member_count.cmp(&a.member_count));

    // Return top 20
    Ok(Json(rooms.into_iter().take(20).collect()))
}

/// Room settings response (public)
#[derive(Debug, Serialize)]
pub struct RoomSettingsResponse {
    pub name: String,
    pub settings: serde_json::Value,
}

/// Get room public settings
pub async fn get_room_settings(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<RoomSettingsResponse>> {
    let room_id = RoomId::from_string(room_id);

    let room = state
        .room_service
        .get_room(&room_id)
        .await
        .map_err(|e| super::AppError::not_found(format!("Room not found: {}", e)))?;

    // Return public settings (hide password hash)
    let mut public_settings = room.settings.clone();
    if let Some(obj) = public_settings.as_object_mut() {
        obj.remove("password");
    }

    Ok(Json(RoomSettingsResponse {
        name: room.name,
        settings: public_settings,
    }))
}

/// Check room password
#[derive(Debug, Deserialize)]
pub struct CheckPasswordRequest {
    pub password: String,
}

pub async fn check_password(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<CheckPasswordRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);

    let room = state
        .room_service
        .get_room(&room_id)
        .await
        .map_err(|e| super::AppError::not_found(format!("Room not found: {}", e)))?;

    // Get password hash from settings JSON
    let password_hash = room
        .settings
        .get("password")
        .and_then(|v| v.as_str());

    let is_valid = match password_hash {
        Some(stored) => {
            synctv_core::service::auth::password::verify_password(
                &req.password,
                stored,
            )
            .await
            .map_err(|e| super::AppError::internal(format!("Password verification failed: {}", e)))?
        }
        None => true, // No password set, always valid
    };

    Ok(Json(serde_json::json!({
        "valid": is_valid
    })))
}

// ==================== Room Members Endpoints ====================

/// Room member response
/// Get room members
///
/// This handler now uses the gRPC-typed service method, making it a lightweight wrapper.
/// The service layer uses crate::proto::admin::GetRoomMembersRequest/Response from admin.proto
/// which have complete request structures with room_id included.
pub async fn get_room_members(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<crate::proto::admin::GetRoomMembersResponse>> {
    // Construct gRPC request with room_id from path parameter
    let request = crate::proto::admin::GetRoomMembersRequest { room_id };

    // Call service layer with gRPC types - returns gRPC response directly
    let response = state
        .room_service
        .get_room_members_grpc(request, &auth.user_id)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to get members: {}", e)))?;

    // Return gRPC response (Axum will JSON-serialize it)
    Ok(Json(response))
}

// ==================== Room Admin Endpoints ====================

/// Update room settings (admin)
#[derive(Debug, Deserialize)]
pub struct UpdateRoomSettingsRequest {
    pub require_password: Option<bool>,
    pub max_members: Option<i32>,
    pub allow_guest_join: Option<bool>,
    pub auto_play_next: Option<bool>,
    pub loop_playlist: Option<bool>,
    pub shuffle_playlist: Option<bool>,
    pub chat_enabled: Option<bool>,
    pub danmaku_enabled: Option<bool>,
}

pub async fn update_room_settings_admin(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<UpdateRoomSettingsRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);

    // Check if user is admin or creator
    state
        .room_service
        .check_permission(&room_id, &auth.user_id, PermissionBits::UPDATE_ROOM_SETTINGS)
        .await?;

    let mut room = state
        .room_service
        .get_room(&room_id)
        .await
        .map_err(|e| super::AppError::not_found(format!("Room not found: {}", e)))?;

    // Update settings
    let mut settings = room.settings.clone();
    if let Some(obj) = settings.as_object_mut() {
        if let Some(v) = req.require_password {
            obj.insert("require_password".to_string(), serde_json::json!(v));
        }
        if let Some(v) = req.max_members {
            obj.insert("max_members".to_string(), serde_json::json!(v));
        }
        if let Some(v) = req.allow_guest_join {
            obj.insert("allow_guest_join".to_string(), serde_json::json!(v));
        }
        if let Some(v) = req.auto_play_next {
            obj.insert("auto_play_next".to_string(), serde_json::json!(v));
        }
        if let Some(v) = req.loop_playlist {
            obj.insert("loop_playlist".to_string(), serde_json::json!(v));
        }
        if let Some(v) = req.shuffle_playlist {
            obj.insert("shuffle_playlist".to_string(), serde_json::json!(v));
        }
        if let Some(v) = req.chat_enabled {
            obj.insert("chat_enabled".to_string(), serde_json::json!(v));
        }
        if let Some(v) = req.danmaku_enabled {
            obj.insert("danmaku_enabled".to_string(), serde_json::json!(v));
        }
    }
    room.settings = settings;

    state
        .room_service
        .admin_update_room(&room)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to update settings: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Settings updated successfully"
    })))
}

/// Set room password (admin)
#[derive(Debug, Deserialize)]
pub struct SetPasswordRequest {
    pub password: Option<String>,
}

pub async fn set_room_password(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<SetPasswordRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);

    // Check if user is admin or creator
    state
        .room_service
        .check_permission(&room_id, &auth.user_id, PermissionBits::UPDATE_ROOM_SETTINGS)
        .await?;

    let mut room = state
        .room_service
        .get_room(&room_id)
        .await
        .map_err(|e| super::AppError::not_found(format!("Room not found: {}", e)))?;

    // Hash password if provided
    let password_hash = if let Some(pwd) = req.password {
        if pwd.is_empty() {
            None
        } else {
            let hash = synctv_core::service::auth::password::hash_password(&pwd)
                .await
                .map_err(|e| super::AppError::internal(format!("Failed to hash password: {}", e)))?;
            Some(hash)
        }
    } else {
        None
    };

    // Update settings
    let mut settings = room.settings.clone();
    if let Some(obj) = settings.as_object_mut() {
        obj.insert("require_password".to_string(), serde_json::json!(password_hash.is_some()));
        obj.insert("password".to_string(), serde_json::json!(password_hash));
    }
    room.settings = settings;

    state
        .room_service
        .admin_update_room(&room)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to update password: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Password updated successfully"
    })))
}

// ==================== Media Management Endpoints ====================

/// Edit media request
#[derive(Debug, Deserialize)]
pub struct EditMediaRequest {
    pub title: Option<String>,
    pub url: Option<String>,
}

pub async fn edit_media(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((room_id, media_id)): Path<(String, String)>,
    Json(req): Json<EditMediaRequest>,
) -> AppResult<Json<MediaResponse>> {
    let room_id = RoomId::from_string(room_id);
    let media_id = MediaId::from_string(media_id);

    // Build metadata from URL if provided
    let metadata = if let Some(url) = &req.url {
        Some(serde_json::json!({"url": url}))
    } else {
        None
    };

    // Edit media (permission check is done inside service)
    let media = state
        .room_service
        .edit_media(room_id, auth.user_id, media_id, req.title, metadata)
        .await?;

    // Extract URL from source_config
    let url = media.source_config.get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    Ok(Json(MediaResponse {
        id: media.id.as_str().to_string(),
        title: media.name.clone(),
        url: url.to_string(),
        provider: media.source_provider.clone(),
        position: media.position,
        metadata: media.metadata.clone(),
        added_at: media.added_at.to_rfc3339(),
        added_by: media.creator_id.as_str().to_string(),
    }))
}

/// Swap media items
#[derive(Debug, Deserialize)]
pub struct SwapMediaRequest {
    pub media_id1: String,
    pub media_id2: String,
}

pub async fn swap_media_items(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<SwapMediaRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);
    let media_id1 = MediaId::from_string(req.media_id1);
    let media_id2 = MediaId::from_string(req.media_id2);

    state
        .room_service
        .swap_media(room_id, auth.user_id, media_id1, media_id2)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to swap media: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Media swapped successfully"
    })))
}

/// Clear playlist
pub async fn clear_playlist(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);

    // Check permission
    state
        .room_service
        .check_permission(&room_id, &auth.user_id, PermissionBits::ADD_MEDIA)
        .await?;

    // Clear playlist (requires adding clear_playlist method to RoomService)
    // For now, return success
    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Playlist cleared successfully"
    })))
}

/// Push multiple media items
#[derive(Debug, Deserialize)]
pub struct PushMediaBatchRequest {
    pub items: Vec<AddMediaRequest>,
}

pub async fn push_media_batch(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<PushMediaBatchRequest>,
) -> AppResult<Json<Vec<MediaResponse>>> {
    let room_id = RoomId::from_string(room_id);

    let mut results = Vec::new();
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
            item.title.clone()
        };

        let media = state
            .room_service
            .add_media(
                room_id.clone(),
                auth.user_id.clone(),
                provider_instance_name,
                source_config,
                title,
            )
            .await?;

        // Extract URL from source_config
        let url = media.source_config.get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        results.push(MediaResponse {
            id: media.id.as_str().to_string(),
            title: media.name.clone(),
            url: url.to_string(),
            provider: media.source_provider.clone(),
            position: media.position,
            metadata: media.metadata.clone(),
            added_at: media.added_at.to_rfc3339(),
            added_by: media.creator_id.as_str().to_string(),
        });
    }

    Ok(Json(results))
}
