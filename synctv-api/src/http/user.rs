//! User management HTTP handlers
//
// This layer now uses proto types and delegates to the impls layer for business logic

use axum::{
    extract::{Path, State},
    Json,
};
use synctv_core::models::id::RoomId;

use super::{middleware::AuthUser, AppResult, AppState};
use crate::proto::client::{
    LogoutRequest, LogoutResponse, GetProfileResponse, SetUsernameRequest, SetUsernameResponse,
    SetPasswordRequest, SetPasswordResponse, ListParticipatedRoomsResponse,
    LeaveRoomRequest, LeaveRoomResponse, DeleteRoomRequest, DeleteRoomResponse,
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

/// Get current user info (equivalent to GetProfile)
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

/// Update username
pub async fn update_username(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<SetUsernameRequest>,
) -> AppResult<Json<SetUsernameResponse>> {
    if req.new_username.is_empty() {
        return Err(super::AppError::bad_request("Username cannot be empty"));
    }

    let response = state
        .client_api
        .set_username(&auth.user_id.to_string(), req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}

/// Update password
pub async fn update_password(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<SetPasswordRequest>,
) -> AppResult<Json<SetPasswordResponse>> {
    // Note: The proto SetPasswordRequest only has new_password, no old_password
    // In a real implementation, we'd want old_password verification
    // For now, we'll proceed with the new proto-based approach
    let response = state
        .client_api
        .set_password(&auth.user_id.to_string(), req)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
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

/// Exit a room
pub async fn exit_room(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<LeaveRoomResponse>> {
    let request = LeaveRoomRequest { room_id };
    let response = state
        .client_api
        .leave_room(&auth.user_id.to_string(), request)
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
        .map_err(|e| super::AppError::not_found(format!("Room not found: {}", e)))?;

    if room.created_by != auth.user_id {
        return Err(super::AppError::forbidden("You can only delete your own rooms"));
    }

    let request = DeleteRoomRequest { room_id };
    let response = state
        .client_api
        .delete_room(&auth.user_id.to_string(), request)
        .await
        .map_err(super::AppError::internal_server_error)?;

    Ok(Json(response))
}
