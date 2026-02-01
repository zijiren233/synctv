//! Additional room management API endpoints

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::http::{AppState, AppResult, AppError};
use synctv_core::{
    models::{RoomId, RoomSettings, UserId},
    service::RoomService,
};

/// Update room settings
#[derive(Debug, Deserialize)]
pub struct UpdateRoomSettingsRequest {
    pub user_id: String,
    pub settings: serde_json::Value,
}

pub async fn update_room_settings(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<UpdateRoomSettingsRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);
    let user_id = UserId::from_string(req.user_id);

    // Parse JSON value into RoomSettings
    let settings: RoomSettings = serde_json::from_value(req.settings)
        .map_err(|e| AppError::bad_request(format!("Invalid settings: {}", e)))?;

    state
        .room_service
        .update_settings(room_id, user_id, settings)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;

    Ok(Json(serde_json::json!({"success": true})))
}

/// Get current user's info in room
#[derive(Debug, Serialize)]
pub struct RoomMeResponse {
    pub room_id: String,
    pub user_id: String,
    pub permissions: i64,
}

pub async fn get_room_me(
    State(state): State<AppState>,
    Path((room_id, user_id)): Path<(String, String)>,
) -> AppResult<Json<RoomMeResponse>> {
    let room_id = RoomId::from_string(room_id);
    let user_id = UserId::from_string(user_id);

    let member = state
        .room_service
        .member_service()
        .get_member(&room_id, &user_id)
        .await?
        .ok_or_else(|| AppError::not_found("Not a member of this room"))?;

    Ok(Json(RoomMeResponse {
        room_id: room_id.as_str().to_string(),
        user_id: user_id.as_str().to_string(),
        permissions: member.permissions.0,
    }))
}

/// Get user's joined rooms
pub async fn get_joined_rooms(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> AppResult<Json<Vec<serde_json::Value>>> {
    let user_id = UserId::from_string(user_id);

    let (room_ids, _total) = state
        .room_service
        .member_service()
        .list_user_rooms(&user_id, 0, 100)
        .await?;

    // Get room details for each room
    let mut rooms = Vec::new();
    for room_id in room_ids {
        if let Ok(room) = state.room_service.get_room(&room_id).await {
            rooms.push(serde_json::json!({
                "room_id": room.id.as_str(),
                "name": room.name,
                "created_at": room.created_at.timestamp(),
            }));
        }
    }

    Ok(Json(rooms))
}
