// Room management HTTP handlers
//
// This layer now uses proto types and delegates to the impls layer for business logic

use axum::{
    extract::{Path, Query, State},
    Json,
};
use synctv_core::models::{
    id::{MediaId, RoomId},
    permission::PermissionBits,
};

use super::{middleware::AuthUser, AppResult, AppState};
use crate::proto::client::{CreateRoomResponse, CreateRoomRequest, GetRoomResponse, JoinRoomResponse, JoinRoomRequest, LeaveRoomResponse, LeaveRoomRequest, DeleteRoomResponse, DeleteRoomRequest, AddMediaResponse, AddMediaRequest, RemoveMediaResponse, RemoveMediaRequest, ListPlaylistResponse, SwapMediaResponse, SwapMediaRequest, PlayResponse, PlayRequest, PauseResponse, SeekResponse, SeekRequest, GetPlaybackStateResponse, GetPlaybackStateRequest, GetRoomMembersResponse, CheckRoomResponse, ListRoomsResponse, ListRoomsRequest, UpdateRoomSettingsRequest};

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
        "max_members": max_members.unwrap_or(0),
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
#[tracing::instrument(name = "http_create_room", skip(state), fields(user_id = %auth.user_id))]
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
    if name.len() > 100 {
        return Err(super::AppError::bad_request("Room name too long (max 100 characters)"));
    }

    tracing::info!(user_id = %auth.user_id, room_name = %name, "Creating new room");

    let description = req.get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let password = req.get("password")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let max_members = req.get("max_members").and_then(serde_json::Value::as_i64).map(|v| v.clamp(0, 10000) as i32);
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
        name: name.clone(),
        password,
        settings: settings_bytes,
        description,
    };

    // Call impls layer
    let response = state
        .client_api
        .create_room(&auth.user_id.to_string(), proto_req)
        .await
        .map_err(|e| {
            tracing::error!(user_id = %auth.user_id, room_name = %name, error = %e, "Failed to create room");
            super::AppError::internal_server_error(e)
        })?;

    let room_id = response.room.as_ref().map_or("unknown", |r| r.id.as_str());
    tracing::info!(user_id = %auth.user_id, room_id = %room_id, "Room created successfully");
    Ok(Json(response))
}

