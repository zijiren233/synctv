//! Alist Provider HTTP Routes

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
use synctv_core::provider::provider_client::{load_local_alist_client, create_remote_alist_client};

use crate::http::AppState;

/// Alist login request
#[derive(Debug, Deserialize)]
pub struct AlistLoginRequest {
    pub host: String,
    pub username: String,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub hashed_password: Option<String>,
}

/// Alist login response
#[derive(Debug, Serialize)]
pub struct AlistLoginResponse {
    pub token: String,
}

/// Alist list request
#[derive(Debug, Deserialize)]
pub struct AlistListRequest {
    pub host: String,
    pub token: String,
    pub path: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub page: u64,
    #[serde(default)]
    pub per_page: u64,
    #[serde(default)]
    pub refresh: bool,
}

/// Alist me request
#[derive(Debug, Deserialize)]
pub struct AlistMeRequest {
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

/// Build Alist HTTP routes
fn alist_routes() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/list", post(list))
        .route("/me", get(me))
        .route("/binds", get(binds))
}

/// Self-register Alist routes on module load
pub fn init() {
    super::register_route_builder(|| {
        ("alist".to_string(), alist_routes())
    });
}

// Handlers

/// Login to Alist
///
/// Authenticates with Alist server and returns a token.
async fn login(
    State(state): State<AppState>,
    Query(query): Query<BackendQuery>,
    Json(req): Json<AlistLoginRequest>,
) -> impl IntoResponse {
    tracing::info!("Alist login request: host={}, username={}", req.host, req.username);

    // Determine which client to use (remote or local)
    let client = if let Some(backend_name) = query.backend {
        // Try to get remote instance
        if let Some(channel) = state.provider_instance_manager.get(&backend_name).await {
            tracing::debug!("Using remote Alist instance: {}", backend_name);
            create_remote_alist_client(channel)
        } else {
            tracing::warn!("Remote instance '{}' not found, falling back to local", backend_name);
            load_local_alist_client()
        }
    } else {
        // Use local singleton client
        tracing::debug!("Using local Alist client");
        load_local_alist_client()
    };

    // Prepare password (use hashed if provided, otherwise use plain)
    let (password, is_hashed) = if let Some(hashed_pwd) = req.hashed_password {
        (hashed_pwd, true)
    } else {
        (req.password.unwrap_or_default(), false)
    };

    // Build proto request
    let login_req = synctv_providers::grpc::alist::LoginReq {
        host: req.host,
        username: req.username,
        password,
        hashed: is_hashed,
    };

    // Call login
    match client.login(login_req).await {
        Ok(token) => {
            tracing::info!("Alist login successful");
            (
                StatusCode::OK,
                Json(AlistLoginResponse { token })
            ).into_response()
        }
        Err(e) => {
            tracing::error!("Alist login failed: {}", e);
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

/// List Alist directory
async fn list(
    State(state): State<AppState>,
    Query(query): Query<BackendQuery>,
    Json(req): Json<AlistListRequest>,
) -> impl IntoResponse {
    tracing::info!("Alist list request: host={}, path={}", req.host, req.path);

    let client = if let Some(backend_name) = query.backend {
        if let Some(channel) = state.provider_instance_manager.get(&backend_name).await {
            create_remote_alist_client(channel)
        } else {
            load_local_alist_client()
        }
    } else {
        load_local_alist_client()
    };

    let list_req = synctv_providers::grpc::alist::FsListReq {
        host: req.host,
        token: req.token,
        path: req.path,
        password: req.password,
        page: req.page,
        per_page: req.per_page,
        refresh: req.refresh,
    };

    match client.fs_list(list_req).await {
        Ok(resp) => {
            // Convert FsListContent items to JSON
            let content: Vec<_> = resp.content.into_iter().map(|item| {
                json!({
                    "name": item.name,
                    "size": item.size,
                    "is_dir": item.is_dir,
                    "modified": item.modified,
                    "sign": item.sign,
                    "thumb": item.thumb,
                    "type": item.r#type,
                })
            }).collect();

            (
                StatusCode::OK,
                Json(json!({
                    "content": content,
                    "total": resp.total,
                }))
            ).into_response()
        }
        Err(e) => {
            tracing::error!("Alist list failed: {}", e);
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

/// Get Alist user info
async fn me(
    State(state): State<AppState>,
    Query(query): Query<BackendQuery>,
    Json(req): Json<AlistMeRequest>,
) -> impl IntoResponse {
    tracing::info!("Alist me request: host={}", req.host);

    let client = if let Some(backend_name) = query.backend {
        if let Some(channel) = state.provider_instance_manager.get(&backend_name).await {
            create_remote_alist_client(channel)
        } else {
            load_local_alist_client()
        }
    } else {
        load_local_alist_client()
    };

    let me_req = synctv_providers::grpc::alist::MeReq {
        host: req.host,
        token: req.token,
    };

    match client.me(me_req).await {
        Ok(resp) => {
            (
                StatusCode::OK,
                Json(json!({
                    "username": resp.username,
                    "base_path": resp.base_path,
                }))
            ).into_response()
        }
        Err(e) => {
            tracing::error!("Alist me failed: {}", e);
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

/// Logout from Alist
async fn logout() -> impl IntoResponse {
    tracing::info!("Alist logout request");
    (
        StatusCode::OK,
        Json(json!({
            "message": "Logout successful"
        }))
    ).into_response()
}

/// Get Alist binds (saved credentials)
async fn binds(
    auth: crate::http::middleware::AuthUser,
    State(state): State<AppState>,
) -> impl IntoResponse {
    tracing::info!("Alist binds request for user: {}", auth.user_id);

    // Query saved Alist credentials for current user
    match state.credential_repository.get_by_user(&auth.user_id.to_string()).await {
        Ok(credentials) => {
            // Filter for Alist provider only
            let alist_binds: Vec<_> = credentials
                .into_iter()
                .filter(|c| c.provider == "alist")
                .map(|c| {
                    // Parse credential data to extract host
                    let host = c.credential_data.get("host")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    let username = c.credential_data.get("username")
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
                Json(json!({
                    "binds": alist_binds
                }))
            ).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to query credentials: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to query credentials",
                    "message": e.to_string()
                }))
            ).into_response()
        }
    }
}
