//! Email verification and password reset endpoints
//!
//! Public endpoints for email verification and password recovery.

use axum::{
    extract::State,
    response::Json,
    routing::post,
    Router,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::http::{AppState, AppError, AppResult};

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
    // Check if email service exists
    let email_service = state.email_service.as_ref()
        .ok_or_else(|| AppError::bad_request("Email service not configured"))?;

    // Check if user exists with this email
    let user = state
        .user_service
        .get_by_email(&req.email)
        .await
        .map_err(|e| AppError::internal_server_error(format!("Database error: {e}")))?;

    // SECURITY: Always return the same message regardless of whether user exists
    // to prevent user enumeration attacks.
    let generic_message = "If an account exists with this email, a verification code will be sent.".to_string();

    let user = match user {
        Some(u) => u,
        None => {
            return Ok(Json(EmailVerificationResponse {
                message: generic_message,
            }));
        }
    };

    // Generate and send verification email
    let token_service = synctv_core::service::EmailTokenService::new(
        state.user_service.pool().clone()
    );

    let _token = email_service
        .send_verification_email(&req.email, &token_service, &user.id)
        .await
        .map_err(|e| AppError::internal_server_error(format!("Failed to send email: {e}")))?;

    // SECURITY: Token is never logged or returned to prevent security leaks
    // Token is only sent via email to the user

    Ok(Json(EmailVerificationResponse {
        message: generic_message,
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
    use synctv_core::service::{EmailTokenService, EmailTokenType};

    // Check if email service exists
    let _email_service = state.email_service.as_ref()
        .ok_or_else(|| AppError::bad_request("Email service not configured"))?;

    // Validate token first (constant-time regardless of user existence)
    let token_service = EmailTokenService::new(
        state.user_service.pool().clone()
    );

    let validated_user_id = token_service
        .validate_token(&req.token, EmailTokenType::EmailVerification)
        .await
        .map_err(|_| AppError::bad_request("Invalid or expired verification token"))?;

    // Look up user by email
    let user = state
        .user_service
        .get_by_email(&req.email)
        .await
        .map_err(|e| AppError::internal_server_error(format!("Database error: {e}")))?;

    // Use generic error to prevent user enumeration
    let user = user.ok_or_else(|| AppError::bad_request("Invalid or expired verification token"))?;

    // Verify token matches user
    if validated_user_id != user.id {
        return Err(AppError::bad_request("Invalid or expired verification token"));
    }

    // Mark email as verified
    state
        .user_service
        .set_email_verified(&user.id, true)
        .await
        .map_err(|e| AppError::internal_server_error(format!("Failed to update email verification: {e}")))?;

    info!("Email verified for user {}", user.id.as_str());

    Ok(Json(serde_json::json!({
        "message": "Email verified successfully",
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
    // Check if email service exists
    let email_service = state.email_service.as_ref()
        .ok_or_else(|| AppError::bad_request("Email service not configured"))?;

    // Check if user exists (don't reveal if not found for security)
    let user = state
        .user_service
        .get_by_email(&req.email)
        .await
        .map_err(|e| AppError::internal_server_error(format!("Database error: {e}")))?;

    let Some(user) = user else {
        // Don't reveal whether email exists
        return Ok(Json(PasswordResetResponse {
            message: "If an account exists with this email, a password reset code will be sent.".to_string(),
        }));
    };

    // Generate and send reset email
    let token_service = synctv_core::service::EmailTokenService::new(
        state.user_service.pool().clone()
    );

    let _token = email_service
        .send_password_reset_email(&req.email, &token_service, &user.id)
        .await
        .map_err(|e| AppError::internal_server_error(format!("Failed to send email: {e}")))?;

    // SECURITY: Token is never logged or returned to prevent security leaks
    // Token is only sent via email to the user

    info!("Password reset requested for user {}", user.id.as_str());

    Ok(Json(PasswordResetResponse {
        message: "Password reset code sent to your email".to_string(),
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
    use synctv_core::service::{EmailTokenService, EmailTokenType};

    // Validate new password first (fast-fail before any DB lookups)
    // Use constants from validation module for consistency
    use super::validation::limits::{PASSWORD_MIN, PASSWORD_MAX};
    if req.new_password.len() < PASSWORD_MIN {
        return Err(AppError::bad_request(format!("Password must be at least {PASSWORD_MIN} characters")));
    }
    if req.new_password.len() > PASSWORD_MAX {
        return Err(AppError::bad_request(format!("Password must be at most {PASSWORD_MAX} characters")));
    }

    // Validate token first (constant-time regardless of user existence)
    let token_service = EmailTokenService::new(
        state.user_service.pool().clone()
    );

    let validated_user_id = token_service
        .validate_token(&req.token, EmailTokenType::PasswordReset)
        .await
        .map_err(|_| AppError::bad_request("Invalid or expired reset token"))?;

    // Look up user by email
    let user = state
        .user_service
        .get_by_email(&req.email)
        .await
        .map_err(|e| AppError::internal_server_error(format!("Database error: {e}")))?;

    // Use generic error to prevent user enumeration
    let user = user.ok_or_else(|| AppError::bad_request("Invalid or expired reset token"))?;

    // Verify token matches user
    if validated_user_id != user.id {
        return Err(AppError::bad_request("Invalid or expired reset token"));
    }

    // Check if user is banned (don't allow password reset for suspended accounts)
    if user.status == synctv_core::models::UserStatus::Banned {
        return Err(AppError::bad_request("Invalid or expired reset token"));
    }

    // Update password using UserService
    state
        .user_service
        .set_password(&user.id, &req.new_password)
        .await
        .map_err(|e| AppError::internal_server_error(format!("Failed to update password: {e}")))?;

    info!("Password reset completed for user {}", user.id.as_str());

    Ok(Json(serde_json::json!({
        "message": "Password reset successfully",
    })))
}

/// Email verification confirmation request
#[derive(Debug, Deserialize)]
pub struct EmailVerificationConfirm {
    pub email: String,
    pub token: String,
}
