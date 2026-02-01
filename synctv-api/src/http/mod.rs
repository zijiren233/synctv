// Module: http
// HTTP/JSON REST API for backward compatibility and easier integration

pub mod auth;
pub mod error;
pub mod health;
pub mod middleware;
pub mod oauth2;
pub mod room;
pub mod websocket;

// Provider HTTP route extensions (decoupled via trait)
pub mod provider_common;
pub mod providers;

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use synctv_core::repository::ProviderInstanceRepository;
use synctv_core::service::{ProviderInstanceManager, RoomService, UserService};
use synctv_stream::streaming::{create_streaming_router, StreamingHttpState};
use synctv_cluster::sync::PublishRequest;
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub use error::{AppError, AppResult};

/// Shared application state
#[derive(Clone, Debug)]
pub struct AppState {
    pub user_service: Arc<UserService>,
    pub room_service: Arc<RoomService>,
    pub provider_instance_manager: Arc<ProviderInstanceManager>,
    pub provider_instance_repository: Arc<ProviderInstanceRepository>,
    pub message_hub: Arc<synctv_cluster::sync::RoomMessageHub>,
    pub jwt_service: synctv_core::service::JwtService,
    pub redis_publish_tx: Option<mpsc::UnboundedSender<PublishRequest>>,
    pub oauth2_service: Option<Arc<synctv_core::service::OAuth2Service>>,
}

/// Create the HTTP router with all routes
pub fn create_router(
    user_service: Arc<UserService>,
    room_service: Arc<RoomService>,
    provider_instance_manager: Arc<ProviderInstanceManager>,
    provider_instance_repository: Arc<ProviderInstanceRepository>,
    message_hub: Arc<synctv_cluster::sync::RoomMessageHub>,
    jwt_service: synctv_core::service::JwtService,
    redis_publish_tx: Option<mpsc::UnboundedSender<PublishRequest>>,
    streaming_state: Option<StreamingHttpState>,
    oauth2_service: Option<Arc<synctv_core::service::OAuth2Service>>,
) -> Router {
    let state = AppState {
        user_service,
        room_service,
        provider_instance_manager: provider_instance_manager.clone(),
        provider_instance_repository,
        message_hub,
        jwt_service,
        redis_publish_tx,
        oauth2_service,
    };

    let mut router = Router::new()
        // Health check endpoints
        .merge(health::create_health_router())
        // Authentication routes
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/refresh", post(auth::refresh_token))
        // OAuth2 routes
        .route("/api/oauth2/:provider/authorize", get(oauth2::get_authorize_url))
        .route("/api/oauth2/:provider/callback", get(oauth2::oauth2_callback_get))
        .route("/api/oauth2/:provider/callback", post(oauth2::oauth2_callback_post))
        .route("/api/oauth2/:provider/bind", post(oauth2::bind_provider))
        .route("/api/oauth2/:provider/bind", axum::routing::delete(oauth2::unbind_provider))
        .route("/api/oauth2/providers", get(oauth2::list_providers))
        // Room management routes
        .route("/api/rooms", post(room::create_room))
        .route("/api/rooms/:room_id", get(room::get_room))
        .route("/api/rooms/:room_id/join", post(room::join_room))
        .route("/api/rooms/:room_id/leave", post(room::leave_room))
        .route(
            "/api/rooms/:room_id",
            axum::routing::delete(room::delete_room),
        )
        // Media/playlist routes
        .route("/api/rooms/:room_id/media", post(room::add_media))
        .route("/api/rooms/:room_id/media", get(room::get_playlist))
        .route(
            "/api/rooms/:room_id/media/:media_id",
            axum::routing::delete(room::remove_media),
        )
        // Playback control routes
        .route("/api/rooms/:room_id/playback/play", post(room::play))
        .route("/api/rooms/:room_id/playback/pause", post(room::pause))
        .route("/api/rooms/:room_id/playback/seek", post(room::seek))
        .route(
            "/api/rooms/:room_id/playback/speed",
            post(room::change_speed),
        )
        .route(
            "/api/rooms/:room_id/playback/switch",
            post(room::switch_media),
        )
        .route(
            "/api/rooms/:room_id/playback",
            get(room::get_playback_state),
        )
        // WebSocket endpoint for real-time messaging
        .route("/ws/rooms/:room_id", axum::routing::get(websocket::websocket_handler))
        .with_state(state);

    // Add streaming routes if streaming state is provided
    // Routes: /live/:room_id/:media_id.flv, /hls/:room_id/:media_id.m3u8, /hls/:room_id/:media_id/:segment
    if let Some(streaming_state) = streaming_state {
        router = router.merge(create_streaming_router(streaming_state));
    }

    router
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
}

// TODO: Re-enable provider routes when UserProviderCredentialRepository is properly implemented
/*
/// Build provider routes by composing individual provider routers
///
/// Uses registry pattern for complete decoupling:
/// - Each provider module self-registers its routes via init()
/// - No central code needs to know about specific provider types
/// - Adding new providers requires zero changes here!
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
*/

/// Health check endpoint
async fn health_check() -> &'static str {
    "OK"
}
