//! User management HTTP handlers
//
// This layer now uses proto types and delegates to the impls layer for business logic

use axum::{
    extract::{Path, Query, State},
    Json,
};
use synctv_core::models::id::RoomId;

use super::{middleware::AuthUser, AppResult, AppState};
use crate::proto::client::{
    LogoutRequest, LogoutResponse, GetProfileResponse, SetUsernameRequest,
    SetPasswordRequest, ListParticipatedRoomsResponse,
    DeleteRoomResponse,
    ListCreatedRoomsResponse,
};

/// Logout user
pub async fn logout(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<LogoutResponse>> {
    // Note: The user_id is available in auth.user_id but the proto LogoutRequest doesn't use it
    // Logout is primarily handled client-side by deleting the token
    let response = state
        .client_api
        .logout(LogoutRequest {})
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Get current user info
pub async fn get_me(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<GetProfileResponse>> {
    let response = state
        .client_api
        .get_profile(&auth.user_id.to_string())
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Update user (unified endpoint for username and password via PATCH)
pub async fn update_user(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    // Check if username update is requested
    if let Some(username) = req.get("username").and_then(|v| v.as_str()) {
        if username.is_empty() {
            return Err(super::AppError::bad_request("Username cannot be empty"));
        }

        let set_username_req = SetUsernameRequest {
            new_username: username.to_string(),
        };

        let response = state
            .client_api
            .set_username(&auth.user_id.to_string(), set_username_req)
            .await
            .map_err(super::AppError::internal_server_error)?;

        // Extract username from user object
        let new_username = response.user.as_ref().map_or_else(|| username.to_string(), |u| u.username.clone());

        return Ok(Json(serde_json::json!({
            "message": "Username updated successfully",
            "username": new_username
        })));
    }

    // Check if password update is requested
    if let Some(password) = req.get("password").and_then(|v| v.as_str()) {
        // Get old password if provided (for security)
        let old_password = req.get("old_password")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let set_password_req = SetPasswordRequest {
            old_password,
            new_password: password.to_string(),
        };

        let _response = state
            .client_api
            .set_password(&auth.user_id.to_string(), set_password_req)
            .await
            .map_err(super::AppError::internal_server_error)?;

        return Ok(Json(serde_json::json!({
            "message": "Password updated successfully"
        })));
    }

    Err(super::AppError::bad_request("No valid update fields provided (username or password)"))
}

/// Get user's joined rooms
pub async fn get_joined_rooms(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<ListParticipatedRoomsResponse>> {
    let response = state
        .client_api
        .get_joined_rooms(&auth.user_id.to_string())
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Delete a room (user's own room)
pub async fn delete_my_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<DeleteRoomResponse>> {
    // Verify user is the creator first
    let room_id_obj = RoomId::from_string(room_id.clone());
    let room = state
        .room_service
        .get_room(&room_id_obj)
        .await
        .map_err(|e| super::AppError::not_found(format!("Room not found: {e}")))?;

    if room.created_by != auth.user_id {
        return Err(super::AppError::forbidden("You can only delete your own rooms"));
    }

    let response = state
        .client_api
        .delete_room(&auth.user_id.to_string(), &room_id)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// List rooms created by this user
/// GET /api/user/rooms/created
pub async fn list_created_rooms(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> AppResult<Json<ListCreatedRoomsResponse>> {
    let page = params.get("page").and_then(|v| v.parse().ok()).unwrap_or(1i32).max(1);
    let page_size = params.get("page_size").and_then(|v| v.parse().ok()).unwrap_or(10i32).clamp(1, 50);

    let req = crate::proto::client::ListCreatedRoomsRequest { page, page_size };
    let response = state
        .client_api
        .list_created_rooms(&auth.user_id.to_string(), req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}
