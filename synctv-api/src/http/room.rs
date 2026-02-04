// Room management HTTP handlers
//
// This layer now uses proto types and delegates to the impls layer for business logic

use axum::{
    extract::{Path, State},
    Json,
};
use synctv_core::models::{
    id::{MediaId, RoomId},
    permission::PermissionBits,
};

use super::{middleware::AuthUser, AppResult, AppState};
use crate::proto::client::{CreateRoomResponse, CreateRoomRequest, GetRoomResponse, JoinRoomResponse, JoinRoomRequest, LeaveRoomResponse, LeaveRoomRequest, DeleteRoomResponse, DeleteRoomRequest, AddMediaResponse, AddMediaRequest, RemoveMediaResponse, RemoveMediaRequest, GetPlaylistResponse, SwapMediaResponse, SwapMediaRequest, PlayResponse, PlayRequest, PauseResponse, SeekResponse, SeekRequest, ChangeSpeedResponse, ChangeSpeedRequest, SwitchMediaResponse, SwitchMediaRequest, GetPlaybackStateResponse, GetPlaybackStateRequest, GetRoomMembersResponse, CheckRoomResponse, ListRoomsResponse, ListRoomsRequest, SetRoomSettingsResponse, SetRoomSettingsRequest};

/// Room settings for HTTP requests
#[derive(Debug, Clone, Default)]
pub struct RoomSettingsRequest {
    pub password: Option<String>,
    pub max_members: Option<i32>,
    pub allow_guest_join: Option<bool>,
    pub auto_play_next: Option<bool>,
    pub loop_playlist: Option<bool>,
    pub shuffle_playlist: Option<bool>,
    pub chat_enabled: Option<bool>,
    pub danmaku_enabled: Option<bool>,
}

/// Helper function to convert HTTP-style room settings request to proto settings bytes
#[allow(clippy::too_many_arguments)]
fn build_room_settings_bytes(
    password: &Option<String>,
    max_members: Option<i32>,
    allow_guest_join: Option<bool>,
    auto_play_next: Option<bool>,
    loop_playlist: Option<bool>,
    shuffle_playlist: Option<bool>,
    chat_enabled: Option<bool>,
    danmaku_enabled: Option<bool>,
) -> Result<Vec<u8>, String> {
    use serde_json::json;

    let settings = json!({
        "require_password": password.is_some(),
        "max_members": max_members,
        "allow_guest_join": allow_guest_join.unwrap_or(false),
        "auto_play_next": auto_play_next.unwrap_or(true),
        "loop_playlist": loop_playlist.unwrap_or(false),
        "shuffle_playlist": shuffle_playlist.unwrap_or(false),
        "chat_enabled": chat_enabled.unwrap_or(true),
        "danmaku_enabled": danmaku_enabled.unwrap_or(true),
    });

    serde_json::to_vec(&settings)
        .map_err(|e| format!("Failed to serialize settings: {e}"))
}

// ==================== Room Management Endpoints ====================

