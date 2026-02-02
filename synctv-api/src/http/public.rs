//! Public API endpoints
//!
//! Endpoints that can be accessed without authentication.

use axum::{
    extract::State,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};

use crate::http::AppState;

/// Public settings response
#[derive(Debug, Serialize, Deserialize)]
pub struct PublicSettings {
    /// Whether user registration is enabled
    pub signup_enabled: bool,
    /// Whether creating rooms is enabled
    pub allow_room_creation: bool,
    /// Maximum rooms per user (0 = unlimited)
    pub max_rooms_per_user: i64,
    /// Maximum members per room (0 = unlimited)
    pub max_members_per_room: i64,
}

/// Create public API router
pub fn create_public_router() -> Router<AppState> {
    Router::new().route("/api/public/settings", get(get_public_settings))
}

/// Get public server settings
///
/// This endpoint can be called without authentication and returns
/// public server configuration that clients need to know.
pub async fn get_public_settings(State(state): State<AppState>) -> impl IntoResponse {
    let signup_enabled = if let Some(ref registry) = state.settings_registry {
        registry.signup_enabled.get().unwrap_or(true)
    } else {
        true // Default to true if settings not available
    };

    let allow_room_creation = if let Some(ref registry) = state.settings_registry {
        registry.allow_room_creation.get().unwrap_or(true)
    } else {
        true
    };

    let max_rooms_per_user = if let Some(ref registry) = state.settings_registry {
        registry.max_rooms_per_user.get().unwrap_or(10)
    } else {
        10
    };

    let max_members_per_room = if let Some(ref registry) = state.settings_registry {
        registry.max_members_per_room.get().unwrap_or(100)
    } else {
        100
    };

    Json(PublicSettings {
        signup_enabled,
        allow_room_creation,
        max_rooms_per_user,
        max_members_per_room,
    })
}
