//! Bilibili Provider HTTP Routes

use std::collections::HashMap;
use std::convert::Infallible;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use futures::stream::Stream;
use serde_json::json;

use crate::http::{AppState, error::AppResult, middleware::AuthUser, provider_common::{InstanceQuery, error_response, parse_provider_error}};
use crate::impls::BilibiliApiImpl;
use synctv_core::models::{MediaId, RoomId};
use synctv_core::provider::{MediaProvider, ProviderContext};

/// Build Bilibili HTTP routes
pub fn bilibili_routes() -> Router<AppState> {
    Router::new()
        .route("/parse", post(parse))
        .route("/login/qr", get(login_qr))
        .route("/login/qr", post(qr_check))
        .route("/login/captcha", get(new_captcha))
        .route("/login/sms/send", post(sms_send))
        .route("/login/sms/login", post(sms_login))
        .route("/me", get(user_info))
        .route("/logout", post(logout))
        // Provider-specific proxy routes
        .route(
            "/proxy/:room_id/:media_id",
            get(proxy_stream).options(synctv_proxy::proxy_options_preflight),
        )
        .route("/proxy/:room_id/:media_id/m3u8", get(proxy_m3u8))
        .route("/proxy/:room_id/:media_id/danmu", get(danmu_sse))
}

// ------------------------------------------------------------------
// Proxy handlers
// ------------------------------------------------------------------

/// Resolve playback URL from Bilibili provider for a media item.
async fn resolve_bilibili_playback(
    auth: &AuthUser,
    room_id: &RoomId,
    media_id: &MediaId,
    state: &AppState,
) -> Result<(String, HashMap<String, String>), crate::http::AppError> {
    let playlist = state
        .room_service
        .get_playlist(room_id)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get playlist: {e}"))?;

    let media = playlist
        .iter()
        .find(|m| m.id == *media_id)
        .ok_or_else(|| anyhow::anyhow!("Media not found in playlist"))?;

    let ctx = ProviderContext::new("synctv")
        .with_user_id(auth.user_id.as_str())
        .with_room_id(room_id.as_str());

    let playback_result = state
        .bilibili_provider
        .generate_playback(&ctx, &media.source_config)
        .await
        .map_err(|e| anyhow::anyhow!("Bilibili generate_playback failed: {e}"))?;

    let default_mode = &playback_result.default_mode;
    let playback_info = playback_result
        .playback_infos
        .get(default_mode)
        .ok_or_else(|| anyhow::anyhow!("Default playback mode not found"))?;

    let url = playback_info
        .urls
        .first()
        .ok_or_else(|| anyhow::anyhow!("No URLs in playback info"))?;

    Ok((url.clone(), playback_info.headers.clone()))
}

/// GET /proxy/:room_id/:media_id - Proxy Bilibili video stream
async fn proxy_stream(
    auth: AuthUser,
    Path((room_id, media_id)): Path<(String, String)>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<axum::response::Response> {
    let room_id = RoomId::from_string(room_id);
    let media_id = MediaId::from_string(media_id);

    let (url, provider_headers) =
        resolve_bilibili_playback(&auth, &room_id, &media_id, &state).await?;

    tracing::info!("Proxying Bilibili media: {}", url);

    let cfg = synctv_proxy::ProxyConfig {
        url: &url,
        provider_headers: &provider_headers,
        client_headers: &headers,
    };

    synctv_proxy::proxy_fetch_and_forward(cfg)
        .await
        .map_err(Into::into)
}

/// GET /proxy/:room_id/:media_id/m3u8 - Proxy Bilibili M3U8
async fn proxy_m3u8(
    auth: AuthUser,
    Path((room_id, media_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> AppResult<axum::response::Response> {
    let room_id_parsed = RoomId::from_string(room_id.clone());
    let media_id_parsed = MediaId::from_string(media_id.clone());

    let (url, provider_headers) =
        resolve_bilibili_playback(&auth, &room_id_parsed, &media_id_parsed, &state).await?;

    let proxy_base = format!("/api/providers/bilibili/proxy/{room_id}/{media_id}");

    synctv_proxy::proxy_m3u8_and_rewrite(&url, &provider_headers, &proxy_base)
        .await
        .map_err(Into::into)
}

/// GET /proxy/:room_id/:media_id/danmu - Bilibili danmaku SSE
async fn danmu_sse(
    _auth: AuthUser,
    Path((_room_id, _media_id)): Path<(String, String)>,
    State(_state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Stub: keep-alive stream. Actual Bilibili danmu fetching to be added.
    let stream = futures::stream::pending::<Result<Event, Infallible>>();
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ------------------------------------------------------------------
// Existing provider API handlers
// ------------------------------------------------------------------

/// Parse Bilibili URL
async fn parse(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    tracing::info!("Bilibili parse request");

    let proto_req = match serde_json::from_value::<crate::proto::providers::bilibili::ParseRequest>(req) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid request: {}", e)})),
            ).into_response();
        }
    };

    let api = BilibiliApiImpl::new(state.bilibili_provider.clone());

    match api.parse(proto_req, query.as_deref()).await {
        Ok(resp) => {
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Bilibili parse failed: {}", e);
            error_response(parse_provider_error(&e)).into_response()
        }
    }
}

/// Generate Bilibili QR code for login
async fn login_qr(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    tracing::info!("Bilibili login QR request");

    let proto_req = match serde_json::from_value::<crate::proto::providers::bilibili::LoginQrRequest>(req) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid request: {}", e)})),
            ).into_response();
        }
    };

    let api = BilibiliApiImpl::new(state.bilibili_provider.clone());

    match api.login_qr(proto_req, query.as_deref()).await {
        Ok(resp) => {
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to generate QR code: {}", e);
            error_response(parse_provider_error(&e)).into_response()
        }
    }
}

