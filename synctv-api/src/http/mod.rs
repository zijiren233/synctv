// Module: http
// HTTP/JSON REST API for backward compatibility and easier integration

pub mod auth;
pub mod email_verification;
pub mod error;
pub mod health;
pub mod media;
pub mod media_proxy;
pub mod middleware;
pub mod notifications;
pub mod oauth2;
pub mod public;
pub mod publish_key;
pub mod room;
pub mod room_extra;
pub mod user;
pub mod websocket;

// Provider HTTP routes
// Provider-specific HTTP endpoints are registered from provider instances
pub mod provider_common;
pub mod providers;

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use synctv_cluster::sync::PublishRequest;
use synctv_core::provider::{AlistProvider, BilibiliProvider, EmbyProvider};
use synctv_core::repository::{ProviderInstanceRepository, UserProviderCredentialRepository};
use synctv_core::service::{ProviderInstanceManager, RoomService, UserService};
use synctv_stream::streaming::{create_streaming_router, StreamingHttpState};
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub use error::{AppError, AppResult};

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub user_service: Arc<UserService>,
    pub room_service: Arc<RoomService>,
    pub provider_instance_manager: Arc<ProviderInstanceManager>,
    pub user_provider_credential_repository: Arc<UserProviderCredentialRepository>,
    pub alist_provider: Arc<AlistProvider>,
    pub bilibili_provider: Arc<BilibiliProvider>,
    pub emby_provider: Arc<EmbyProvider>,
    pub message_hub: Arc<synctv_cluster::sync::RoomMessageHub>,
    pub cluster_manager: Option<Arc<synctv_cluster::sync::ClusterManager>>,
    pub jwt_service: synctv_core::service::JwtService,
    pub redis_publish_tx: Option<mpsc::UnboundedSender<PublishRequest>>,
    pub oauth2_service: Option<Arc<synctv_core::service::OAuth2Service>>,
    pub settings_service: Option<Arc<synctv_core::service::SettingsService>>,
    pub settings_registry: Option<Arc<synctv_core::service::SettingsRegistry>>,
    pub email_service: Option<Arc<synctv_core::service::EmailService>>,
    pub publish_key_service: Option<Arc<synctv_core::service::PublishKeyService>>,
    pub notification_service: Option<Arc<synctv_core::service::UserNotificationService>>,
    // Unified API implementation layer
    pub client_api: Arc<crate::impls::ClientApiImpl>,
    pub admin_api: Option<Arc<crate::impls::AdminApiImpl>>,
}

