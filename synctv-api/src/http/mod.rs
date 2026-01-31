// Module: http
// HTTP/JSON REST API for backward compatibility and easier integration

pub mod auth;
pub mod room;
pub mod error;
pub mod middleware;
pub mod provider_routes;

use axum::{
    Router,
    routing::{get, post},
};
use tower_http::cors::{CorsLayer, Any};
use tower_http::trace::TraceLayer;
use std::sync::Arc;
use synctv_core::service::{UserService, RoomService};

pub use error::{AppError, AppResult};

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub user_service: Arc<UserService>,
    pub room_service: Arc<RoomService>,
}

/// Create the HTTP router with all routes
pub fn create_router(
    user_service: Arc<UserService>,
    room_service: Arc<RoomService>,
) -> Router {
    let state = AppState {
        user_service,
        room_service,
    };

    Router::new()
        // Health check
        .route("/health", get(health_check))

        // Authentication routes
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/refresh", post(auth::refresh_token))

        // Room management routes
        .route("/api/rooms", post(room::create_room))
        .route("/api/rooms/:room_id", get(room::get_room))
        .route("/api/rooms/:room_id/join", post(room::join_room))
        .route("/api/rooms/:room_id/leave", post(room::leave_room))
        .route("/api/rooms/:room_id", axum::routing::delete(room::delete_room))

        // Media/playlist routes
        .route("/api/rooms/:room_id/media", post(room::add_media))
        .route("/api/rooms/:room_id/media", get(room::get_playlist))
        .route("/api/rooms/:room_id/media/:media_id", axum::routing::delete(room::remove_media))

        // Playback control routes
        .route("/api/rooms/:room_id/playback/play", post(room::play))
        .route("/api/rooms/:room_id/playback/pause", post(room::pause))
        .route("/api/rooms/:room_id/playback/seek", post(room::seek))
        .route("/api/rooms/:room_id/playback/speed", post(room::change_speed))
        .route("/api/rooms/:room_id/playback/switch", post(room::switch_media))
        .route("/api/rooms/:room_id/playback", get(room::get_playback_state))

        // Provider routes (parse, login, proxy)
        .nest("/api/providers", provider_routes::build_provider_routes())

        .with_state(state)
        .layer(CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any))
        .layer(TraceLayer::new_for_http())
}

/// Health check endpoint
async fn health_check() -> &'static str {
    "OK"
}
