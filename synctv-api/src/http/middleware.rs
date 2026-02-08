// HTTP middleware

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Request, State},
    http::{request::Parts, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use synctv_core::{
    models::id::UserId,
    service::auth::JwtValidator,
    service::rate_limit::{RateLimiter, RateLimitError},
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

/// Rate limiting configuration for different endpoint categories
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Authentication endpoints (login, register) - stricter limits
    pub auth_max_requests: u32,
    pub auth_window_seconds: u64,

    /// Write operations (create, update, delete) - moderate limits
    pub write_max_requests: u32,
    pub write_window_seconds: u64,

    /// Read operations (get, list) - relaxed limits
    pub read_max_requests: u32,
    pub read_window_seconds: u64,

    /// Media operations (add, remove media) - moderate limits
    pub media_max_requests: u32,
    pub media_window_seconds: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            // Auth: 5 requests per minute
            auth_max_requests: 5,
            auth_window_seconds: 60,

            // Write: 30 requests per minute
            write_max_requests: 30,
            write_window_seconds: 60,

            // Read: 100 requests per minute
            read_max_requests: 100,
            read_window_seconds: 60,

            // Media: 20 requests per minute
            media_max_requests: 20,
            media_window_seconds: 60,
        }
    }
}

/// Rate limit category for different types of operations
#[derive(Debug, Clone, Copy)]
pub enum RateLimitCategory {
    Auth,
    Write,
    Read,
    Media,
}

/// Middleware for rate limiting based on user ID and endpoint category
pub async fn rate_limit_middleware(
    State(state): State<AppState>,
    category: RateLimitCategory,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    // Extract user ID from authorization header if present
    let user_id = extract_user_id_from_header(&request, &state);

    // Use IP address as fallback if no user ID (for public endpoints)
    let rate_limit_key = user_id.unwrap_or_else(|| {
        // Extract IP from headers (X-Forwarded-For or X-Real-IP) or connection
        request
            .headers()
            .get("X-Forwarded-For")
            .and_then(|h| h.to_str().ok())
            .or_else(|| {
                request
                    .headers()
                    .get("X-Real-IP")
                    .and_then(|h| h.to_str().ok())
            })
            .unwrap_or("unknown")
            .to_string()
    });

    // Get rate limiter from app state (if configured)
    // For now, we'll create a disabled rate limiter if Redis is not configured
    let rate_limiter = RateLimiter::disabled("synctv:rate_limit:".to_string());
    let config = RateLimitConfig::default();

    // Determine rate limit parameters based on category
    let (max_requests, window_seconds, category_name) = match category {
        RateLimitCategory::Auth => (config.auth_max_requests, config.auth_window_seconds, "auth"),
        RateLimitCategory::Write => (config.write_max_requests, config.write_window_seconds, "write"),
        RateLimitCategory::Read => (config.read_max_requests, config.read_window_seconds, "read"),
        RateLimitCategory::Media => (config.media_max_requests, config.media_window_seconds, "media"),
    };

    // Check rate limit
    let key = format!("{}:{}:{}", category_name, rate_limit_key, request.uri().path());
    match rate_limiter.check_rate_limit(&key, max_requests, window_seconds).await {
        Ok(()) => {
            // Rate limit check passed, proceed with request
            Ok(next.run(request).await)
        }
        Err(RateLimitError::RateLimitExceeded { retry_after_seconds }) => {
            // Rate limit exceeded, return 429 Too Many Requests
            let response = (
                StatusCode::TOO_MANY_REQUESTS,
                [
                    ("Retry-After", retry_after_seconds.to_string()),
                    ("X-RateLimit-Limit", max_requests.to_string()),
                    ("X-RateLimit-Reset", retry_after_seconds.to_string()),
                ],
                format!("Rate limit exceeded. Try again in {} seconds", retry_after_seconds),
            )
                .into_response();

            Ok(response)
        }
        Err(e) => {
            // Redis error or other issue, log and allow request through
            tracing::warn!("Rate limit check failed: {}. Allowing request.", e);
            Ok(next.run(request).await)
        }
    }
}

/// Helper function to extract user ID from authorization header
fn extract_user_id_from_header(request: &Request, state: &AppState) -> Option<String> {
    let auth_header = request.headers().get(axum::http::header::AUTHORIZATION)?;
    let auth_str = auth_header.to_str().ok()?;

    let validator = Arc::new(JwtValidator::new(Arc::new(state.jwt_service.clone())));
    let user_id = validator.validate_http_extract_user_id(auth_str).ok()?;

    Some(user_id.to_string())
}

/// Middleware factory for authentication endpoints
pub async fn auth_rate_limit(
    state: State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    rate_limit_middleware(state, RateLimitCategory::Auth, request, next).await
}

/// Middleware factory for write operations
pub async fn write_rate_limit(
    state: State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    rate_limit_middleware(state, RateLimitCategory::Write, request, next).await
}

/// Middleware factory for read operations
pub async fn read_rate_limit(
    state: State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    rate_limit_middleware(state, RateLimitCategory::Read, request, next).await
}

/// Middleware factory for media operations
pub async fn media_rate_limit(
    state: State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    rate_limit_middleware(state, RateLimitCategory::Media, request, next).await
}
