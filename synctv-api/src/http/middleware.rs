// HTTP middleware

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Request, State},
    http::{request::Parts, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use once_cell::sync::Lazy;
use std::sync::Arc;
use synctv_core::{
    models::{id::UserId, UserStatus},
    service::auth::JwtValidator,
    service::rate_limit::RateLimitError,
};

use super::{AppError, AppState};

/// Pre-validated security header names (validated once at startup via Lazy)
static X_FRAME_OPTIONS: Lazy<axum::http::HeaderName> = Lazy::new(|| {
    axum::http::HeaderName::from_static("x-frame-options")
});
static X_CONTENT_TYPE_OPTIONS: Lazy<axum::http::HeaderName> = Lazy::new(|| {
    axum::http::HeaderName::from_static("x-content-type-options")
});
static X_XSS_PROTECTION: Lazy<axum::http::HeaderName> = Lazy::new(|| {
    axum::http::HeaderName::from_static("x-xss-protection")
});
static CONTENT_SECURITY_POLICY: Lazy<axum::http::HeaderName> = Lazy::new(|| {
    axum::http::HeaderName::from_static("content-security-policy")
});
static REFERRER_POLICY: Lazy<axum::http::HeaderName> = Lazy::new(|| {
    axum::http::HeaderName::from_static("referrer-policy")
});
static PERMISSIONS_POLICY: Lazy<axum::http::HeaderName> = Lazy::new(|| {
    axum::http::HeaderName::from_static("permissions-policy")
});
static PRAGMA: Lazy<axum::http::HeaderName> = Lazy::new(|| {
    axum::http::HeaderName::from_static("pragma")
});

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

        // Parse Bearer token and validate using unified validator.
        // We extract full claims (not just user_id) so we can check the
        // issued-at timestamp against password-change invalidation.
        let auth_str = auth_header
            .to_str()
            .map_err(|e| AppError::unauthorized(format!("Invalid Authorization header: {e}")))?;

        let claims = validator
            .validate_http(auth_str)
            .map_err(|e| AppError::unauthorized(format!("{e}")))?;

        let user_id = UserId::from_string(claims.sub);

        // Check if user is banned or deleted (defense-in-depth: catches banned
        // users even if they hold a valid JWT issued before the ban)
        let user = app_state.user_service.get_user(&user_id).await
            .map_err(|_| AppError::unauthorized("User not found"))?;
        if user.is_deleted() || user.status == UserStatus::Banned {
            return Err(AppError::unauthorized("Authentication failed"));
        }

        // Reject tokens issued before the user's last password change.
        // This ensures that stolen tokens become useless after a password reset.
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

    /// Admin operations - moderate limits to prevent brute force
    pub admin_max_requests: u32,
    pub admin_window_seconds: u64,

    /// Streaming operations (FLV/HLS) - per-user concurrency limits
    pub streaming_max_requests: u32,
    pub streaming_window_seconds: u64,

    /// WebSocket connection attempts
    pub websocket_max_requests: u32,
    pub websocket_window_seconds: u64,
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

            // Admin: 30 requests per minute
            admin_max_requests: 30,
            admin_window_seconds: 60,

            // Streaming: 50 requests per minute (playlist + segment fetches)
            streaming_max_requests: 50,
            streaming_window_seconds: 60,

            // WebSocket: 10 connection attempts per minute
            websocket_max_requests: 10,
            websocket_window_seconds: 60,
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
    Admin,
    Streaming,
    WebSocket,
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

    // Use IP address as fallback if no user ID (for public endpoints).
    // We only trust X-Forwarded-For/X-Real-IP headers when:
    // 1. The request comes from a configured trusted proxy, OR
    // 2. Development mode is enabled (for local testing)
    //
    // This prevents header spoofing attacks that could bypass rate limiting.
    let rate_limit_key = user_id.unwrap_or_else(|| {
        // Try to get the remote/socket address from ConnectInfo extension
        let remote_addr = request
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0.ip());

        // Check if we should trust proxy headers
        let should_trust_headers = state.config.server.development_mode
            || remote_addr.map_or(false, |ip| state.config.server.is_trusted_proxy(&ip));

        if should_trust_headers {
            // Trust X-Forwarded-For from trusted proxies (or in dev mode)
            let forwarded = request
                .headers()
                .get("X-Forwarded-For")
                .and_then(|h| h.to_str().ok())
                .and_then(|v| v.split(',').next())
                .map(str::trim);

            if let Some(ip) = forwarded {
                ip.to_string()
            } else if let Some(ip) = request
                .headers()
                .get("X-Real-IP")
                .and_then(|h| h.to_str().ok())
            {
                ip.to_string()
            } else if let Some(ip) = remote_addr {
                ip.to_string()
            } else {
                "unknown".to_string()
            }
        } else {
            // Don't trust headers - use socket address directly
            remote_addr
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        }
    });

    // Get rate limiter from app state
    let rate_limiter = state.rate_limiter.clone();
    let config = RateLimitConfig::default();

    // Determine rate limit parameters based on category
    let (max_requests, window_seconds, category_name) = match category {
        RateLimitCategory::Auth => (config.auth_max_requests, config.auth_window_seconds, "auth"),
        RateLimitCategory::Write => (config.write_max_requests, config.write_window_seconds, "write"),
        RateLimitCategory::Read => (config.read_max_requests, config.read_window_seconds, "read"),
        RateLimitCategory::Media => (config.media_max_requests, config.media_window_seconds, "media"),
        RateLimitCategory::Admin => (config.admin_max_requests, config.admin_window_seconds, "admin"),
        RateLimitCategory::Streaming => (config.streaming_max_requests, config.streaming_window_seconds, "streaming"),
        RateLimitCategory::WebSocket => (config.websocket_max_requests, config.websocket_window_seconds, "websocket"),
    };

    // Check rate limit
    // FIXED: P0.13 - Removed path from key to enforce category-wide limit
    // Previously: format!("{}:{}:{}", category_name, rate_limit_key, path)
    // This caused each endpoint to have its own counter, effectively multiplying the limit
    // Now: All endpoints in same category share the limit (e.g., 30 req/min for ALL write operations)
    let key = format!("ratelimit:{}:{}", category_name, rate_limit_key);
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
                format!("Rate limit exceeded. Try again in {retry_after_seconds} seconds"),
            )
                .into_response();

            Ok(response)
        }
        Err(e) => {
            // Redis error or other issue — fail closed to prevent abuse.
            // When rate limiting is unavailable, reject requests rather than
            // allowing unbounded traffic.
            tracing::error!("Rate limit check failed: {}. Denying request (fail closed).", e);
            let response = (
                StatusCode::SERVICE_UNAVAILABLE,
                "Rate limiting service unavailable. Please try again later.",
            )
                .into_response();
            Ok(response)
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

/// Middleware factory for admin operations
pub async fn admin_rate_limit(
    state: State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    rate_limit_middleware(state, RateLimitCategory::Admin, request, next).await
}

/// Middleware factory for streaming operations (FLV/HLS)
pub async fn streaming_rate_limit(
    state: State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    rate_limit_middleware(state, RateLimitCategory::Streaming, request, next).await
}

/// Middleware factory for WebSocket connection attempts
pub async fn websocket_rate_limit(
    state: State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    rate_limit_middleware(state, RateLimitCategory::WebSocket, request, next).await
}

/// Security headers middleware
///
/// Adds security-related HTTP headers to all responses to protect against
/// common web vulnerabilities:
/// - X-Frame-Options: Prevents clickjacking
/// - X-Content-Type-Options: Prevents MIME type sniffing
/// - X-XSS-Protection: Enables browser XSS filter (legacy, but still useful)
/// - Content-Security-Policy: Restricts resource loading
/// - Strict-Transport-Security: Enforces HTTPS (only if configured)
/// - Referrer-Policy: Controls referrer information
/// - Permissions-Policy: Restricts browser features
pub async fn security_headers_middleware(
    request: Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;

    let headers = response.headers_mut();

    // Prevent clickjacking attacks
    // DENY: The page cannot be displayed in a frame, regardless of the site attempting to do so.
    if !headers.contains_key("X-Frame-Options") {
        headers.insert(
            // Static header name - validated at compile time via Lazy
            X_FRAME_OPTIONS.clone(),
            axum::http::HeaderValue::from_static("DENY"),
        );
    }

    // Prevent MIME type sniffing
    // nosniff: Blocks a request if the requested type is "style" or "script"
    // and the MIME type is not a valid MIME type for the requested type.
    if !headers.contains_key("X-Content-Type-Options") {
        headers.insert(
            X_CONTENT_TYPE_OPTIONS.clone(),
            axum::http::HeaderValue::from_static("nosniff"),
        );
    }

    // Enable XSS filtering in browsers (legacy but still useful for older browsers)
    // 1; mode=block: Enables XSS filtering. Rather than sanitizing the page,
    // the browser will prevent rendering of the page entirely if an attack is detected.
    if !headers.contains_key("X-XSS-Protection") {
        headers.insert(
            X_XSS_PROTECTION.clone(),
            axum::http::HeaderValue::from_static("1; mode=block"),
        );
    }

    // Content Security Policy
    // Restricts sources of executable scripts, styles, and other resources
    // This is a restrictive default - applications may need to customize this
    if !headers.contains_key("Content-Security-Policy") {
        // Note: 'unsafe-inline' for style is often needed for inline styles
        // For APIs, we can be very restrictive
        headers.insert(
            CONTENT_SECURITY_POLICY.clone(),
            axum::http::HeaderValue::from_static(
                "default-src 'none'; frame-ancestors 'none'; base-uri 'none'"
            ),
        );
    }

    // Referrer Policy
    // strict-origin-when-cross-origin: Send origin only for cross-origin requests,
    // full URL for same-origin requests
    if !headers.contains_key("Referrer-Policy") {
        headers.insert(
            REFERRER_POLICY.clone(),
            axum::http::HeaderValue::from_static("strict-origin-when-cross-origin"),
        );
    }

    // Permissions Policy (formerly Feature Policy)
    // Disables various browser features that are typically not needed by APIs
    if !headers.contains_key("Permissions-Policy") {
        headers.insert(
            PERMISSIONS_POLICY.clone(),
            axum::http::HeaderValue::from_static(
                "accelerometer=(), camera=(), geolocation=(), gyroscope=(), \
                 magnetometer=(), microphone=(), payment=(), usb=()"
            ),
        );
    }

    // Cache Control for API responses
    // Prevents caching of sensitive API responses
    if !headers.contains_key("Cache-Control") {
        headers.insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static(
                "no-store, no-cache, must-revalidate, proxy-revalidate"
            ),
        );
    }

    // Pragma: no-cache (for HTTP/1.0 compatibility)
    if !headers.contains_key("Pragma") {
        headers.insert(
            PRAGMA.clone(),
            axum::http::HeaderValue::from_static("no-cache"),
        );
    }

    response
}

/// HSTS (HTTP Strict Transport Security) middleware
///
/// Should be used alongside security_headers_middleware when HTTPS is enabled.
/// This tells browsers to always use HTTPS for this site.
///
/// # Arguments
/// * `max_age` - The time, in seconds, that the browser should remember
///   that a site is only to be accessed using HTTPS.
/// * `include_subdomains` - If true, this rule applies to all subdomains as well.
/// * `preload` - If true, the site can be included in browser HSTS preload lists.
pub fn hsts_header(max_age: u64, include_subdomains: bool, preload: bool) -> String {
    let mut value = format!("max-age={max_age}");

    if include_subdomains {
        value.push_str("; includeSubDomains");
    }

    if preload {
        value.push_str("; preload");
    }

    value
}