/// Create a new room
pub async fn create_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<CreateRoomResponse>> {
    // Extract fields from JSON request
    let name = req.get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| super::AppError::bad_request("Missing name field"))?
        .to_string();

    if name.is_empty() {
        return Err(super::AppError::bad_request("Room name cannot be empty"));
    }

    let password = req.get("password")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let max_members = req.get("max_members").and_then(serde_json::Value::as_i64).map(|v| v as i32);
    let allow_guest_join = req.get("allow_guest_join").and_then(serde_json::Value::as_bool);
    let auto_play_next = req.get("auto_play_next").and_then(serde_json::Value::as_bool);
    let loop_playlist = req.get("loop_playlist").and_then(serde_json::Value::as_bool);
    let shuffle_playlist = req.get("shuffle_playlist").and_then(serde_json::Value::as_bool);
    let chat_enabled = req.get("chat_enabled").and_then(serde_json::Value::as_bool);
    let danmaku_enabled = req.get("danmaku_enabled").and_then(serde_json::Value::as_bool);

    // Build settings JSON
    let settings_bytes = build_room_settings_bytes(
        &if password.is_empty() { None } else { Some(password.clone()) },
        max_members,
        allow_guest_join,
        auto_play_next,
        loop_playlist,
        shuffle_playlist,
        chat_enabled,
        danmaku_enabled,
    ).map_err(super::AppError::bad_request)?;

    // Create proto request
    let proto_req = CreateRoomRequest {
        name,
        password,
        settings: settings_bytes,
    };

    // Call impls layer
    let response = state
        .client_api
        .create_room(&auth.user_id.to_string(), proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Get room information
pub async fn get_room(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<GetRoomResponse>> {
    let response = state
        .client_api
        .get_room(&room_id)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Join a room
pub async fn join_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<JoinRoomResponse>> {
    let password = req.get("password")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let proto_req = JoinRoomRequest {
        room_id: room_id.clone(),
        password,
    };

    let response = state
        .client_api
        .join_room(&auth.user_id.to_string(), proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Leave a room
pub async fn leave_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<LeaveRoomResponse>> {
    let proto_req = LeaveRoomRequest { room_id };
    let response = state
        .client_api
        .leave_room(&auth.user_id.to_string(), proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Delete a room
pub async fn delete_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<DeleteRoomResponse>> {
    let proto_req = DeleteRoomRequest { room_id };
    let response = state
        .client_api
        .delete_room(&auth.user_id.to_string(), proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

// ==================== Media Management Endpoints ====================

/// Add media to playlist
pub async fn add_media(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<AddMediaResponse>> {
    let url = req.get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| super::AppError::bad_request("Missing url field"))?
        .to_string();

    let provider = req.get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let title = req.get("title")
        .and_then(|v| v.as_str()).map_or_else(|| {
            // Extract title from URL
            url.split('/').next_back().unwrap_or("Unknown").to_string()
        }, std::string::ToString::to_string);

    // Build source config JSON
    let source_config = serde_json::json!({"url": url});
    let source_config_bytes = serde_json::to_vec(&source_config)
        .map_err(|e| super::AppError::bad_request(format!("Invalid source config: {e}")))?;

    let proto_req = AddMediaRequest {
        playlist_id: String::new(), // Default/empty playlist
        url,
        provider,
        provider_instance_name: String::new(),
        source_config: source_config_bytes,
        title,
    };

    let response = state
        .client_api
        .add_media(&auth.user_id.to_string(), &room_id, proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Remove media from playlist
pub async fn remove_media(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((room_id, media_id)): Path<(String, String)>,
) -> AppResult<Json<RemoveMediaResponse>> {
    let proto_req = RemoveMediaRequest { media_id };
    let response = state
        .client_api
        .remove_media(&auth.user_id.to_string(), &room_id, proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Get playlist
pub async fn get_playlist(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<GetPlaylistResponse>> {
    let response = state
        .client_api
        .get_playlist(&room_id)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Swap media items
pub async fn swap_media_items(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<SwapMediaResponse>> {
    let media_id1 = req.get("media_id1")
        .and_then(|v| v.as_str())
        .ok_or_else(|| super::AppError::bad_request("Missing media_id1 field"))?
        .to_string();

    let media_id2 = req.get("media_id2")
        .and_then(|v| v.as_str())
        .ok_or_else(|| super::AppError::bad_request("Missing media_id2 field"))?
        .to_string();

    let proto_req = SwapMediaRequest {
        room_id: room_id.clone(),
        media_id1,
        media_id2,
    };

    let response = state
        .client_api
        .swap_media(&auth.user_id.to_string(), proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

// ==================== Playback Control Endpoints ====================

/// Play (resume playback)
pub async fn play(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<PlayResponse>> {
    let proto_req = PlayRequest {};
    let response = state
        .client_api
        .play(&auth.user_id.to_string(), &room_id, proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Pause playback
pub async fn pause(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<PauseResponse>> {
    let response = state
        .client_api
        .pause(&auth.user_id.to_string(), &room_id)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Seek to position
pub async fn seek(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<SeekResponse>> {
    let position = req.get("position")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| super::AppError::bad_request("Missing or invalid position field"))?;

    let proto_req = SeekRequest { position };
    let response = state
        .client_api
        .seek(&auth.user_id.to_string(), &room_id, proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Change playback speed
pub async fn change_speed(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<ChangeSpeedResponse>> {
    let speed = req.get("speed")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| super::AppError::bad_request("Missing or invalid speed field"))?;

    let proto_req = ChangeSpeedRequest { speed };
    let response = state
        .client_api
        .change_speed(&auth.user_id.to_string(), &room_id, proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Switch to a different media
pub async fn switch_media(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<SwitchMediaResponse>> {
    let media_id = req.get("media_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| super::AppError::bad_request("Missing media_id field"))?
        .to_string();

    let proto_req = SwitchMediaRequest { media_id };
    let response = state
        .client_api
        .switch_media(&auth.user_id.to_string(), &room_id, proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Get playback state
pub async fn get_playback_state(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<GetPlaybackStateResponse>> {
    let proto_req = GetPlaybackStateRequest {};
    let response = state
        .client_api
        .get_playback_state(&room_id, proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

// ==================== Room Members Endpoints ====================

/// Get room members
pub async fn get_room_members(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<GetRoomMembersResponse>> {
    let response = state
        .client_api
        .get_room_members(&room_id)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

// ==================== Room Discovery & Public Endpoints ====================

/// Check if room exists (public endpoint)
pub async fn check_room(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<CheckRoomResponse>> {
    let room_id_obj = RoomId::from_string(room_id.clone());

    let (exists, requires_password, name) = match state.room_service.get_room(&room_id_obj).await {
        Ok(room) => {
            let settings = state.room_service.get_room_settings(&room_id_obj).await
                .unwrap_or_default();
            (true, settings.require_password.0, room.name.clone())
        }
        Err(_) => (false, false, String::new()),
    };

    Ok(Json(CheckRoomResponse {
        exists,
        requires_password,
        name,
    }))
}

/// List rooms (public endpoint)
pub async fn list_rooms(
    State(state): State<AppState>,
) -> AppResult<Json<ListRoomsResponse>> {
    let proto_req = ListRoomsRequest {
        page: 1,
        page_size: 50,
    };
    let response = state
        .client_api
        .list_rooms(proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Get hot rooms (sorted by activity)
pub async fn hot_rooms(
    State(state): State<AppState>,
) -> AppResult<Json<ListRoomsResponse>> {
    let proto_req = ListRoomsRequest {
        page: 1,
        page_size: 100,
    };
    let mut response = state
        .client_api
        .list_rooms(proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    // Sort by member count (hot rooms)
    response.rooms.sort_by(|a, b| b.member_count.cmp(&a.member_count));

    // Return top 20
    response.rooms = response.rooms.into_iter().take(20).collect();
    response.total = response.rooms.len() as i32;

    Ok(Json(response))
}

// ==================== Room Settings Endpoints ====================

/// Update room settings
pub async fn set_room_settings_admin(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<SetRoomSettingsResponse>> {
    // Build settings JSON from request fields
    let settings_json = req.clone();
    let settings_bytes = serde_json::to_vec(&settings_json)
        .map_err(|e| super::AppError::bad_request(format!("Invalid settings JSON: {e}")))?;

    let proto_req = SetRoomSettingsRequest {
        room_id: room_id.clone(),
        settings: settings_bytes,
    };

    let response = state
        .client_api
        .update_room_settings(&auth.user_id.to_string(), &room_id, proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Set room password
pub async fn set_room_password(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let password = req.get("password")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let room_id_obj = RoomId::from_string(room_id.clone());

    // Check if user is admin or creator
    state
        .room_service
        .check_permission(&room_id_obj, &auth.user_id, PermissionBits::UPDATE_ROOM_SETTINGS)
        .await?;

    // Hash password if provided
    let password_hash = if password.is_empty() {
        None
    } else {
        let hash = synctv_core::service::auth::password::hash_password(&password)
            .await
            .map_err(|e| super::AppError::internal(format!("Failed to hash password: {e}")))?;
        Some(hash)
    };

    // Update password hash
    state
        .room_service
        .update_room_password(&room_id_obj, password_hash)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to update password: {e}")))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Password updated successfully"
    })))
}

/// Check room password
pub async fn check_password(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let password = req.get("password")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let room_id_obj = RoomId::from_string(room_id);

    // Verify room exists
    state
        .room_service
        .get_room(&room_id_obj)
        .await
        .map_err(|e| super::AppError::not_found(format!("Room not found: {e}")))?;

    let is_valid = state
        .room_service
        .check_room_password(&room_id_obj, &password)
        .await
        .map_err(|e| super::AppError::internal(format!("Password verification failed: {e}")))?;

    Ok(Json(serde_json::json!({
        "valid": is_valid
    })))
}

/// Get room public settings
pub async fn get_room_settings(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id_obj = RoomId::from_string(room_id);

    let room = state
        .room_service
        .get_room(&room_id_obj)
        .await
        .map_err(|e| super::AppError::not_found(format!("Room not found: {e}")))?;

    // Load settings from service layer
    let room_settings = state.room_service.get_room_settings(&room_id_obj).await
        .map_err(|e| super::AppError::internal(format!("Failed to load settings: {e}")))?;

    // Convert to JSON and hide password hash
    let mut settings_json = serde_json::to_value(&room_settings)
        .map_err(|e| super::AppError::internal(format!("Failed to serialize settings: {e}")))?;

    if let Some(obj) = settings_json.as_object_mut() {
        obj.remove("password");
    }

    Ok(Json(serde_json::json!({
        "name": room.name,
        "settings": settings_json
    })))
}

/// Push multiple media items to playlist
pub async fn push_media_batch(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<Vec<AddMediaResponse>>> {
    let room_id_obj = RoomId::from_string(room_id.clone());

    // Check permission
    state
        .room_service
        .check_permission(&room_id_obj, &auth.user_id, PermissionBits::ADD_MEDIA)
        .await?;

    let items = req.get("items")
        .and_then(|v| v.as_array())
        .ok_or_else(|| super::AppError::bad_request("Missing items array"))?;

    let mut results = Vec::new();
    for item in items {
        let url = item.get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| super::AppError::bad_request("Missing url in item"))?
            .to_string();

        let provider = item.get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let title = item.get("title")
            .and_then(|v| v.as_str()).map_or_else(|| {
                // Extract title from URL
                url.split('/').next_back().unwrap_or("Unknown").to_string()
            }, std::string::ToString::to_string);

        // Build source config JSON
        let source_config = serde_json::json!({"url": url});
        let source_config_bytes = serde_json::to_vec(&source_config)
            .map_err(|e| super::AppError::bad_request(format!("Invalid source config: {e}")))?;

        let proto_req = AddMediaRequest {
            playlist_id: String::new(),
            url,
            provider,
            provider_instance_name: String::new(),
            source_config: source_config_bytes,
            title,
        };

        let response = state
            .client_api
            .add_media(&auth.user_id.to_string(), &room_id, proto_req)
            .await
            .map_err(super::AppError::internal_server_error)?;

        results.push(response);
    }

    Ok(Json(results))
}

/// Edit media
pub async fn edit_media(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((room_id, media_id)): Path<(String, String)>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<AddMediaResponse>> {
    let room_id_obj = RoomId::from_string(room_id.clone());
    let media_id_obj = MediaId::from_string(media_id.clone());

    // Build metadata from request
    let title = req.get("title")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let url = req.get("url")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    // Build metadata JSON
    let metadata = url.as_ref().map(|url| serde_json::json!({"url": url}));

    // Edit media (permission check is done inside service)
    let media = state
        .room_service
        .edit_media(room_id_obj, auth.user_id, media_id_obj, title, metadata)
        .await
        .map_err(|e| super::AppError::internal_server_error(format!("Failed to edit media: {e}")))?;

    // Convert to response
    let media_url = media.source_config.get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    Ok(Json(AddMediaResponse {
        media: Some(crate::proto::client::Media {
            id: media.id.as_str().to_string(),
            room_id: media.room_id.as_str().to_string(),
            url: media_url.to_string(),
            provider: media.source_provider.clone(),
            title: media.name.clone(),
            metadata: serde_json::to_vec(&media.metadata).unwrap_or_default(),
            position: media.position,
            added_at: media.added_at.timestamp(),
            added_by: media.creator_id.as_str().to_string(),
            provider_instance_name: media.provider_instance_name.clone().unwrap_or_default(),
            source_config: serde_json::to_vec(&media.source_config).unwrap_or_default(),
        }),
    }))
}

/// Clear playlist
pub async fn clear_playlist(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id_obj = RoomId::from_string(room_id);

    // Check permission
    state
        .room_service
        .check_permission(&room_id_obj, &auth.user_id, PermissionBits::ADD_MEDIA)
        .await?;

    // Get current playlist
    let media_list = state.room_service.get_playlist(&room_id_obj).await
        .map_err(|e| super::AppError::internal_server_error(format!("Failed to get playlist: {e}")))?;

    // Remove all media
    for media in media_list {
        state.room_service.remove_media(room_id_obj.clone(), auth.user_id.clone(), media.id.clone()).await
            .map_err(|e| super::AppError::internal_server_error(format!("Failed to remove media: {e}")))?;
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Playlist cleared successfully"
    })))
}