/// Create the HTTP router with all routes
pub fn create_router(
    user_service: Arc<UserService>,
    room_service: Arc<RoomService>,
    provider_instance_manager: Arc<ProviderInstanceManager>,
    provider_instance_repository: Arc<ProviderInstanceRepository>,
    user_provider_credential_repository: Arc<UserProviderCredentialRepository>,
    alist_provider: Arc<AlistProvider>,
    bilibili_provider: Arc<BilibiliProvider>,
    emby_provider: Arc<EmbyProvider>,
    message_hub: Arc<synctv_cluster::sync::RoomMessageHub>,
    cluster_manager: Option<Arc<synctv_cluster::sync::ClusterManager>>,
    jwt_service: synctv_core::service::JwtService,
    redis_publish_tx: Option<mpsc::UnboundedSender<PublishRequest>>,
    streaming_state: Option<StreamingHttpState>,
    oauth2_service: Option<Arc<synctv_core::service::OAuth2Service>>,
    settings_service: Option<Arc<synctv_core::service::SettingsService>>,
    settings_registry: Option<Arc<synctv_core::service::SettingsRegistry>>,
    email_service: Option<Arc<synctv_core::service::EmailService>>,
    publish_key_service: Option<Arc<synctv_core::service::PublishKeyService>>,
    notification_service: Option<Arc<synctv_core::service::UserNotificationService>>,
) -> axum::Router {
    // Create the unified API implementation layer
    let client_api = Arc::new(crate::impls::ClientApiImpl::new(
        user_service.clone(),
        room_service.clone(),
    ));

    // AdminApi requires SettingsService and EmailService
    // If they're not configured, we need to handle this appropriately
    // For now, we'll skip creating admin_api if these aren't available
    let admin_api = if let (Some(settings_svc), Some(email_svc)) = (&settings_service, &email_service) {
        Some(Arc::new(crate::impls::AdminApiImpl::new(
            room_service.clone(),
            user_service.clone(),
            settings_svc.clone(),
            email_svc.clone(),
        )))
    } else {
        None
    };

    let state = AppState {
        user_service,
        room_service,
        provider_instance_manager: provider_instance_manager.clone(),
        user_provider_credential_repository,
        alist_provider,
        bilibili_provider,
        emby_provider,
        message_hub,
        cluster_manager,
        jwt_service,
        redis_publish_tx,
        oauth2_service,
        settings_service,
        settings_registry,
        email_service,
        publish_key_service,
        notification_service,
        client_api,
        admin_api,
    };

    // Note: If admin_api is None, admin endpoints won't work
    // This is acceptable for configurations without email/settings services

    let mut router = Router::new()
        // Health check endpoints (for monitoring probes)
        .merge(health::create_health_router())
        // Public endpoints (no authentication required)
        .merge(public::create_public_router())
        // Email verification and password reset
        .merge(email_verification::create_email_router())
        // Publish key routes
        .merge(publish_key::create_publish_key_router())
        // Notification routes
        .merge(notifications::create_notification_router())
        // Media proxy routes
        .merge(media_proxy::create_media_proxy_router())
        // Authentication routes
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/refresh", post(auth::refresh_token))
        // OAuth2 routes
        .route(
            "/api/oauth2/:provider/authorize",
            get(oauth2::get_authorize_url),
        )
        .route(
            "/api/oauth2/:provider/callback",
            get(oauth2::oauth2_callback_get),
        )
        .route(
            "/api/oauth2/:provider/callback",
            post(oauth2::oauth2_callback_post),
        )
        .route("/api/oauth2/:provider/bind", post(oauth2::bind_provider))
        .route(
            "/api/oauth2/:provider/bind",
            axum::routing::delete(oauth2::unbind_provider),
        )
        .route("/api/oauth2/providers", get(oauth2::list_providers))
        // User management routes
        .route("/api/user/me", get(user::get_me))
        .route("/api/user/logout", post(user::logout))
        .route("/api/user/username", post(user::update_username))
        .route("/api/user/password", post(user::update_password))
        .route("/api/user/rooms", get(user::get_my_rooms))
        .route("/api/user/rooms/joined", get(user::get_joined_rooms))
        .route(
            "/api/user/rooms/:room_id",
            axum::routing::delete(user::delete_my_room),
        )
        .route("/api/user/rooms/:room_id/exit", post(user::exit_room))
        // Room discovery routes (public)
        .route("/api/room/check/:room_id", get(room::check_room))
        .route("/api/room/list", get(room::list_rooms))
        .route("/api/room/hot", get(room::hot_rooms))
        // Room management routes
        .route("/api/rooms", post(room::create_room))
        .route("/api/rooms/:room_id", get(room::get_room))
        .route("/api/rooms/:room_id/join", post(room::join_room))
        .route("/api/rooms/:room_id/leave", post(room::leave_room))
        .route("/api/rooms/:room_id/settings", get(room::get_room_settings))
        .route("/api/rooms/:room_id/members", get(room::get_room_members))
        .route("/api/rooms/:room_id/pwd/check", post(room::check_password))
        .route(
            "/api/rooms/:room_id",
            axum::routing::delete(room::delete_room),
        )
        // Room admin routes
        .route(
            "/api/rooms/:room_id/admin/settings",
            post(room::set_room_settings_admin),
        )
        .route(
            "/api/rooms/:room_id/admin/password",
            post(room::set_room_password),
        )
        // Media/playlist routes
        .route("/api/rooms/:room_id/media", post(room::add_media))
        .route("/api/rooms/:room_id/media", get(room::get_playlist))
        .route(
            "/api/rooms/:room_id/media/batch",
            post(room::push_media_batch),
        )
        .route(
            "/api/rooms/:room_id/media/:media_id",
            axum::routing::delete(room::remove_media),
        )
        .route(
            "/api/rooms/:room_id/media/:media_id/edit",
            post(room::edit_media),
        )
        .route(
            "/api/rooms/:room_id/media/swap",
            post(room::swap_media_items),
        )
        .route(
            "/api/rooms/:room_id/media/clear",
            post(room::clear_playlist),
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
        .route(
            "/ws/rooms/:room_id",
            axum::routing::get(websocket::websocket_handler),
        )
        // Provider-specific HTTP routes
        .nest("/api/providers/bilibili", providers::bilibili::bilibili_routes())
        .nest("/api/providers/alist", providers::alist::alist_routes())
        .nest("/api/providers/emby", providers::emby::emby_routes());

    // Note: Streaming routes (/live, /hls) are not merged here because they use StreamingHttpState
    // which is incompatible with our AppState. They should be run on a separate port or service.

    // Apply layers before state
    let router = router
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http());

    // Apply state to all routes (must be last)
    router.with_state(state)
}
