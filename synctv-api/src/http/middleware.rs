// HTTP middleware

use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, HeaderValue},
};
use synctv_core::models::id::UserId;

use super::AppError;

/// Authenticated user extracted from JWT token
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: UserId,
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Extract Authorization header
        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .ok_or_else(|| AppError::unauthorized("Missing Authorization header"))?;

        // Parse Bearer token
        let token = extract_bearer_token(auth_header).map_err(|e| AppError::unauthorized(e))?;

        // Extract user_id from token
        let user_id_str = extract_user_id_from_token(token)
            .map_err(|e| AppError::unauthorized(format!("Invalid token: {}", e)))?;

        Ok(AuthUser {
            user_id: UserId::from_string(user_id_str),
        })
    }
}

/// Extract bearer token from Authorization header
fn extract_bearer_token(header: &HeaderValue) -> Result<&str, String> {
    let auth_str = header
        .to_str()
        .map_err(|_| "Invalid Authorization header value".to_string())?;

    if !auth_str.starts_with("Bearer ") {
        return Err("Authorization header must start with 'Bearer '".to_string());
    }

    Ok(&auth_str[7..]) // Skip "Bearer "
}

/// Extract user_id from JWT token
///
/// This is a simplified version. In production, use the JWT service to verify the token.
fn extract_user_id_from_token(token: &str) -> Result<String, String> {
    use base64::prelude::*;

    // Split token into parts
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid token format".to_string());
    }

    // Decode payload (second part)
    let payload = parts[1];
    let decoded = BASE64_URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|e| format!("Failed to decode token: {}", e))?;

    // Parse JSON
    let json: serde_json::Value = serde_json::from_slice(&decoded)
        .map_err(|e| format!("Failed to parse token JSON: {}", e))?;

    // Extract sub (subject) claim
    let user_id = json
        .get("sub")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing sub claim".to_string())?;

    Ok(user_id.to_string())
}
