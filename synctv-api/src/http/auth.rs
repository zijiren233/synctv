// Authentication HTTP handlers
//
// This layer now uses proto types and delegates to the impls layer for business logic

use axum::{extract::State, Json};

use super::{AppResult, AppState};
use crate::proto::client::{RegisterRequest, RegisterResponse, LoginRequest, LoginResponse, RefreshTokenRequest, RefreshTokenResponse};

/// Register a new user
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> AppResult<Json<RegisterResponse>> {
    let response = state
        .client_api
        .register(req)
        .await
        .map_err(super::AppError::bad_request)?;

    Ok(Json(response))
}

/// Login with username and password
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> AppResult<Json<LoginResponse>> {
    let response = state
        .client_api
        .login(req)
        .await
        .map_err(super::AppError::unauthorized)?;

    Ok(Json(response))
}

/// Refresh access token using refresh token
pub async fn refresh_token(
    State(state): State<AppState>,
    Json(req): Json<RefreshTokenRequest>,
) -> AppResult<Json<RefreshTokenResponse>> {
    let response = state
        .client_api
        .refresh_token(req)
        .await
        .map_err(super::error::impls_err_to_app_error)?;

    Ok(Json(response))
}
