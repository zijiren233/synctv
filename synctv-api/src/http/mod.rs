// Module: http
// HTTP/JSON REST API for backward compatibility and easier integration

pub mod auth;
pub mod room;
pub mod error;
pub mod middleware;

// Provider HTTP route extensions (decoupled via trait)
pub mod providers;
pub mod provider_common;

use axum::{
    Router,
    routing::{get, post},
};
use tower_http::cors::{CorsLayer, Any};
use tower_http::trace::TraceLayer;
use std::sync::Arc;
use synctv_core::service::{UserService, RoomService, ProviderInstanceManager, ProvidersManager};
use synctv_core::repository::UserProviderCredentialRepository;

pub use error::{AppError, AppResult};

/// Shared application state
#[derive(Clone, Debug)]
pub struct AppState {
    pub user_service: Arc<UserService>,
    pub room_service: Arc<RoomService>,
    pub provider_instance_manager: Arc<ProviderInstanceManager>,
    pub providers_manager: Arc<ProvidersManager>,
    pub credential_repository: Arc<UserProviderCredentialRepository>,
}

/// Create the HTTP router with all routes
pub fn create_router(
    user_service: Arc<UserService>,
    room_service: Arc<RoomService>,
    provider_instance_manager: Arc<ProviderInstanceManager>,
    providers_manager: Arc<ProvidersManager>,
    credential_repository: Arc<UserProviderCredentialRepository>,
) -> Router {
    let state = AppState {
        user_service,
        room_service,
        provider_instance_manager: provider_instance_manager.clone(),
        providers_manager: providers_manager.clone(),
        credential_repository,
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

        // Provider routes (decoupled via registry pattern)
        .nest("/api/providers", build_provider_routes(&state))

        .with_state(state)
        .layer(CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any))
        .layer(TraceLayer::new_for_http())
}

/// Build provider routes by composing individual provider routers
///
/// Uses registry pattern for complete decoupling:
/// - Each provider module self-registers its routes via init()
/// - No central code needs to know about specific provider types
/// - Adding new providers requires zero changes here!
/// - /api/providers/bilibili/* - Bilibili routes (self-registered)
/// - /api/providers/alist/* - Alist routes (self-registered)
/// - /api/providers/emby/* - Emby routes (self-registered)
/// - /api/providers/backends/:vendor - Common route
fn build_provider_routes(_state: &AppState) -> Router<AppState> {
    // Initialize all provider route modules (triggers self-registration)
    providers::bilibili::init();
    providers::alist::init();
    providers::emby::init();

    // Start with common routes
    let mut router = provider_common::register_common_routes();

    // Collect all registered provider routes (no knowledge of specific types!)
    router = router.merge(providers::build_provider_routes());

    router
}

/// Health check endpoint
async fn health_check() -> &'static str {
    "OK"
}
