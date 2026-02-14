//! Admin HTTP handlers
//!
//! All admin routes require authentication and admin/root role.
//! Thin handlers that delegate to `AdminApiImpl`.

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path, Query, State},
    http::request::Parts,
    routing::{get, post, put},
    Json, Router,
};
use std::sync::Arc;
use synctv_core::models::id::UserId;
use synctv_core::service::auth::JwtValidator;

use synctv_core::models::UserStatus;

use super::{AppError, AppResult, AppState};
use crate::proto::admin;

// ------------------------------------------------------------------
// Auth extractors
// ------------------------------------------------------------------

/// Extension to hold JWT validator in request extensions (cached)
#[derive(Clone)]
struct JwtValidatorExt(Arc<JwtValidator>);

/// Authenticated admin user (admin or root role required)
#[derive(Debug, Clone)]
pub struct AuthAdmin {
    pub user_id: UserId,
    pub role: synctv_core::models::UserRole,
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthAdmin
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        // Use cached validator from extensions if available
        let validator = parts
            .extensions
            .get::<JwtValidatorExt>().map_or_else(|| {
                Arc::new(JwtValidator::new(Arc::new(app_state.jwt_service.clone())))
            }, |v| v.0.clone());

        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .ok_or_else(|| AppError::unauthorized("Missing Authorization header"))?;

        let auth_str = auth_header
            .to_str()
            .map_err(|e| AppError::unauthorized(format!("Invalid Authorization header: {e}")))?;

        let claims = validator
            .validate_http(auth_str)
            .map_err(|e| AppError::unauthorized(format!("{e}")))?;

        let user_id = UserId::from_string(claims.sub);

        // Verify admin role and check banned/deleted status
        let user = app_state
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|_| AppError::unauthorized("Failed to verify user"))?;

        if user.is_deleted() || user.status == UserStatus::Banned {
            return Err(AppError::unauthorized("Authentication failed"));
        }

        // Reject tokens issued before last password change
        if app_state
            .user_service
            .is_token_invalidated_by_password_change(&user_id, claims.iat)
            .await
            .unwrap_or(false)
        {
            return Err(AppError::unauthorized(
                "Token invalidated due to password change. Please log in again.",
            ));
        }

        if !user.role.is_admin_or_above() {
            return Err(AppError::forbidden("Admin role required"));
        }

        Ok(Self { user_id, role: user.role })
    }
}

/// Authenticated root user (root role only)
#[derive(Debug, Clone)]
pub struct AuthRoot {
    pub user_id: UserId,
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthRoot
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        // Use cached validator from extensions if available
        let validator = parts
            .extensions
            .get::<JwtValidatorExt>().map_or_else(|| {
                Arc::new(JwtValidator::new(Arc::new(app_state.jwt_service.clone())))
            }, |v| v.0.clone());

        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .ok_or_else(|| AppError::unauthorized("Missing Authorization header"))?;

        let auth_str = auth_header
            .to_str()
            .map_err(|e| AppError::unauthorized(format!("Invalid Authorization header: {e}")))?;

        let claims = validator
            .validate_http(auth_str)
            .map_err(|e| AppError::unauthorized(format!("{e}")))?;

        let user_id = UserId::from_string(claims.sub);

        // Check banned/deleted status
        let user = app_state
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|_| AppError::unauthorized("Failed to verify user"))?;

        if user.is_deleted() || user.status == UserStatus::Banned {
            return Err(AppError::unauthorized("Authentication failed"));
        }

        // Reject tokens issued before last password change
        if app_state
            .user_service
            .is_token_invalidated_by_password_change(&user_id, claims.iat)
            .await
            .unwrap_or(false)
        {
            return Err(AppError::unauthorized(
                "Token invalidated due to password change. Please log in again.",
            ));
        }

        if !matches!(user.role, synctv_core::models::UserRole::Root) {
            return Err(AppError::forbidden("Root role required"));
        }

        Ok(Self { user_id })
    }
}

