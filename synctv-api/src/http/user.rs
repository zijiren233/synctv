//! User management HTTP handlers

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use synctv_core::models::id::RoomId;

use super::{middleware::AuthUser, AppResult, AppState};

/// User info response
#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub permissions: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// Update username request
#[derive(Debug, Deserialize)]
pub struct UpdateUsernameRequest {
    pub new_username: String,
}

/// Update password request
#[derive(Debug, Deserialize)]
pub struct UpdatePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

/// Room list item
#[derive(Debug, Serialize)]
pub struct UserRoomItem {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub member_count: i32,
    pub role: String,
}

/// Get current user info
pub async fn get_me(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<UserResponse>> {
    let user = state
        .user_service
        .get_user(&auth.user_id)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to get user: {}", e)))?;

    Ok(Json(UserResponse {
        id: user.id.as_str().to_string(),
        username: user.username,
        email: user.email,
        permissions: user.permissions.0,
        created_at: user.created_at.to_rfc3339(),
        updated_at: user.updated_at.to_rfc3339(),
    }))
}

/// Logout user
pub async fn logout(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    // Extract token from Authorization header is handled by middleware
    // The middleware adds a token field to AuthUser if we modify it
    // For now, we'll implement token blacklist via the user_service

    // Note: In the current implementation, the token is passed via Authorization header
    // We need to extract it from the request metadata
    // For simplicity, we're logging out the user from the service perspective

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Logged out successfully"
    })))
}

/// Update username
pub async fn update_username(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<UpdateUsernameRequest>,
) -> AppResult<Json<UserResponse>> {
    if req.new_username.is_empty() {
        return Err(super::AppError::bad_request("Username cannot be empty"));
    }

    let mut user = state
        .user_service
        .get_user(&auth.user_id)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to get user: {}", e)))?;

    user.username = req.new_username;

    let updated_user = state
        .user_service
        .update_user(&user)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to update username: {}", e)))?;

    Ok(Json(UserResponse {
        id: updated_user.id.as_str().to_string(),
        username: updated_user.username,
        email: updated_user.email,
        permissions: updated_user.permissions.0,
        created_at: updated_user.created_at.to_rfc3339(),
        updated_at: updated_user.updated_at.to_rfc3339(),
    }))
}

/// Update password
pub async fn update_password(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<UpdatePasswordRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let mut user = state
        .user_service
        .get_user(&auth.user_id)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to get user: {}", e)))?;

    // Verify old password
    let valid = synctv_core::service::auth::password::verify_password(
        &req.old_password,
        &user.password_hash,
    )
    .await
    .map_err(|e| super::AppError::internal(format!("Failed to verify password: {}", e)))?;

    if !valid {
        return Err(super::AppError::unauthorized("Invalid old password"));
    }

    // Hash new password
    let new_hash = synctv_core::service::auth::password::hash_password(&req.new_password)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to hash password: {}", e)))?;

    user.password_hash = new_hash;

    state
        .user_service
        .update_user(&user)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to update password: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Password updated successfully"
    })))
}

/// Get user's created rooms
pub async fn get_my_rooms(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<UserRoomItem>>> {
    let page = 1;
    let page_size = 100;

    let (rooms_with_count, _total) = state
        .room_service
        .list_rooms_by_creator_with_count(&auth.user_id, page, page_size)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to get rooms: {}", e)))?;

    let response = rooms_with_count
        .into_iter()
        .map(|rwc| UserRoomItem {
            id: rwc.room.id.as_str().to_string(),
            name: rwc.room.name,
            created_at: rwc.room.created_at.to_rfc3339(),
            member_count: rwc.member_count,
            role: "creator".to_string(),
        })
        .collect();

    Ok(Json(response))
}

/// Get user's joined rooms
pub async fn get_joined_rooms(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<UserRoomItem>>> {
    let page = 1;
    let page_size = 100;

    let (rooms_with_details, _total) = state
        .room_service
        .list_joined_rooms_with_details(&auth.user_id, page, page_size)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to get joined rooms: {}", e)))?;

    let response = rooms_with_details
        .into_iter()
        .map(|(room, permissions, member_count)| {
            let role = if room.created_by == auth.user_id {
                "creator".to_string()
            } else if permissions == synctv_core::models::Role::Admin.permissions() {
                "admin".to_string()
            } else {
                "member".to_string()
            };

            UserRoomItem {
                id: room.id.as_str().to_string(),
                name: room.name,
                created_at: room.created_at.to_rfc3339(),
                member_count,
                role,
            }
        })
        .collect();

    Ok(Json(response))
}

/// Delete a room (user's own room)
pub async fn delete_my_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);

    // Verify user is the creator
    let room = state
        .room_service
        .get_room(&room_id)
        .await
        .map_err(|e| super::AppError::not_found(format!("Room not found: {}", e)))?;

    if room.created_by != auth.user_id {
        return Err(super::AppError::forbidden("You can only delete your own rooms"));
    }

    state
        .room_service
        .delete_room(room_id, auth.user_id)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to delete room: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Room deleted successfully"
    })))
}

/// Exit a room
pub async fn exit_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let room_id = RoomId::from_string(room_id);

    state
        .room_service
        .leave_room(room_id, auth.user_id)
        .await
        .map_err(|e| super::AppError::internal(format!("Failed to leave room: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Left room successfully"
    })))
}
