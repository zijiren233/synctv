//! Public API endpoints
//!
//! Endpoints that can be accessed without authentication.

use axum::{
    extract::State,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};

use synctv_core::service::PublicSettings;

use crate::http::AppState;

/// Create public API router
pub fn create_public_router() -> Router<AppState> {
    Router::new().route("/api/public/settings", get(get_public_settings))
}

/// Get public server settings
///
/// This endpoint can be called without authentication and returns
/// public server configuration that clients need to know.
pub async fn get_public_settings(State(state): State<AppState>) -> impl IntoResponse {
    let settings = match &state.settings_registry {
        Some(reg) => reg.to_public_settings(),
        None => PublicSettings::defaults(),
    };

    Json(settings)
}
