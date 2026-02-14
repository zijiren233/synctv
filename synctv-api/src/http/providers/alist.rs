//! Alist Provider HTTP Routes

use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;

use crate::http::{AppState, error::AppResult, middleware::AuthUser, provider_common::{InstanceQuery, error_response, parse_provider_error}};
use crate::impls::AlistApiImpl;
use crate::impls::providers::get_provider_binds;
use synctv_core::models::{MediaId, RoomId};
use synctv_core::provider::{MediaProvider, ProviderContext};

/// Build Alist HTTP routes
pub fn alist_routes() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/list", post(list))
        .route("/me", post(me))
        .route("/binds", get(binds))
        // Provider-specific proxy routes
        .route(
            "/proxy/:room_id/:media_id",
            get(proxy_stream).options(synctv_proxy::proxy_options_preflight),
        )
        .route("/proxy/:room_id/:media_id/m3u8", get(proxy_m3u8))
}

// ------------------------------------------------------------------
// Proxy handlers
// ------------------------------------------------------------------

/// Resolve playback URL from Alist provider for a media item.
async fn resolve_alist_playback(
    auth: &AuthUser,
    room_id: &RoomId,
    media_id: &MediaId,
    state: &AppState,
) -> Result<(String, HashMap<String, String>), crate::http::AppError> {
    // Verify user is a member of this room
    state.room_service.check_membership(room_id, &auth.user_id).await
        .map_err(|_| crate::http::AppError::forbidden("Not a member of this room"))?;

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
        .alist_provider
        .generate_playback(&ctx, &media.source_config)
        .await
        .map_err(|e| anyhow::anyhow!("Alist generate_playback failed: {e}"))?;

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

/// GET /`proxy/:room_id/:media_id` - Proxy Alist video stream
async fn proxy_stream(
    auth: AuthUser,
    Path((room_id, media_id)): Path<(String, String)>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<axum::response::Response> {
    let room_id = RoomId::from_string(room_id);
    let media_id = MediaId::from_string(media_id);

    let (url, provider_headers) =
        resolve_alist_playback(&auth, &room_id, &media_id, &state).await?;

    tracing::info!("Proxying Alist media: {}", url);

    let cfg = synctv_proxy::ProxyConfig {
        url: &url,
        provider_headers: &provider_headers,
        client_headers: &headers,
    };

    synctv_proxy::proxy_fetch_and_forward(cfg)
        .await
        .map_err(Into::into)
}

/// GET /`proxy/:room_id/:media_id/m3u8` - Proxy Alist M3U8
async fn proxy_m3u8(
    auth: AuthUser,
    Path((room_id, media_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> AppResult<axum::response::Response> {
    let room_id_parsed = RoomId::from_string(room_id.clone());
    let media_id_parsed = MediaId::from_string(media_id.clone());

    let (url, provider_headers) =
        resolve_alist_playback(&auth, &room_id_parsed, &media_id_parsed, &state).await?;

    let proxy_base = format!("/api/providers/alist/proxy/{room_id}/{media_id}");

    synctv_proxy::proxy_m3u8_and_rewrite(&url, &provider_headers, &proxy_base)
        .await
        .map_err(Into::into)
}

// ------------------------------------------------------------------
// Existing provider API handlers
// ------------------------------------------------------------------

/// Login to Alist
async fn login(
    _auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<crate::proto::providers::alist::LoginRequest>,
) -> impl IntoResponse {
    tracing::info!("Alist login request");

    let api = AlistApiImpl::new(state.alist_provider.clone());

    match api.login(req, query.as_deref()).await {
        Ok(resp) => {
            tracing::info!("Alist login successful");
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Alist login failed: {}", e);
            error_response(parse_provider_error(&e)).into_response()
        }
    }
}

/// List Alist directory
async fn list(
    _auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<crate::proto::providers::alist::ListRequest>,
) -> impl IntoResponse {
    tracing::info!("Alist list request");

    let api = AlistApiImpl::new(state.alist_provider.clone());

    match api.list(req, query.as_deref()).await {
        Ok(resp) => {
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Alist list failed: {}", e);
            error_response(parse_provider_error(&e)).into_response()
        }
    }
}

/// Get Alist user info
async fn me(
    _auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<crate::proto::providers::alist::GetMeRequest>,
) -> impl IntoResponse {
    tracing::info!("Alist me request");

    let api = AlistApiImpl::new(state.alist_provider.clone());

    match api.get_me(req, query.as_deref()).await {
        Ok(resp) => {
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Alist me failed: {}", e);
            error_response(parse_provider_error(&e)).into_response()
        }
    }
}

/// Logout from Alist
async fn logout() -> impl IntoResponse {
    tracing::info!("Alist logout request");
    (
        StatusCode::OK,
        Json(json!({"message": "Logout successful"})),
    )
        .into_response()
}

/// Get Alist binds (saved credentials)
async fn binds(
    auth: crate::http::middleware::AuthUser,
    State(state): State<AppState>,
) -> impl IntoResponse {
    tracing::info!("Alist binds request for user: {}", auth.user_id);

    match get_provider_binds(
        &state.user_provider_credential_repository,
        &auth.user_id.to_string(),
        "alist",
        "username",
    )
    .await
    {
        Ok(provider_binds) => {
            let alist_binds: Vec<_> = provider_binds
                .into_iter()
                .map(|b| {
                    json!({
                        "id": b.id,
                        "host": b.host,
                        "username": b.label_value,
                        "created_at": b.created_at_str,
                    })
                })
                .collect();

            (
                StatusCode::OK,
                Json(json!({"binds": alist_binds})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to query credentials: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to query credentials"})),
            )
                .into_response()
        }
    }
}
