//! Email verification and password reset endpoints
//!
//! Public endpoints for email verification and password recovery.
//! Delegates to shared `EmailApiImpl` to avoid duplicating logic with gRPC handlers.

use axum::{
    extract::State,
    response::Json,
    routing::post,
    Router,
};
use serde::{Deserialize, Serialize};

use crate::http::{AppState, AppError, AppResult};
use crate::impls::EmailApiImpl;

/// Email verification request
#[derive(Debug, Deserialize)]
pub struct EmailVerificationRequest {
    pub email: String,
}

/// Email verification response
#[derive(Debug, Serialize)]
pub struct EmailVerificationResponse {
    pub message: String,
}

/// Password reset request
#[derive(Debug, Deserialize)]
pub struct PasswordResetRequest {
    pub email: String,
}

/// Password reset confirmation
#[derive(Debug, Deserialize)]
pub struct PasswordResetConfirm {
    pub email: String,
    pub token: String,
    pub new_password: String,
}

/// Password reset response
#[derive(Debug, Serialize)]
pub struct PasswordResetResponse {
    pub message: String,
}

/// Email verification confirmation request
#[derive(Debug, Deserialize)]
pub struct EmailVerificationConfirm {
    pub email: String,
    pub token: String,
}

/// Build an `EmailApiImpl` from `AppState`, or return an error if email is not configured.
fn require_email_api(state: &AppState) -> Result<EmailApiImpl, AppError> {
    let email_service = state
        .email_service
        .as_ref()
        .ok_or_else(|| AppError::bad_request("Email service not configured"))?;

    // Build the email token service from the user service pool
    let email_token_service = std::sync::Arc::new(
        synctv_core::service::EmailTokenService::new(state.user_service.pool().clone()),
    );

    Ok(EmailApiImpl::new(
        state.user_service.clone(),
        email_service.clone(),
        email_token_service,
    ))
}

/// Create email-related routes
///
/// Rate limiting is applied externally in `create_router` where `AppState` is available.
pub fn create_email_router() -> Router<AppState> {
    Router::new()
        .route("/api/email/verify/send", post(send_verification_email))
        .route("/api/email/verify/confirm", post(confirm_email))
        .route("/api/email/password/reset", post(request_password_reset))
        .route("/api/email/password/confirm", post(confirm_password_reset))
}

/// Send verification email
///
/// POST /api/email/verify/send
/// Public endpoint - no authentication required
pub async fn send_verification_email(
    State(state): State<AppState>,
    Json(req): Json<EmailVerificationRequest>,
) -> AppResult<Json<EmailVerificationResponse>> {
    let email_api = require_email_api(&state)?;

    let result = email_api
        .send_verification_email(&req.email)
        .await
        .map_err(AppError::internal_server_error)?;

    Ok(Json(EmailVerificationResponse {
        message: result.message,
    }))
}

/// Confirm email verification
///
/// POST /api/email/verify/confirm
/// Public endpoint - no authentication required
pub async fn confirm_email(
    State(state): State<AppState>,
    Json(req): Json<EmailVerificationConfirm>,
) -> AppResult<Json<serde_json::Value>> {
    let email_api = require_email_api(&state)?;

    let result = email_api
        .confirm_email(&req.email, &req.token)
        .await
        .map_err(AppError::bad_request)?;

    Ok(Json(serde_json::json!({
        "message": result.message,
    })))
}

/// Request password reset
///
/// POST /api/email/password/reset
/// Public endpoint - no authentication required
pub async fn request_password_reset(
    State(state): State<AppState>,
    Json(req): Json<PasswordResetRequest>,
) -> AppResult<Json<PasswordResetResponse>> {
    let email_api = require_email_api(&state)?;

    let result = email_api
        .request_password_reset(&req.email)
        .await
        .map_err(AppError::internal_server_error)?;

    Ok(Json(PasswordResetResponse {
        message: result.message,
    }))
}

/// Confirm password reset
///
/// POST /api/email/password/confirm
/// Public endpoint - no authentication required
pub async fn confirm_password_reset(
    State(state): State<AppState>,
    Json(req): Json<PasswordResetConfirm>,
) -> AppResult<Json<serde_json::Value>> {
    let email_api = require_email_api(&state)?;

    let result = email_api
        .confirm_password_reset(&req.email, &req.token, &req.new_password)
        .await
        .map_err(AppError::bad_request)?;

    Ok(Json(serde_json::json!({
        "message": result.message,
    })))
}
