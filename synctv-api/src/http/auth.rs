// Authentication HTTP handlers

use axum::{
    extract::State,
    Json,
};
use serde::{Deserialize, Serialize};
use synctv_core::models::id::UserId;

use super::{AppState, AppResult};

/// Register request
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    pub email: Option<String>,
}

/// Login request
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Token refresh request
#[derive(Debug, Deserialize)]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

/// Authentication response
#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub user_id: String,
    pub username: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

/// Register a new user
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> AppResult<Json<AuthResponse>> {
    // Validate input
    if req.username.is_empty() {
        return Err(super::AppError::bad_request("Username cannot be empty"));
    }
    if req.password.len() < 6 {
        return Err(super::AppError::bad_request("Password must be at least 6 characters"));
    }

    let email = req.email.unwrap_or_else(|| format!("{}@temp.local", req.username));

    // Register user (returns tuple: (User, access_token, refresh_token))
    let (user, access_token, refresh_token) = state.user_service
        .register(req.username, email, req.password)
        .await?;

    Ok(Json(AuthResponse {
        user_id: user.id.as_str().to_string(),
        username: user.username,
        access_token,
        refresh_token,
        expires_in: 3600, // 1 hour
    }))
}

/// Login with username and password
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> AppResult<Json<AuthResponse>> {
    // Login user (returns tuple: (User, access_token, refresh_token))
    let (user, access_token, refresh_token) = state.user_service
        .login(req.username, req.password)
        .await?;

    Ok(Json(AuthResponse {
        user_id: user.id.as_str().to_string(),
        username: user.username,
        access_token,
        refresh_token,
        expires_in: 3600, // 1 hour
    }))
}

/// Refresh access token using refresh token
pub async fn refresh_token(
    State(state): State<AppState>,
    Json(req): Json<RefreshTokenRequest>,
) -> AppResult<Json<AuthResponse>> {
    // Refresh tokens (returns tuple: (new_access_token, new_refresh_token))
    let (access_token, refresh_token) = state.user_service
        .refresh_token(req.refresh_token)
        .await?;

    // Extract user_id from new access token to get user info
    let user_id_str = extract_user_id_from_token(&access_token)
        .map_err(|e| super::AppError::internal_server_error(format!("Failed to extract user_id: {}", e)))?;
    let user_id = UserId::from_string(user_id_str);

    // Get user info
    let user = state.user_service
        .get_user(&user_id)
        .await?;

    Ok(Json(AuthResponse {
        user_id: user.id.as_str().to_string(),
        username: user.username,
        access_token,
        refresh_token,
        expires_in: 3600, // 1 hour
    }))
}

/// Extract user_id from JWT token (simple version, no verification)
fn extract_user_id_from_token(token: &str) -> Result<String, String> {
    use base64::prelude::*;

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid token format".to_string());
    }

    let payload = parts[1];
    let decoded = BASE64_URL_SAFE_NO_PAD.decode(payload)
        .map_err(|e| format!("Failed to decode token: {}", e))?;

    let json: serde_json::Value = serde_json::from_slice(&decoded)
        .map_err(|e| format!("Failed to parse token JSON: {}", e))?;

    let user_id = json.get("sub")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing sub claim".to_string())?;

    Ok(user_id.to_string())
}
