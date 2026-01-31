//! Provider HTTP Routes
//!
//! HTTP/REST routes for provider-specific functionality:
//! - Parse: Convert URLs to source_config
//! - Login: Authentication endpoints
//! - Proxy: Media proxy endpoints

use axum::{
    Router,
    routing::{get, post},
    extract::{Path, Query},
    response::IntoResponse,
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
// TODO: Import provider clients when implementing handlers
// use synctv_providers::{BilibiliClient, AlistClient, EmbyClient};

/// Bilibili parse request
#[derive(Debug, Deserialize)]
pub struct BilibiliParseRequest {
    pub url: String,
    #[serde(default)]
    pub cookies: std::collections::HashMap<String, String>,
}

/// Bilibili parse response
#[derive(Debug, Serialize)]
pub struct BilibiliParseResponse {
    pub title: String,
    pub videos: Vec<VideoInfo>,
}

#[derive(Debug, Serialize)]
pub struct VideoInfo {
    pub bvid: Option<String>,
    pub cid: u64,
    pub name: String,
}

/// Bilibili login QR code request
#[derive(Debug, Deserialize)]
pub struct LoginQRRequest {}

/// Bilibili login QR code response
#[derive(Debug, Serialize)]
pub struct LoginQRResponse {
    pub url: String,
    pub key: String,
}

/// Bilibili login check request
#[derive(Debug, Deserialize)]
pub struct LoginCheckRequest {
    pub key: String,
}

/// Bilibili login check response
#[derive(Debug, Serialize)]
pub struct LoginCheckResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cookies: Option<std::collections::HashMap<String, String>>,
}

/// Proxy query parameters
#[derive(Debug, Deserialize)]
pub struct ProxyQuery {
    pub url: String,
    #[serde(default)]
    pub cookies: String, // JSON-encoded cookies
}

/// Build provider HTTP routes
///
/// Routes:
/// - POST /api/providers/bilibili/parse - Parse Bilibili URL
/// - POST /api/providers/bilibili/login/qr - Get QR code for login
/// - GET /api/providers/bilibili/login/check/:key - Check QR code status
/// - GET /api/providers/bilibili/proxy - Proxy media content
/// - Similar routes for alist and emby
pub fn build_provider_routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        // Bilibili routes
        .route("/bilibili/parse", post(bilibili_parse))
        .route("/bilibili/login/qr", post(bilibili_login_qr))
        .route("/bilibili/login/check/:key", get(bilibili_login_check))
        .route("/bilibili/proxy", get(bilibili_proxy))
        // Alist routes
        .route("/alist/parse", post(alist_parse))
        .route("/alist/list", get(alist_list))
        .route("/alist/proxy", get(alist_proxy))
        // Emby routes
        .route("/emby/parse", post(emby_parse))
        .route("/emby/libraries", get(emby_libraries))
        .route("/emby/proxy", get(emby_proxy))
}

// ========== Bilibili Handlers ==========

async fn bilibili_parse(
    Json(req): Json<BilibiliParseRequest>,
) -> Result<Json<BilibiliParseResponse>, StatusCode> {
    // TODO: Implement using BilibiliClient
    // For now, return placeholder
    tracing::info!("Bilibili parse request: {:?}", req.url);

    Err(StatusCode::NOT_IMPLEMENTED)
}

async fn bilibili_login_qr(
    Json(_req): Json<LoginQRRequest>,
) -> Result<Json<LoginQRResponse>, StatusCode> {
    // TODO: Implement using BilibiliClient
    tracing::info!("Bilibili login QR request");

    Err(StatusCode::NOT_IMPLEMENTED)
}

async fn bilibili_login_check(
    Path(key): Path<String>,
) -> Result<Json<LoginCheckResponse>, StatusCode> {
    // TODO: Implement using BilibiliClient
    tracing::info!("Bilibili login check: {}", key);

    Err(StatusCode::NOT_IMPLEMENTED)
}

async fn bilibili_proxy(
    Query(params): Query<ProxyQuery>,
) -> impl IntoResponse {
    // TODO: Implement media proxying
    tracing::info!("Bilibili proxy request: {}", params.url);

    StatusCode::NOT_IMPLEMENTED
}

// ========== Alist Handlers ==========

async fn alist_parse(
    Json(req): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    // TODO: Implement using AlistClient
    tracing::info!("Alist parse request");

    Err(StatusCode::NOT_IMPLEMENTED)
}

async fn alist_list(
    Query(params): Query<Value>,
) -> Result<Json<Value>, StatusCode> {
    // TODO: Implement using AlistClient
    tracing::info!("Alist list request");

    Err(StatusCode::NOT_IMPLEMENTED)
}

async fn alist_proxy(
    Query(params): Query<ProxyQuery>,
) -> impl IntoResponse {
    // TODO: Implement media proxying
    tracing::info!("Alist proxy request: {}", params.url);

    StatusCode::NOT_IMPLEMENTED
}

// ========== Emby Handlers ==========

async fn emby_parse(
    Json(req): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    // TODO: Implement using EmbyClient
    tracing::info!("Emby parse request");

    Err(StatusCode::NOT_IMPLEMENTED)
}

async fn emby_libraries(
    Query(params): Query<Value>,
) -> Result<Json<Value>, StatusCode> {
    // TODO: Implement using EmbyClient
    tracing::info!("Emby libraries request");

    Err(StatusCode::NOT_IMPLEMENTED)
}

async fn emby_proxy(
    Query(params): Query<ProxyQuery>,
) -> impl IntoResponse {
    // TODO: Implement media proxying
    tracing::info!("Emby proxy request: {}", params.url);

    StatusCode::NOT_IMPLEMENTED
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_routes() {
        let router = build_provider_routes();
        // Basic test to ensure router builds successfully
        assert!(true);
    }
}
