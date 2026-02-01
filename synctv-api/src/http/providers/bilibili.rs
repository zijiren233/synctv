//! Bilibili Provider HTTP Routes

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use synctv_core::provider::provider_client::{
    create_remote_bilibili_client, load_local_bilibili_client,
};

use crate::http::AppState;

/// Bilibili parse request
#[derive(Debug, Deserialize)]
pub struct BilibiliParseRequest {
    pub url: String,
    #[serde(default)]
    pub cookies: HashMap<String, String>,
}

/// Bilibili QR code login request
#[derive(Debug, Deserialize)]
pub struct BilibiliQRLoginRequest {
    pub key: String,
}

/// Bilibili QR code response
#[derive(Debug, Serialize)]
pub struct QrCodeResponse {
    pub url: String,
    pub key: String,
}

/// Bilibili QR check response
#[derive(Debug, Serialize)]
pub struct QrStatusResponse {
    /// Status: 0=success, 1=key expired, 2=not scanned, 3=scanned but not confirmed
    pub status: i32,
    /// Cookies if status is 0 (success)
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub cookies: HashMap<String, String>,
}

/// Bilibili SMS send request
#[derive(Debug, Deserialize)]
pub struct BilibiliSMSSendRequest {
    pub phone: String,
    pub token: String,
    pub challenge: String,
    pub validate: String,
}

/// Bilibili SMS login request
#[derive(Debug, Deserialize)]
pub struct BilibiliSMSLoginRequest {
    pub phone: String,
    pub code: String,
    pub captcha_key: String,
}

/// Bilibili user info request (cookies in body or query)
#[derive(Debug, Deserialize)]
pub struct BilibiliUserInfoRequest {
    #[serde(default)]
    pub cookies: HashMap<String, String>,
}

/// Instance query parameter
#[derive(Debug, Deserialize)]
pub struct InstanceQuery {
    /// Optional provider instance name for remote provider
    #[serde(default)]
    pub instance_name: Option<String>,
}

/// Build Bilibili HTTP routes
fn bilibili_routes() -> Router<AppState> {
    Router::new()
        .route("/parse", post(parse))
        .route("/login/qr", get(login_qr))
        .route("/login/qr", post(qr_check))
        .route("/login/captcha", get(new_captcha))
        .route("/login/sms/send", post(sms_send))
        .route("/login/sms/login", post(sms_login))
        .route("/me", get(user_info))
        .route("/logout", post(logout))
}

/// Self-register Bilibili routes on module load
///
/// This runs automatically when the module is loaded, no manual registration needed!
pub fn init() {
    super::register_route_builder(|| ("bilibili".to_string(), bilibili_routes()));
}

// Handlers

