//! Emby Provider HTTP Routes

use axum::{
    Router,
    routing::{post, get},
    extract::{Query, State},
    response::IntoResponse,
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synctv_core::provider::provider_client::{load_local_emby_client, create_remote_emby_client};

use crate::http::AppState;

/// Emby login request
#[derive(Debug, Deserialize)]
pub struct EmbyLoginRequest {
    pub host: String,
    pub api_key: String,
}

/// Emby login response (user info)
#[derive(Debug, Serialize)]
pub struct EmbyLoginResponse {
    pub user_id: String,
    pub username: String,
    pub is_admin: bool,
}

/// Emby list request
#[derive(Debug, Deserialize)]
pub struct EmbyListRequest {
    pub host: String,
    pub token: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub start_index: u64,
    #[serde(default)]
    pub limit: u64,
    #[serde(default)]
    pub search_term: String,
    #[serde(default)]
    pub user_id: String,
}

/// Emby me request
#[derive(Debug, Deserialize)]
pub struct EmbyMeRequest {
    pub host: String,
    pub token: String,
}

/// Backend query parameter
#[derive(Debug, Deserialize)]
pub struct BackendQuery {
    /// Optional backend instance name for remote provider
    #[serde(default)]
    pub backend: Option<String>,
}

/// Build Emby HTTP routes
fn emby_routes() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/list", post(list))
        .route("/me", get(me))
        .route("/binds", get(binds))
}

/// Self-register Emby routes on module load
pub fn init() {
    super::register_route_builder(|| {
        ("emby".to_string(), emby_routes())
    });
}

// Handlers

/// Login to Emby/Jellyfin (validate API key)
async fn login(
    State(state): State<AppState>,
    Query(query): Query<BackendQuery>,
    Json(req): Json<EmbyLoginRequest>,
) -> impl IntoResponse {
    tracing::info!("Emby login request: host={}", req.host);

    // Determine which client to use (remote or local)
    let client = if let Some(backend_name) = query.backend {
        if let Some(channel) = state.provider_instance_manager.get(&backend_name).await {
            tracing::debug!("Using remote Emby instance: {}", backend_name);
            create_remote_emby_client(channel)
        } else {
            tracing::warn!("Remote instance '{}' not found, falling back to local", backend_name);
            load_local_emby_client()
        }
    } else {
        tracing::debug!("Using local Emby client");
        load_local_emby_client()
    };

    // Build proto request to get user info (validates API key)
    // Note: user_id can be empty to get current user info
    let me_req = synctv_providers::grpc::emby::MeReq {
        host: req.host.clone(),
        token: req.api_key,
        user_id: String::new(), // Empty = get current user
    };

    // Call me() to validate API key and get user info
    match client.me(me_req).await {
        Ok(user_info) => {
            tracing::info!("Emby login successful: user_id={}", user_info.id);
            (
                StatusCode::OK,
                Json(EmbyLoginResponse {
                    user_id: user_info.id,
                    username: user_info.name,
                    // Note: MeResp doesn't include policy info, set to false for now
                    // TODO: Add a separate call to check if user is admin if needed
                    is_admin: false,
                })
            ).into_response()
        }
        Err(e) => {
            tracing::error!("Emby login failed: {}", e);
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "Login failed",
                    "message": e.to_string()
                }))
            ).into_response()
        }
    }
}

/// List Emby library items
async fn list(
    State(state): State<AppState>,
    Query(query): Query<BackendQuery>,
    Json(req): Json<EmbyListRequest>,
) -> impl IntoResponse {
    tracing::info!("Emby list request: host={}, path={}", req.host, req.path);

    let client = if let Some(backend_name) = query.backend {
        if let Some(channel) = state.provider_instance_manager.get(&backend_name).await {
            create_remote_emby_client(channel)
        } else {
            load_local_emby_client()
        }
    } else {
        load_local_emby_client()
    };

    let list_req = synctv_providers::grpc::emby::FsListReq {
        host: req.host,
        token: req.token,
        path: req.path,
        start_index: req.start_index,
        limit: req.limit,
        search_term: req.search_term,
        user_id: req.user_id,
    };

    match client.fs_list(list_req).await {
        Ok(resp) => {
            // Convert items to JSON
            let items: Vec<_> = resp.items.into_iter().map(|item| {
                json!({
                    "id": item.id,
                    "name": item.name,
                    "type": item.r#type,
                    "parent_id": item.parent_id,
                    "series_name": item.series_name,
                    "series_id": item.series_id,
                    "season_name": item.season_name,
                })
            }).collect();

            (
                StatusCode::OK,
                Json(json!({
                    "items": items,
                    "total": resp.total,
                }))
            ).into_response()
        }
        Err(e) => {
            tracing::error!("Emby list failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "List failed",
                    "message": e.to_string()
                }))
            ).into_response()
        }
    }
}

/// Get Emby user info
async fn me(
    State(state): State<AppState>,
    Query(query): Query<BackendQuery>,
    Json(req): Json<EmbyMeRequest>,
) -> impl IntoResponse {
    tracing::info!("Emby me request: host={}", req.host);

    let client = if let Some(backend_name) = query.backend {
        if let Some(channel) = state.provider_instance_manager.get(&backend_name).await {
            create_remote_emby_client(channel)
        } else {
            load_local_emby_client()
        }
    } else {
        load_local_emby_client()
    };

    let me_req = synctv_providers::grpc::emby::MeReq {
        host: req.host,
        token: req.token,
        user_id: String::new(), // Empty = get current user
    };

    match client.me(me_req).await {
        Ok(resp) => {
            (
                StatusCode::OK,
                Json(json!({
                    "id": resp.id,
                    "name": resp.name,
                }))
            ).into_response()
        }
        Err(e) => {
            tracing::error!("Emby me failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Get user info failed",
                    "message": e.to_string()
                }))
            ).into_response()
        }
    }
}

/// Logout from Emby
async fn logout() -> impl IntoResponse {
    tracing::info!("Emby logout request");
    (
        StatusCode::OK,
        Json(json!({
            "message": "Logout successful"
        }))
    ).into_response()
}

/// Get Emby binds (saved credentials)
async fn binds(
    State(_state): State<AppState>,
) -> impl IntoResponse {
    tracing::info!("Emby binds request");

    // TODO: Implement getting saved Emby credentials from database
    // This would query UserProviderCredential table

    (
        StatusCode::OK,
        Json(json!({
            "binds": []
        }))
    ).into_response()
}