// ------------------------------------------------------------------
// Typed request structs for admin endpoints
// ------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct SetUserRoleRequest {
    role: String,
}

#[derive(serde::Deserialize)]
struct SetUserPasswordRequest {
    password: String,
}

#[derive(serde::Deserialize)]
struct SetUserUsernameRequest {
    username: String,
}

#[derive(serde::Deserialize)]
struct BanRequest {
    #[serde(default)]
    reason: String,
}

#[derive(serde::Deserialize)]
struct SetRoomPasswordAdminRequest {
    #[serde(default)]
    password: String,
}

// ------------------------------------------------------------------
// Helper to get admin_api or 503
// ------------------------------------------------------------------

fn require_admin_api(state: &AppState) -> Result<&Arc<crate::impls::AdminApiImpl>, AppError> {
    state
        .admin_api
        .as_ref()
        .ok_or_else(|| AppError::internal("Admin service not configured"))
}

// ------------------------------------------------------------------
// Router
// ------------------------------------------------------------------

pub fn create_admin_router() -> Router<AppState> {
    Router::new()
        // System stats
        .route("/stats", get(get_system_stats))
        // Settings
        .route("/settings", get(get_settings).post(set_settings))
        .route("/settings/:group", get(get_settings_group))
        // Email
        .route("/email/test", post(send_test_email))
        // User management
        .route("/users", get(list_users).post(create_user))
        .route("/users/:user_id", get(get_user).delete(delete_user))
        .route("/users/:user_id/role", post(set_user_role))
        .route("/users/:user_id/password", post(set_user_password))
        .route("/users/:user_id/username", post(set_user_username))
        .route("/users/:user_id/ban", post(ban_user))
        .route("/users/:user_id/unban", post(unban_user))
        .route("/users/:user_id/approve", post(approve_user))
        .route("/users/:user_id/rooms", get(get_user_rooms))
        // Room management
        .route("/rooms", get(list_rooms))
        .route("/rooms/:room_id", get(get_room).delete(delete_room))
        .route("/rooms/:room_id/password", post(set_room_password))
        .route("/rooms/:room_id/members", get(get_room_members))
        .route("/rooms/:room_id/ban", post(ban_room))
        .route("/rooms/:room_id/unban", post(unban_room))
        .route("/rooms/:room_id/approve", post(approve_room))
        .route("/rooms/:room_id/settings", get(get_room_settings).post(set_room_settings))
        .route("/rooms/:room_id/settings/reset", post(reset_room_settings))
        // Provider instances
        .route("/providers", get(list_providers).post(add_provider))
        .route("/providers/:name", put(update_provider).delete(delete_provider))
        .route("/providers/:name/reconnect", post(reconnect_provider))
        .route("/providers/:name/enable", post(enable_provider))
        .route("/providers/:name/disable", post(disable_provider))
        // Admin management (root only)
        .route("/admins", get(list_admins))
        .route("/admins/:user_id", post(add_admin).delete(remove_admin))
}

// ------------------------------------------------------------------
// System Stats
// ------------------------------------------------------------------

async fn get_system_stats(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> AppResult<Json<admin::GetSystemStatsResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .get_system_stats(admin::GetSystemStatsRequest {})
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

// ------------------------------------------------------------------
// Settings
// ------------------------------------------------------------------

async fn get_settings(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> AppResult<Json<admin::GetSettingsResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .get_settings(admin::GetSettingsRequest {})
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn get_settings_group(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(group): Path<String>,
) -> AppResult<Json<admin::GetSettingsGroupResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .get_settings_group(admin::GetSettingsGroupRequest { group })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn set_settings(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Json(req): Json<admin::UpdateSettingsRequest>,
) -> AppResult<Json<admin::UpdateSettingsResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api.update_settings(req).await.map_err(AppError::internal)?;
    Ok(Json(resp))
}

// ------------------------------------------------------------------
// Email
// ------------------------------------------------------------------

async fn send_test_email(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Json(req): Json<admin::SendTestEmailRequest>,
) -> AppResult<Json<admin::SendTestEmailResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api.send_test_email(req).await.map_err(AppError::internal)?;
    Ok(Json(resp))
}