/// Parse Bilibili URL
async fn parse(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<BilibiliParseRequest>,
) -> impl IntoResponse {
    tracing::info!("Bilibili parse request: url={}", req.url);

    // Determine which client to use (remote or local)
    let client = if let Some(instance_name) = query.instance_name {
        if let Some(channel) = state.provider_instance_manager.get(&instance_name).await {
            tracing::debug!("Using remote Bilibili instance: {}", instance_name);
            create_remote_bilibili_client(channel)
        } else {
            tracing::warn!(
                "Remote instance '{}' not found, falling back to local",
                instance_name
            );
            load_local_bilibili_client()
        }
    } else {
        tracing::debug!("Using local Bilibili client");
        load_local_bilibili_client()
    };

    // Step 1: Match URL to determine type and ID
    let match_req = synctv_providers::grpc::bilibili::MatchReq {
        url: req.url.clone(),
    };

    let match_resp = match client.r#match(match_req).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Failed to match URL: {}", e);
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Failed to parse URL",
                    "message": e.to_string()
                })),
            )
                .into_response();
        }
    };

    tracing::debug!(
        "URL matched: type={}, id={}",
        match_resp.r#type,
        match_resp.id
    );

    // Step 2: Parse based on type
    let page_info = match match_resp.r#type.as_str() {
        "video" | "bv" | "av" => {
            // Parse video page (bvid or aid)
            let parse_req = synctv_providers::grpc::bilibili::ParseVideoPageReq {
                cookies: req.cookies,
                bvid: if match_resp.r#type == "bv" {
                    match_resp.id.clone()
                } else {
                    String::new()
                },
                aid: if match_resp.r#type == "av" {
                    match_resp.id.parse().unwrap_or(0)
                } else {
                    0
                },
                sections: false,
            };

            match client.parse_video_page(parse_req).await {
                Ok(info) => info,
                Err(e) => {
                    tracing::error!("Failed to parse video page: {}", e);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "error": "Failed to parse video page",
                            "message": e.to_string()
                        })),
                    )
                        .into_response();
                }
            }
        }
        "pgc" | "ep" | "ss" => {
            // Parse PGC (anime/bangumi) page
            let parse_req = synctv_providers::grpc::bilibili::ParsePgcPageReq {
                cookies: req.cookies,
                epid: if match_resp.r#type == "ep" {
                    match_resp.id.parse().unwrap_or(0)
                } else {
                    0
                },
                ssid: if match_resp.r#type == "ss" {
                    match_resp.id.parse().unwrap_or(0)
                } else {
                    0
                },
            };

            match client.parse_pgc_page(parse_req).await {
                Ok(info) => info,
                Err(e) => {
                    tracing::error!("Failed to parse PGC page: {}", e);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "error": "Failed to parse PGC page",
                            "message": e.to_string()
                        })),
                    )
                        .into_response();
                }
            }
        }
        "live" => {
            // Parse live room page
            let parse_req = synctv_providers::grpc::bilibili::ParseLivePageReq {
                cookies: req.cookies,
                room_id: match_resp.id.parse().unwrap_or(0),
            };

            match client.parse_live_page(parse_req).await {
                Ok(info) => info,
                Err(e) => {
                    tracing::error!("Failed to parse live page: {}", e);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "error": "Failed to parse live page",
                            "message": e.to_string()
                        })),
                    )
                        .into_response();
                }
            }
        }
        _ => {
            tracing::error!("Unsupported URL type: {}", match_resp.r#type);
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Unsupported URL type",
                    "type": match_resp.r#type
                })),
            )
                .into_response();
        }
    };

    // Return parsed page info
    tracing::info!("Parse successful: title={}", page_info.title);
    (
        StatusCode::OK,
        Json(json!({
            "title": page_info.title,
            "actors": page_info.actors,
            "videos": page_info.video_infos.iter().map(|v| json!({
                "bvid": v.bvid,
                "cid": v.cid,
                "epid": v.epid,
                "name": v.name,
                "cover": v.cover_image,
                "is_live": v.live,
            })).collect::<Vec<_>>(),
        })),
    )
        .into_response()
}

