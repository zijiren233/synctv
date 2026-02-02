//! Alist Provider HTTP Routes

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;

use crate::http::{AppState, provider_common::{InstanceQuery, error_response}};
use crate::impls::AlistApiImpl;

/// Build Alist HTTP routes
pub fn alist_routes() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/list", post(list))
        .route("/me", get(me))
        .route("/binds", get(binds))
}

// Handlers

/// Login to Alist
async fn login(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    tracing::info!("Alist login request");

    // Convert JSON to proto request
    let proto_req = match serde_json::from_value::<crate::proto::providers::alist::LoginRequest>(req) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid request: {}", e)})),
            ).into_response();
        }
    };

    let api = AlistApiImpl::new(state.alist_provider.clone());

    match api.login(proto_req, query.as_deref()).await {
        Ok(resp) => {
            tracing::info!("Alist login successful");
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Alist login failed: {}", e);
            error_response(crate::http::provider_common::parse_provider_error(&e)).into_response()
        }
    }
}

/// List Alist directory
async fn list(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    tracing::info!("Alist list request");

    let proto_req = match serde_json::from_value::<crate::proto::providers::alist::ListRequest>(req) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid request: {}", e)})),
            ).into_response();
        }
    };

    let api = AlistApiImpl::new(state.alist_provider.clone());

    match api.list(proto_req, query.as_deref()).await {
        Ok(resp) => {
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Alist list failed: {}", e);
            error_response(crate::http::provider_common::parse_provider_error(&e)).into_response()
        }
    }
}

/// Get Alist user info
async fn me(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    tracing::info!("Alist me request");

    let proto_req = match serde_json::from_value::<crate::proto::providers::alist::GetMeRequest>(req) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid request: {}", e)})),
            ).into_response();
        }
    };

    let api = AlistApiImpl::new(state.alist_provider.clone());

    match api.get_me(proto_req, query.as_deref()).await {
        Ok(resp) => {
            (StatusCode::OK, Json(json!(resp))).into_response()
        }
        Err(e) => {
            tracing::error!("Alist me failed: {}", e);
            error_response(crate::http::provider_common::parse_provider_error(&e)).into_response()
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

    // Query saved Alist credentials for current user
    match state
        .user_provider_credential_repository
        .get_by_user(&auth.user_id.to_string())
        .await
    {
        Ok(credentials) => {
            // Filter for Alist provider only
            let alist_binds: Vec<_> = credentials
                .into_iter()
                .filter(|c| c.provider == "alist")
                .map(|c| {
                    let host = c
                        .credential_data
                        .get("host")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    let username = c
                        .credential_data
                        .get("username")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    json!({
                        "id": c.id,
                        "host": host,
                        "username": username,
                        "created_at": c.created_at.to_rfc3339(),
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
                Json(json!({"error": "Failed to query credentials", "message": e.to_string()})),
            )
                .into_response()
        }
    }
}
