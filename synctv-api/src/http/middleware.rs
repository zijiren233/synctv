// HTTP middleware

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::request::Parts,
};
use std::sync::Arc;
use synctv_core::{
    models::id::UserId,
    service::auth::JwtValidator,
};

use super::{AppError, AppState};

/// Authenticated user extracted from JWT token
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: UserId,
}

/// Extension to hold JWT validator in request extensions
#[derive(Clone)]
struct JwtValidatorExt(Arc<JwtValidator>);

#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Get AppState from state
        let app_state = AppState::from_ref(state);

        // Create or extract JWT validator
        let validator = parts
            .extensions
            .get::<JwtValidatorExt>().map_or_else(|| {
                Arc::new(JwtValidator::new(Arc::new(app_state.jwt_service.clone())))
            }, |v| v.0.clone());

        // Extract Authorization header
        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .ok_or_else(|| AppError::unauthorized("Missing Authorization header"))?;

        // Parse Bearer token and validate using unified validator
        let auth_str = auth_header
            .to_str()
            .map_err(|e| AppError::unauthorized(format!("Invalid Authorization header: {e}")))?;

        let user_id = validator
            .validate_http_extract_user_id(auth_str)
            .map_err(|e| AppError::unauthorized(format!("{e}")))?;

        Ok(Self { user_id })
    }
}