/// Check Bilibili QR code login status
async fn qr_check(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    tracing::info!("Bilibili QR check");

    let proto_req = match serde_json::from_value::<crate::proto::providers::bilibili::CheckQrRequest>(req) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid request: {}", e)})),
            ).into_response();
        }
    };

    let api = BilibiliApiImpl::new(state.bilibili_provider.clone());

    match api.check_qr(proto_req, query.as_deref()).await {
        Ok(resp) => {
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to check QR status: {}", e);
            error_response(parse_provider_error(&e)).into_response()
        }
    }
}

/// Get captcha for SMS login
async fn new_captcha(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    tracing::info!("Bilibili new captcha request");

    let proto_req = match serde_json::from_value::<crate::proto::providers::bilibili::GetCaptchaRequest>(req) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid request: {}", e)})),
            ).into_response();
        }
    };

    let api = BilibiliApiImpl::new(state.bilibili_provider.clone());

    match api.get_captcha(proto_req, query.as_deref()).await {
        Ok(resp) => {
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to get captcha: {}", e);
            error_response(parse_provider_error(&e)).into_response()
        }
    }
}

/// Send SMS verification code
async fn sms_send(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    tracing::info!("Bilibili SMS send request");

    let proto_req = match serde_json::from_value::<crate::proto::providers::bilibili::SendSmsRequest>(req) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid request: {}", e)})),
            ).into_response();
        }
    };

    let api = BilibiliApiImpl::new(state.bilibili_provider.clone());

    match api.send_sms(proto_req, query.as_deref()).await {
        Ok(resp) => {
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to send SMS: {}", e);
            error_response(parse_provider_error(&e)).into_response()
        }
    }
}

/// Login with SMS code
async fn sms_login(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    tracing::info!("Bilibili SMS login request");

    let proto_req = match serde_json::from_value::<crate::proto::providers::bilibili::LoginSmsRequest>(req) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid request: {}", e)})),
            ).into_response();
        }
    };

    let api = BilibiliApiImpl::new(state.bilibili_provider.clone());

    match api.login_sms(proto_req, query.as_deref()).await {
        Ok(resp) => {
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to login with SMS: {}", e);
            error_response(parse_provider_error(&e)).into_response()
        }
    }
}

/// Get Bilibili user info
async fn user_info(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    tracing::info!("Bilibili user info request");

    let proto_req = match serde_json::from_value::<crate::proto::providers::bilibili::UserInfoRequest>(req) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid request: {}", e)})),
            ).into_response();
        }
    };

    let api = BilibiliApiImpl::new(state.bilibili_provider.clone());

    match api.get_user_info(proto_req, query.as_deref()).await {
        Ok(resp) => {
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to get user info: {}", e);
            error_response(parse_provider_error(&e)).into_response()
        }
    }
}

/// Logout (just return success, cookies are client-side)
async fn logout() -> impl IntoResponse {
    tracing::info!("Bilibili logout request");
    (
        StatusCode::OK,
        Json(json!({"message": "Logout successful"})),
    )
        .into_response()
}
