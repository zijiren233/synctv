//! Bilibili Provider HTTP Routes

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;

use crate::http::{AppState, provider_common::{InstanceQuery, error_response, parse_provider_error}};
use crate::impls::BilibiliApiImpl;

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
}

// Handlers

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