/// Generate Bilibili QR code for login
async fn login_qr(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
) -> impl IntoResponse {
    tracing::info!("Bilibili login QR request");

    // Determine which client to use (remote or local)
    let client = if let Some(instance_name) = query.instance_name {
        if let Some(channel) = state.provider_instance_manager.get(&instance_name).await {
            tracing::debug!("Using remote Bilibili instance: {}", instance_name);
            create_remote_bilibili_client(channel)
        } else {
            tracing::warn!(
                "Remote instance '{}' not found, falling back to local",
                instance_name
            );
            load_local_bilibili_client()
        }
    } else {
        tracing::debug!("Using local Bilibili client");
        load_local_bilibili_client()
    };

    // Call new_qr_code
    match client
        .new_qr_code(synctv_providers::grpc::bilibili::Empty {})
        .await
    {
        Ok(resp) => {
            tracing::info!("QR code generated successfully");
            (
                StatusCode::OK,
                Json(QrCodeResponse {
                    url: resp.url,
                    key: resp.key,
                }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to generate QR code: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to generate QR code",
                    "message": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Check Bilibili QR code login status
async fn qr_check(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<BilibiliQRLoginRequest>,
) -> impl IntoResponse {
    tracing::info!("Bilibili QR check: {}", req.key);

    // Determine which client to use (remote or local)
    let client = if let Some(instance_name) = query.instance_name {
        if let Some(channel) = state.provider_instance_manager.get(&instance_name).await {
            tracing::debug!("Using remote Bilibili instance: {}", instance_name);
            create_remote_bilibili_client(channel)
        } else {
            tracing::warn!(
                "Remote instance '{}' not found, falling back to local",
                instance_name
            );
            load_local_bilibili_client()
        }
    } else {
        tracing::debug!("Using local Bilibili client");
        load_local_bilibili_client()
    };

    // Build proto request
    let check_req = synctv_providers::grpc::bilibili::LoginWithQrCodeReq { key: req.key };

    // Call login_with_qr_code
    match client.login_with_qr_code(check_req).await {
        Ok(resp) => {
            tracing::info!("QR status checked: status={}", resp.status);
            (
                StatusCode::OK,
                Json(QrStatusResponse {
                    status: resp.status,
                    cookies: resp.cookies,
                }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to check QR status: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to check QR status",
                    "message": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Get captcha for SMS login
async fn new_captcha(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
) -> impl IntoResponse {
    tracing::info!("Bilibili new captcha request");

    let client = if let Some(instance_name) = query.instance_name {
        if let Some(channel) = state.provider_instance_manager.get(&instance_name).await {
            create_remote_bilibili_client(channel)
        } else {
            load_local_bilibili_client()
        }
    } else {
        load_local_bilibili_client()
    };

    match client
        .new_captcha(synctv_providers::grpc::bilibili::Empty {})
        .await
    {
        Ok(resp) => (
            StatusCode::OK,
            Json(json!({
                "token": resp.token,
                "gt": resp.gt,
                "challenge": resp.challenge,
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to get captcha: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to get captcha",
                    "message": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Send SMS verification code
async fn sms_send(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<BilibiliSMSSendRequest>,
) -> impl IntoResponse {
    tracing::info!("Bilibili SMS send request: phone={}", req.phone);

    let client = if let Some(instance_name) = query.instance_name {
        if let Some(channel) = state.provider_instance_manager.get(&instance_name).await {
            create_remote_bilibili_client(channel)
        } else {
            load_local_bilibili_client()
        }
    } else {
        load_local_bilibili_client()
    };

    let sms_req = synctv_providers::grpc::bilibili::NewSmsReq {
        phone: req.phone,
        token: req.token,
        challenge: req.challenge,
        validate: req.validate,
    };

    match client.new_sms(sms_req).await {
        Ok(resp) => (
            StatusCode::OK,
            Json(json!({
                "captcha_key": resp.captcha_key,
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to send SMS: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to send SMS",
                    "message": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Login with SMS code
async fn sms_login(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<BilibiliSMSLoginRequest>,
) -> impl IntoResponse {
    tracing::info!("Bilibili SMS login request: phone={}", req.phone);

    let client = if let Some(instance_name) = query.instance_name {
        if let Some(channel) = state.provider_instance_manager.get(&instance_name).await {
            create_remote_bilibili_client(channel)
        } else {
            load_local_bilibili_client()
        }
    } else {
        load_local_bilibili_client()
    };

    let login_req = synctv_providers::grpc::bilibili::LoginWithSmsReq {
        phone: req.phone,
        code: req.code,
        captcha_key: req.captcha_key,
    };

    match client.login_with_sms(login_req).await {
        Ok(resp) => (
            StatusCode::OK,
            Json(json!({
                "cookies": resp.cookies,
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to login with SMS: {}", e);
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "Login failed",
                    "message": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Get Bilibili user info
async fn user_info(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<BilibiliUserInfoRequest>,
) -> impl IntoResponse {
    tracing::info!("Bilibili user info request");

    let client = if let Some(instance_name) = query.instance_name {
        if let Some(channel) = state.provider_instance_manager.get(&instance_name).await {
            create_remote_bilibili_client(channel)
        } else {
            load_local_bilibili_client()
        }
    } else {
        load_local_bilibili_client()
    };

    let info_req = synctv_providers::grpc::bilibili::UserInfoReq {
        cookies: req.cookies,
    };

    match client.user_info(info_req).await {
        Ok(resp) => (
            StatusCode::OK,
            Json(json!({
                "is_login": resp.is_login,
                "username": resp.username,
                "face": resp.face,
                "is_vip": resp.is_vip,
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to get user info: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to get user info",
                    "message": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Logout (just return success, cookies are client-side)
async fn logout() -> impl IntoResponse {
    tracing::info!("Bilibili logout request");
    (
        StatusCode::OK,
        Json(json!({
            "message": "Logout successful"
        })),
    )
        .into_response()
}