/// Get room information
pub async fn get_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<GetRoomResponse>> {
    let room_id_obj = RoomId::from_string(room_id.clone());
    state.room_service
        .check_membership(&room_id_obj, &auth.user_id)
        .await
        .map_err(|e| super::AppError::forbidden(e.to_string()))?;

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
#[tracing::instrument(name = "http_delete_room", skip(state), fields(user_id = %auth.user_id, room_id = %room_id))]
pub async fn delete_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<DeleteRoomResponse>> {
    tracing::info!(user_id = %auth.user_id, room_id = %room_id, "Deleting room");

    let proto_req = DeleteRoomRequest { room_id: room_id.clone() };
    let response = state
        .client_api
        .delete_room(&auth.user_id.to_string(), proto_req)
        .await
        .map_err(|e| {
            tracing::error!(user_id = %auth.user_id, room_id = %room_id, error = %e, "Failed to delete room");
            super::AppError::internal_server_error(e)
        })?;

    tracing::info!(user_id = %auth.user_id, room_id = %room_id, "Room deleted successfully");
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

    if url.len() > 2048 {
        return Err(super::AppError::bad_request("URL too long (max 2048 characters)"));
    }

    let provider = req.get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let title = req.get("title")
        .and_then(|v| v.as_str()).map_or_else(|| {
            // Extract title from URL
            url.split('/').next_back().unwrap_or("Unknown").to_string()
        }, |t| t.chars().take(500).collect::<String>());

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
    let proto_req = RemoveMediaRequest { media_id: media_id.clone() };
    let response = state
        .client_api
        .remove_media(&auth.user_id.to_string(), &room_id, proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    // Kick active stream for deleted media (local + cluster-wide)
    super::kick_stream_cluster(&state, &room_id, &media_id, "media_deleted");

    Ok(Json(response))
}

/// Bulk remove media from playlist
///
/// Removes multiple media items in a single transaction
#[utoipa::path(
    delete,
    path = "/api/rooms/{room_id}/media/batch",
    tag = "Room Media",
    params(
        ("room_id" = String, Path, description = "Room ID")
    ),
    request_body(
        content = inline(serde_json::Value),
        description = "JSON object with media_ids array",
        example = json!({"media_ids": ["abc123", "def456", "ghi789"]})
    ),
    responses(
        (status = 200, description = "Media items removed successfully",
         body = inline(serde_json::Value),
         example = json!({"deleted_count": 3})),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - insufficient permissions"),
        (status = 404, description = "Room not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "http_remove_media_batch", skip(state, req), fields(user_id = %auth.user_id, room_id = %room_id))]
pub async fn remove_media_batch(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let media_ids = req.get("media_ids")
        .and_then(|v| v.as_array())
        .ok_or_else(|| super::AppError::bad_request("Missing or invalid media_ids array"))?;

    let media_ids_str: Vec<String> = media_ids
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();

    if media_ids_str.is_empty() {
        return Err(super::AppError::bad_request("media_ids array cannot be empty"));
    }

    tracing::info!(
        user_id = %auth.user_id,
        room_id = %room_id,
        count = media_ids_str.len(),
        "Removing media batch"
    );

    // Clone media_ids for kick logic after batch deletion
    let media_ids_for_kick = media_ids_str.clone();

    let deleted_count = state
        .client_api
        .remove_media_batch(&auth.user_id.to_string(), &room_id, media_ids_str)
        .await
        .map_err(|e| {
            tracing::error!(
                user_id = %auth.user_id,
                room_id = %room_id,
                error = %e,
                "Failed to remove media batch"
            );
            super::AppError::internal_server_error(e)
        })?;

    tracing::info!(
        user_id = %auth.user_id,
        room_id = %room_id,
        deleted_count,
        "Media batch removed successfully"
    );

    // Kick active streams for deleted media (local + cluster-wide)
    for media_id in &media_ids_for_kick {
        super::kick_stream_cluster(&state, &room_id, media_id, "media_deleted");
    }

    Ok(Json(serde_json::json!({
        "deleted_count": deleted_count
    })))
}

/// Bulk reorder media items in playlist
///
/// Reorders multiple media items to new positions in a single transaction
#[utoipa::path(
    post,
    path = "/api/rooms/{room_id}/media/reorder",
    tag = "Room Media",
    params(
        ("room_id" = String, Path, description = "Room ID")
    ),
    request_body(
        content = inline(serde_json::Value),
        description = "JSON object with updates array containing {media_id, position} pairs",
        example = json!({
            "updates": [
                {"media_id": "abc123", "position": 0},
                {"media_id": "def456", "position": 1},
                {"media_id": "ghi789", "position": 2}
            ]
        })
    ),
    responses(
        (status = 200, description = "Media items reordered successfully",
         body = inline(serde_json::Value),
         example = json!({"success": true})),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - insufficient permissions"),
        (status = 404, description = "Room not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "http_reorder_media_batch", skip(state, req), fields(user_id = %auth.user_id, room_id = %room_id))]
pub async fn reorder_media_batch(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let updates = req.get("updates")
        .and_then(|v| v.as_array())
        .ok_or_else(|| super::AppError::bad_request("Missing or invalid updates array"))?;

    let mut updates_vec = Vec::new();
    for update in updates {
        let media_id = update.get("media_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| super::AppError::bad_request("Missing media_id in update"))?
            .to_string();

        let position_raw = update.get("position")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| super::AppError::bad_request("Missing or invalid position in update"))?;
        if position_raw < 0 || position_raw > i32::MAX as i64 {
            return Err(super::AppError::bad_request("Position must be a non-negative integer within i32 range"));
        }
        let position = position_raw as i32;

        updates_vec.push((media_id, position));
    }

    if updates_vec.is_empty() {
        return Err(super::AppError::bad_request("updates array cannot be empty"));
    }

    tracing::info!(
        user_id = %auth.user_id,
        room_id = %room_id,
        count = updates_vec.len(),
        "Reordering media batch"
    );

    state
        .client_api
        .reorder_media_batch(&auth.user_id.to_string(), &room_id, updates_vec)
        .await
        .map_err(|e| {
            tracing::error!(
                user_id = %auth.user_id,
                room_id = %room_id,
                error = %e,
                "Failed to reorder media batch"
            );
            super::AppError::internal_server_error(e)
        })?;

    tracing::info!(
        user_id = %auth.user_id,
        room_id = %room_id,
        "Media batch reordered successfully"
    );

    Ok(Json(serde_json::json!({
        "success": true
    })))
}

/// Get playlist
pub async fn get_playlist(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<ListPlaylistResponse>> {
    let room_id_obj = RoomId::from_string(room_id.clone());
    state.room_service
        .check_membership(&room_id_obj, &auth.user_id)
        .await
        .map_err(|e| super::AppError::forbidden(e.to_string()))?;

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

/// Get playback state
pub async fn get_playback_state(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<GetPlaybackStateResponse>> {
    let room_id_obj = RoomId::from_string(room_id.clone());
    state.room_service
        .check_membership(&room_id_obj, &auth.user_id)
        .await
        .map_err(|e| super::AppError::forbidden(e.to_string()))?;

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
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<GetRoomMembersResponse>> {
    let room_id_obj = RoomId::from_string(room_id.clone());
    state.room_service
        .check_membership(&room_id_obj, &auth.user_id)
        .await
        .map_err(|e| super::AppError::forbidden(e.to_string()))?;

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
                .map_err(|e| super::AppError::internal_server_error(format!("Failed to get room settings: {e}")))?;
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
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> AppResult<Json<ListRoomsResponse>> {
    let page: i32 = params.get("page").and_then(|v| v.parse().ok()).unwrap_or(1).max(1);
    let page_size: i32 = params.get("page_size").and_then(|v| v.parse().ok()).unwrap_or(50).clamp(1, 100);
    let search = params.get("search").cloned().unwrap_or_default();

    let proto_req = ListRoomsRequest {
        page,
        page_size,
        search,
    };
    let response = state
        .client_api
        .list_rooms(proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

// ==================== Room Settings Endpoints ====================

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

/// Check room password (requires authentication to prevent brute force)
pub async fn check_password(
    _auth: AuthUser,
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

    // Get title from request
    let title = req.get("title")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    // Edit media (permission check is done inside service)
    // Note: metadata is no longer stored in database, only in PlaybackResult
    let media = state
        .room_service
        .edit_media(room_id_obj, auth.user_id, media_id_obj, title)
        .await
        .map_err(|e| super::AppError::internal_server_error(format!("Failed to edit media: {e}")))?;

    // Convert to response
    let media_url = media.source_config.get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Get metadata from PlaybackResult if available (for direct URLs)
    let metadata_bytes = if media.is_direct() {
        media
            .get_playback_result()
            .map(|pb| serde_json::to_vec(&pb.metadata).unwrap_or_default())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(Json(AddMediaResponse {
        media: Some(crate::proto::client::Media {
            id: media.id.as_str().to_string(),
            room_id: media.room_id.as_str().to_string(),
            url: media_url.to_string(),
            provider: media.source_provider.clone(),
            title: media.name.clone(),
            metadata: metadata_bytes,
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

    // Check CLEAR_PLAYLIST permission
    state
        .room_service
        .check_permission(&room_id_obj, &auth.user_id, PermissionBits::CLEAR_PLAYLIST)
        .await?;

    let deleted_count = state.room_service.clear_playlist(room_id_obj, auth.user_id).await
        .map_err(|e| super::AppError::internal_server_error(format!("Failed to clear playlist: {e}")))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "deleted_count": deleted_count,
        "message": "Playlist cleared successfully"
    })))
}

/// GET /`api/rooms/:room_id/movie/:media_id` - Get movie playback info
pub async fn get_movie_info(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((room_id, media_id)): Path<(String, String)>,
) -> AppResult<Json<crate::proto::client::GetMovieInfoResponse>> {
    let resp = state
        .client_api
        .get_movie_info(
            auth.user_id.as_str(),
            &room_id,
            &media_id,
            &state.bilibili_provider,
            &state.alist_provider,
            &state.emby_provider,
            state.settings_registry.as_deref(),
        )
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(resp))
}

// ==================== New RESTful Endpoints ====================

/// Unified handler for listing rooms (with query params) or getting single room by ID
/// GET /api/rooms (list) or GET /api/rooms?id=xxx (single)
pub async fn list_or_get_rooms(
    _auth: Option<AuthUser>,
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> AppResult<Json<ListRoomsResponse>> {
    // List rooms with optional filtering
    let search = params.get("search").cloned().unwrap_or_default();
    let limit: i32 = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(50).clamp(1, 100);
    let offset: i32 = params.get("offset").and_then(|s| s.parse().ok()).unwrap_or(0).max(0);

    let request = ListRoomsRequest {
        page: (offset / limit) + 1,
        page_size: limit,
        search,
    };

    let response = state
        .client_api
        .list_rooms(request)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Unified handler for updating room settings via PATCH
/// PATCH /`api/rooms/:room_id/settings`
pub async fn update_room_settings(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    // Parse settings from request body
    let password = req.get("password").and_then(|v| v.as_str()).map(String::from);
    let max_members = req.get("max_members").and_then(serde_json::Value::as_i64).map(|v| v.clamp(0, 10000) as i32);
    let allow_guest_join = req.get("allow_guest_join").and_then(serde_json::Value::as_bool);
    let auto_play_next = req.get("auto_play_next").and_then(serde_json::Value::as_bool);
    let loop_playlist = req.get("loop_playlist").and_then(serde_json::Value::as_bool);
    let shuffle_playlist = req.get("shuffle_playlist").and_then(serde_json::Value::as_bool);
    let chat_enabled = req.get("chat_enabled").and_then(serde_json::Value::as_bool);
    let danmaku_enabled = req.get("danmaku_enabled").and_then(serde_json::Value::as_bool);

    let settings_bytes = build_room_settings_bytes(
        &password,
        max_members,
        allow_guest_join,
        auto_play_next,
        loop_playlist,
        shuffle_playlist,
        chat_enabled,
        danmaku_enabled,
    ).map_err(super::AppError::bad_request)?;

    let proto_req = UpdateRoomSettingsRequest {
        room_id: room_id.clone(),
        settings: settings_bytes,
    };

    let _response = state
        .client_api
        .update_room_settings(&auth.user_id.to_string(), &room_id, proto_req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(serde_json::json!({
        "message": "Room settings updated successfully",
        "room_id": room_id
    })))
}

/// Unified handler for updating playback state via PATCH
/// PATCH /`api/rooms/:room_id/playback`
/// Supports: state (play/pause), position (seek), speed, `media_id` (switch)
pub async fn update_playback(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    // Check which playback property to update

    // Handle state change (play/pause)
    if let Some(state_str) = req.get("state").and_then(|v| v.as_str()) {
        match state_str {
            "playing" => {
                let request = PlayRequest {};
                let _response = state
                    .client_api
                    .play(&auth.user_id.to_string(), &room_id, request)
                    .await
                    .map_err(super::AppError::internal_server_error)?;

                return Ok(Json(serde_json::json!({
                    "message": "Playback started",
                    "state": "playing"
                })));
            }
            "paused" => {
                let _response = state
                    .client_api
                    .pause(&auth.user_id.to_string(), &room_id)
                    .await
                    .map_err(super::AppError::internal_server_error)?;

                return Ok(Json(serde_json::json!({
                    "message": "Playback paused",
                    "state": "paused"
                })));
            }
            _ => return Err(super::AppError::bad_request("Invalid state value, use 'playing' or 'paused'")),
        }
    }

    // Handle position change (seek)
    if let Some(position) = req.get("position").and_then(serde_json::Value::as_f64) {
        let request = SeekRequest {
            position,
        };

        let _response = state
            .client_api
            .seek(&auth.user_id.to_string(), &room_id, request)
            .await
            .map_err(super::AppError::internal_server_error)?;

        return Ok(Json(serde_json::json!({
            "message": "Playback position updated",
            "position": position
        })));
    }

    // Handle speed change
    if let Some(speed) = req.get("speed").and_then(serde_json::Value::as_f64) {
        use crate::proto::client::SetPlaybackSpeedRequest;
        let request = SetPlaybackSpeedRequest {
            speed,
        };

        let _response = state
            .client_api
            .set_playback_speed(&auth.user_id.to_string(), &room_id, request)
            .await
            .map_err(super::AppError::internal_server_error)?;

        return Ok(Json(serde_json::json!({
            "message": "Playback speed updated",
            "speed": speed
        })));
    }

    // Handle media switch
    if let Some(media_id) = req.get("media_id").and_then(|v| v.as_str()) {
        use crate::proto::client::SetCurrentMediaRequest;
        let request = SetCurrentMediaRequest {
            playlist_id: String::new(), // Not used for direct media switch
            media_id: media_id.to_string(),
        };

        let _response = state
            .client_api
            .set_current_media(&auth.user_id.to_string(), &room_id, request)
            .await
            .map_err(super::AppError::internal_server_error)?;

        return Ok(Json(serde_json::json!({
            "message": "Switched to new media",
            "media_id": media_id
        })));
    }

    Err(super::AppError::bad_request(
        "No valid playback update field provided (state, position, speed, or media_id)"
    ))
}

/// Unified handler for media batch operations via PATCH
/// PATCH /`api/rooms/:room_id/media`
/// Supports: reorder, swap operations
pub async fn update_media_batch(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    // Check for reorder operation
    if let Some(updates) = req.get("reorder").and_then(|v| v.as_array()) {
        let mut updates_vec = Vec::new();
        for update in updates {
            let media_id = update.get("media_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| super::AppError::bad_request("Missing media_id in reorder update"))?
                .to_string();

            let position = update.get("position")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| super::AppError::bad_request("Missing or invalid position in reorder update"))?
                as i32;

            updates_vec.push((media_id, position));
        }

        if updates_vec.is_empty() {
            return Err(super::AppError::bad_request("reorder array cannot be empty"));
        }

        let count = updates_vec.len();
        state
            .client_api
            .reorder_media_batch(&auth.user_id.to_string(), &room_id, updates_vec)
            .await
            .map_err(super::AppError::internal_server_error)?;

        return Ok(Json(serde_json::json!({
            "message": "Media reordered successfully",
            "count": count
        })));
    }

    // Check for swap operation
    if let Some(swap_data) = req.get("swap") {
        let media_id1 = swap_data.get("media_id1")
            .and_then(|v| v.as_str())
            .ok_or_else(|| super::AppError::bad_request("Missing media_id1 in swap operation"))?;

        let media_id2 = swap_data.get("media_id2")
            .and_then(|v| v.as_str())
            .ok_or_else(|| super::AppError::bad_request("Missing media_id2 in swap operation"))?;

        let request = SwapMediaRequest {
            room_id: room_id.clone(),
            media_id1: media_id1.to_string(),
            media_id2: media_id2.to_string(),
        };

        let _response = state
            .client_api
            .swap_media(&auth.user_id.to_string(), request)
            .await
            .map_err(super::AppError::internal_server_error)?;

        return Ok(Json(serde_json::json!({
            "message": "Media items swapped successfully"
        })));
    }

    Err(super::AppError::bad_request(
        "No valid batch operation provided (reorder or swap)"
    ))
}
