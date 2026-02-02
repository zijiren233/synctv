//! Emby Provider HTTP Routes

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;

use crate::http::{AppState, provider_common::{InstanceQuery, error_response, parse_provider_error}};
use crate::impls::EmbyApiImpl;

/// Build Emby HTTP routes
pub fn emby_routes() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/list", post(list))
        .route("/me", get(me))
        .route("/binds", get(binds))
}

// Handlers

/// Login to Emby/Jellyfin (validate API key)
async fn login(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    tracing::info!("Emby login request");

    let proto_req = match serde_json::from_value::<crate::proto::providers::emby::LoginRequest>(req) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid request: {}", e)})),
            ).into_response();
        }
    };

    let api = EmbyApiImpl::new(state.emby_provider.clone());

    match api.login(proto_req, query.as_deref()).await {
        Ok(resp) => {
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Emby login failed: {}", e);
            error_response(parse_provider_error(&e)).into_response()
        }
    }
}

/// List Emby library items
async fn list(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    tracing::info!("Emby list request");

    let proto_req = match serde_json::from_value::<crate::proto::providers::emby::ListRequest>(req) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid request: {}", e)})),
            ).into_response();
        }
    };

    let api = EmbyApiImpl::new(state.emby_provider.clone());

    match api.list(proto_req, query.as_deref()).await {
        Ok(resp) => {
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Emby list failed: {}", e);
            error_response(parse_provider_error(&e)).into_response()
        }
    }
}

/// Get Emby user info
async fn me(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    tracing::info!("Emby me request");

    let proto_req = match serde_json::from_value::<crate::proto::providers::emby::GetMeRequest>(req) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid request: {}", e)})),
            ).into_response();
        }
    };

    let api = EmbyApiImpl::new(state.emby_provider.clone());

    match api.get_me(proto_req, query.as_deref()).await {
        Ok(resp) => {
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Emby me failed: {}", e);
            error_response(parse_provider_error(&e)).into_response()
        }
    }
}

/// Logout from Emby
async fn logout() -> impl IntoResponse {
    tracing::info!("Emby logout request");
    (
        StatusCode::OK,
        Json(json!({"message": "Logout successful"})),
    )
        .into_response()
}

/// Get Emby binds (saved credentials)
async fn binds(
    auth: crate::http::middleware::AuthUser,
    State(state): State<AppState>,
) -> impl IntoResponse {
    tracing::info!("Emby binds request for user: {}", auth.user_id);

    // Query saved Emby credentials for current user
    match state
        .user_provider_credential_repository
        .get_by_user(&auth.user_id.to_string())
        .await
    {
        Ok(credentials) => {
            // Filter for Emby provider only
            let emby_binds: Vec<_> = credentials
                .into_iter()
                .filter(|c| c.provider == "emby")
                .map(|c| {
                    // Parse credential data to extract host
                    let host = c
                        .credential_data
                        .get("host")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    let emby_user_id = c
                        .credential_data
                        .get("emby_user_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    json!({
                        "id": c.id,
                        "host": host,
                        "user_id": emby_user_id,
                        "created_at": c.created_at.to_rfc3339(),
                    })
                })
                .collect();

            (
                StatusCode::OK,
                Json(json!({"binds": emby_binds})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to query credentials: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to query credentials", "message": e.to_string()})),
            )
                .into_response()
        }
    }
}