// ------------------------------------------------------------------
// User Management
// ------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
pub struct ListUsersQuery {
    pub page: Option<i32>,
    pub page_size: Option<i32>,
    pub status: Option<String>,
    pub role: Option<String>,
    pub search: Option<String>,
}

async fn list_users(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Query(q): Query<ListUsersQuery>,
) -> AppResult<Json<admin::ListUsersResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .list_users(admin::ListUsersRequest {
            page: q.page.unwrap_or(1).max(1),
            page_size: q.page_size.unwrap_or(20).clamp(1, 100),
            status: q.status.unwrap_or_default(),
            role: q.role.unwrap_or_default(),
            search: q.search.unwrap_or_default(),
        })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn get_user(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> AppResult<Json<admin::GetUserResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .get_user(admin::GetUserRequest { user_id })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn create_user(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(req): Json<admin::CreateUserRequest>,
) -> AppResult<Json<admin::CreateUserResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api.create_user(req, auth.role).await.map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn delete_user(
    _auth: AuthRoot,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> AppResult<Json<admin::DeleteUserResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .delete_user(admin::DeleteUserRequest { user_id })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn set_user_role(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Json(req): Json<SetUserRoleRequest>,
) -> AppResult<Json<admin::UpdateUserRoleResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .update_user_role(admin::UpdateUserRoleRequest { user_id, role: req.role }, auth.role)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn set_user_password(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Json(req): Json<SetUserPasswordRequest>,
) -> AppResult<Json<admin::UpdateUserPasswordResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .update_user_password(admin::UpdateUserPasswordRequest {
            user_id,
            new_password: req.password,
        })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn set_user_username(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Json(req): Json<SetUserUsernameRequest>,
) -> AppResult<Json<admin::UpdateUserUsernameResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .update_user_username(admin::UpdateUserUsernameRequest {
            user_id,
            new_username: req.username,
        })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn ban_user(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Json(req): Json<BanRequest>,
) -> AppResult<Json<admin::BanUserResponse>> {
    if req.reason.len() > 500 {
        return Err(AppError::bad_request("Reason too long (max 500 characters)"));
    }

    let api = require_admin_api(&state)?;
    let resp = api
        .ban_user(admin::BanUserRequest { user_id, reason: req.reason }, auth.role)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn unban_user(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> AppResult<Json<admin::UnbanUserResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .unban_user(admin::UnbanUserRequest { user_id })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn approve_user(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> AppResult<Json<admin::ApproveUserResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .approve_user(admin::ApproveUserRequest { user_id })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn get_user_rooms(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> AppResult<Json<admin::GetUserRoomsResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .get_user_rooms(admin::GetUserRoomsRequest { user_id })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

// ------------------------------------------------------------------
// Room Management
// ------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
pub struct ListRoomsQuery {
    pub page: Option<i32>,
    pub page_size: Option<i32>,
    pub status: Option<String>,
    pub search: Option<String>,
    pub creator_id: Option<String>,
}

async fn list_rooms(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Query(q): Query<ListRoomsQuery>,
) -> AppResult<Json<admin::ListRoomsResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .list_rooms(admin::ListRoomsRequest {
            page: q.page.unwrap_or(1).max(1),
            page_size: q.page_size.unwrap_or(20).clamp(1, 100),
            status: q.status.unwrap_or_default(),
            search: q.search.unwrap_or_default(),
            creator_id: q.creator_id.unwrap_or_default(),
        })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn get_room(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<admin::GetRoomResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .get_room(admin::GetRoomRequest { room_id })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn delete_room(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<admin::DeleteRoomResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .delete_room(admin::DeleteRoomRequest { room_id })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn set_room_password(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<SetRoomPasswordAdminRequest>,
) -> AppResult<Json<admin::UpdateRoomPasswordResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .update_room_password(admin::UpdateRoomPasswordRequest {
            room_id,
            new_password: req.password,
        })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn get_room_members(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<admin::GetRoomMembersResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .get_room_members(admin::GetRoomMembersRequest { room_id })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn ban_room(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<BanRequest>,
) -> AppResult<Json<admin::BanRoomResponse>> {
    if req.reason.len() > 500 {
        return Err(AppError::bad_request("Reason too long (max 500 characters)"));
    }

    let api = require_admin_api(&state)?;
    let resp = api
        .ban_room(admin::BanRoomRequest { room_id, reason: req.reason })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn unban_room(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<admin::UnbanRoomResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .unban_room(admin::UnbanRoomRequest { room_id })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn approve_room(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<admin::ApproveRoomResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .approve_room(admin::ApproveRoomRequest { room_id })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn get_room_settings(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<admin::GetRoomSettingsResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .get_room_settings(admin::GetRoomSettingsRequest { room_id })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn set_room_settings(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<admin::UpdateRoomSettingsResponse>> {
    let settings = serde_json::to_vec(&req)
        .map_err(|e| AppError::bad_request(format!("Invalid settings JSON: {e}")))?;

    let api = require_admin_api(&state)?;
    let resp = api
        .update_room_settings(admin::UpdateRoomSettingsRequest { room_id, settings })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn reset_room_settings(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<admin::ResetRoomSettingsResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .reset_room_settings(admin::ResetRoomSettingsRequest { room_id })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

// ------------------------------------------------------------------
// Provider Instances
// ------------------------------------------------------------------

async fn list_providers(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> AppResult<Json<admin::ListProviderInstancesResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .list_provider_instances(admin::ListProviderInstancesRequest {
            provider_type: String::new(),
        })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn add_provider(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Json(req): Json<admin::AddProviderInstanceRequest>,
) -> AppResult<Json<admin::AddProviderInstanceResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .add_provider_instance(req)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn update_provider(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(mut req): Json<admin::UpdateProviderInstanceRequest>,
) -> AppResult<Json<admin::UpdateProviderInstanceResponse>> {
    req.name = name;
    let api = require_admin_api(&state)?;
    let resp = api
        .update_provider_instance(req)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn delete_provider(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> AppResult<Json<admin::DeleteProviderInstanceResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .delete_provider_instance(admin::DeleteProviderInstanceRequest { name })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn reconnect_provider(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> AppResult<Json<admin::ReconnectProviderInstanceResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .reconnect_provider_instance(admin::ReconnectProviderInstanceRequest { name })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn enable_provider(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> AppResult<Json<admin::EnableProviderInstanceResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .enable_provider_instance(admin::EnableProviderInstanceRequest { name })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn disable_provider(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> AppResult<Json<admin::DisableProviderInstanceResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .disable_provider_instance(admin::DisableProviderInstanceRequest { name })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

// ------------------------------------------------------------------
// Admin Management (Root Only)
// ------------------------------------------------------------------

async fn list_admins(
    _auth: AuthRoot,
    State(state): State<AppState>,
) -> AppResult<Json<admin::ListAdminsResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .list_admins(admin::ListAdminsRequest {})
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn add_admin(
    _auth: AuthRoot,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> AppResult<Json<admin::AddAdminResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .add_admin(admin::AddAdminRequest { user_id })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}

async fn remove_admin(
    _auth: AuthRoot,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> AppResult<Json<admin::RemoveAdminResponse>> {
    let api = require_admin_api(&state)?;
    let resp = api
        .remove_admin(admin::RemoveAdminRequest { user_id })
        .await
        .map_err(AppError::internal)?;
    Ok(Json(resp))
}
