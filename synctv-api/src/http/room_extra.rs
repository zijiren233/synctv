//! Additional room management API endpoints

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::http::{AppState, AppResult, AppError, middleware::AuthUser};
use synctv_core::{
    models::{RoomId, RoomSettings, UserId},
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
        .map_err(|e| AppError::bad_request(format!("Invalid settings: {e}")))?;

    state
        .room_service
        .set_settings(room_id, user_id, settings)
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
        permissions: member.effective_permissions(synctv_core::models::PermissionBits::empty()).0 as i64,
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

// ------------------------------------------------------------------
// Room Member Management (room-scoped, requires room-level permissions)
// ------------------------------------------------------------------

/// Kick a member from a room
pub async fn kick_member(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((room_id, target_user_id)): Path<(String, String)>,
) -> AppResult<Json<crate::proto::client::KickMemberResponse>> {
    let resp = state
        .client_api
        .kick_member(
            auth.user_id.as_str(),
            &room_id,
            crate::proto::client::KickMemberRequest {
                user_id: target_user_id,
            },
        )
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

/// Set member permissions / role
#[derive(Debug, Deserialize)]
pub struct SetMemberPermissionsRequest {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub added_permissions: u64,
    #[serde(default)]
    pub removed_permissions: u64,
    #[serde(default)]
    pub admin_added_permissions: u64,
    #[serde(default)]
    pub admin_removed_permissions: u64,
}

pub async fn set_member_permissions(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((room_id, target_user_id)): Path<(String, String)>,
    Json(req): Json<SetMemberPermissionsRequest>,
) -> AppResult<Json<crate::proto::client::SetMemberPermissionResponse>> {
    let resp = state
        .client_api
        .update_member_permission(
            auth.user_id.as_str(),
            &room_id,
            crate::proto::client::SetMemberPermissionRequest {
                user_id: target_user_id,
                role: req.role,
                added_permissions: req.added_permissions,
                removed_permissions: req.removed_permissions,
                admin_added_permissions: req.admin_added_permissions,
                admin_removed_permissions: req.admin_removed_permissions,
            },
        )
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

/// Ban a member from a room
pub async fn ban_member(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((room_id, target_user_id)): Path<(String, String)>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<crate::proto::client::BanMemberResponse>> {
    let reason = req.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string();

    let resp = state
        .client_api
        .ban_member(
            auth.user_id.as_str(),
            &room_id,
            crate::proto::client::BanMemberRequest {
                user_id: target_user_id,
                reason,
            },
        )
        .await
        .map_err(AppError::internal)?;

    Ok(Json(resp))
}

/// Unban a member from a room
pub async fn unban_member(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((room_id, target_user_id)): Path<(String, String)>,
) -> AppResult<Json<crate::proto::client::UnbanMemberResponse>> {
    let resp = state
        .client_api
        .unban_member(
            auth.user_id.as_str(),
            &room_id,
            crate::proto::client::UnbanMemberRequest {
                user_id: target_user_id,
            },
        )
        .await
        .map_err(AppError::internal)?;

    Ok(Json(resp))
}
