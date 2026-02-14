// Module: http
// HTTP/JSON REST API for backward compatibility and easier integration

pub mod admin;
pub mod auth;
pub mod email_verification;
pub mod error;
pub mod health;
pub mod live;
pub mod media;
pub mod middleware;
pub mod notifications;
pub mod oauth2;
pub mod openapi;
pub mod public;
pub mod publish_key;
pub mod room;
pub mod room_extra;
pub mod user;
pub mod validation;
pub mod webrtc;
pub mod websocket;

// Provider HTTP routes
// Provider-specific HTTP endpoints are registered from provider instances
pub mod provider_common;
pub mod providers;

use axum::{
    http::{Method, HeaderName, HeaderValue},
    middleware as axum_middleware,
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;
use synctv_cluster::sync::{ClusterEvent, PublishRequest};
use synctv_core::models::{RoomId, MediaId};
use synctv_core::provider::{AlistProvider, BilibiliProvider, EmbyProvider};
use synctv_core::repository::UserProviderCredentialRepository;
use synctv_core::service::{RemoteProviderManager, RoomService, UserService};
use synctv_livestream::api::LiveStreamingInfrastructure;
use tokio::sync::mpsc;
use tower_http::trace::TraceLayer;

pub use error::{AppError, AppResult};

/// Configuration for creating the HTTP router
#[derive(Clone)]
pub struct RouterConfig {
    pub config: Arc<synctv_core::Config>,
    pub user_service: Arc<UserService>,
    pub room_service: Arc<RoomService>,
    pub provider_instance_manager: Arc<RemoteProviderManager>,
    pub user_provider_credential_repository: Arc<UserProviderCredentialRepository>,
    pub alist_provider: Arc<AlistProvider>,
    pub bilibili_provider: Arc<BilibiliProvider>,
    pub emby_provider: Arc<EmbyProvider>,
    pub message_hub: Arc<synctv_cluster::sync::RoomMessageHub>,
    pub cluster_manager: Option<Arc<synctv_cluster::sync::ClusterManager>>,
    pub connection_manager: Arc<synctv_cluster::sync::ConnectionManager>,
    pub jwt_service: synctv_core::service::JwtService,
    pub redis_publish_tx: Option<mpsc::UnboundedSender<PublishRequest>>,
    pub oauth2_service: Option<Arc<synctv_core::service::OAuth2Service>>,
    pub settings_service: Option<Arc<synctv_core::service::SettingsService>>,
    pub settings_registry: Option<Arc<synctv_core::service::SettingsRegistry>>,
    pub email_service: Option<Arc<synctv_core::service::EmailService>>,
    pub publish_key_service: Option<Arc<synctv_core::service::PublishKeyService>>,
    pub notification_service: Option<Arc<synctv_core::service::UserNotificationService>>,
    pub live_streaming_infrastructure: Option<Arc<LiveStreamingInfrastructure>>,
    pub sfu_manager: Option<Arc<synctv_sfu::SfuManager>>,
    pub rate_limiter: synctv_core::service::rate_limit::RateLimiter,
}

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<synctv_core::Config>,
    pub user_service: Arc<UserService>,
    pub room_service: Arc<RoomService>,
    pub provider_instance_manager: Arc<RemoteProviderManager>,
    pub user_provider_credential_repository: Arc<UserProviderCredentialRepository>,
    pub alist_provider: Arc<AlistProvider>,
    pub bilibili_provider: Arc<BilibiliProvider>,
    pub emby_provider: Arc<EmbyProvider>,
    pub message_hub: Arc<synctv_cluster::sync::RoomMessageHub>,
    pub cluster_manager: Option<Arc<synctv_cluster::sync::ClusterManager>>,
    pub connection_manager: Arc<synctv_cluster::sync::ConnectionManager>,
    pub jwt_service: synctv_core::service::JwtService,
    pub redis_publish_tx: Option<mpsc::UnboundedSender<PublishRequest>>,
    pub oauth2_service: Option<Arc<synctv_core::service::OAuth2Service>>,
    pub settings_service: Option<Arc<synctv_core::service::SettingsService>>,
    pub settings_registry: Option<Arc<synctv_core::service::SettingsRegistry>>,
    pub email_service: Option<Arc<synctv_core::service::EmailService>>,
    pub publish_key_service: Option<Arc<synctv_core::service::PublishKeyService>>,
    pub notification_service: Option<Arc<synctv_core::service::UserNotificationService>>,
    pub live_streaming_infrastructure: Option<Arc<LiveStreamingInfrastructure>>,
    pub rate_limiter: synctv_core::service::rate_limit::RateLimiter,
    // Unified API implementation layer
    pub client_api: Arc<crate::impls::ClientApiImpl>,
    pub admin_api: Option<Arc<crate::impls::AdminApiImpl>>,
}

/// Kick a stream both locally and cluster-wide via Redis Pub/Sub.
///
/// Used by HTTP handlers that delete media to terminate any active RTMP stream.
pub(crate) fn kick_stream_cluster(state: &AppState, room_id: &str, media_id: &str, reason: &str) {
    // 1. Local kick (no-op if stream not on this node)
    if let Some(infra) = &state.live_streaming_infrastructure {
        if let Err(e) = infra.kick_publisher(room_id, media_id) {
            tracing::warn!(room_id, media_id, error = %e, "Failed to kick local publisher");
        }
    }

    // 2. Cluster-wide via Redis
    if let Some(tx) = &state.redis_publish_tx {
        if tx.send(PublishRequest {
            room_id: Some(RoomId::from_string(room_id.to_string())),
            event: ClusterEvent::KickPublisher {
                room_id: RoomId::from_string(room_id.to_string()),
                media_id: MediaId::from_string(media_id.to_string()),
                reason: reason.to_string(),
                timestamp: chrono::Utc::now(),
            },
        }).is_err() {
            tracing::warn!(room_id, media_id, "Failed to send cluster-wide kick event (Redis channel closed)");
        }
    }
}

/// Create the HTTP router with all routes
#[allow(clippy::too_many_arguments)]
fn create_router(
    config: Arc<synctv_core::Config>,
    user_service: Arc<UserService>,
    room_service: Arc<RoomService>,
    provider_instance_manager: Arc<RemoteProviderManager>,
    user_provider_credential_repository: Arc<UserProviderCredentialRepository>,
    alist_provider: Arc<AlistProvider>,
    bilibili_provider: Arc<BilibiliProvider>,
    emby_provider: Arc<EmbyProvider>,
    message_hub: Arc<synctv_cluster::sync::RoomMessageHub>,
    cluster_manager: Option<Arc<synctv_cluster::sync::ClusterManager>>,
    connection_manager: Arc<synctv_cluster::sync::ConnectionManager>,
    jwt_service: synctv_core::service::JwtService,
    redis_publish_tx: Option<mpsc::UnboundedSender<PublishRequest>>,
    oauth2_service: Option<Arc<synctv_core::service::OAuth2Service>>,
    settings_service: Option<Arc<synctv_core::service::SettingsService>>,
    settings_registry: Option<Arc<synctv_core::service::SettingsRegistry>>,
    email_service: Option<Arc<synctv_core::service::EmailService>>,
    publish_key_service: Option<Arc<synctv_core::service::PublishKeyService>>,
    notification_service: Option<Arc<synctv_core::service::UserNotificationService>>,
    live_streaming_infrastructure: Option<Arc<LiveStreamingInfrastructure>>,
    sfu_manager: Option<Arc<synctv_sfu::SfuManager>>,
    rate_limiter: synctv_core::service::rate_limit::RateLimiter,
) -> axum::Router {
    // Create the unified API implementation layer
    let client_api = Arc::new(crate::impls::ClientApiImpl::new(
        user_service.clone(),
        room_service.clone(),
        connection_manager.clone(),
        config.clone(),
        sfu_manager,
        publish_key_service.clone(),
        jwt_service.clone(),
        live_streaming_infrastructure.clone(),
        None, // providers_manager - HTTP uses individual provider instances
        settings_registry.clone(),
    ).with_redis_publish_tx(redis_publish_tx.clone()));

    // AdminApi requires SettingsService and EmailService
    // If they're not configured, we need to handle this appropriately
    // For now, we'll skip creating admin_api if these aren't available
    let admin_api = if let (Some(settings_svc), Some(email_svc)) = (&settings_service, &email_service) {
        Some(Arc::new(crate::impls::AdminApiImpl::new(
            room_service.clone(),
            user_service.clone(),
            settings_svc.clone(),
            settings_registry.clone(),
            email_svc.clone(),
            connection_manager.clone(),
            provider_instance_manager.clone(),
            live_streaming_infrastructure.clone(),
            redis_publish_tx.clone(),
        )))
    } else {
        None
    };

    let state = AppState {
        config: config.clone(),
        user_service,
        room_service,
        provider_instance_manager,
        user_provider_credential_repository,
        alist_provider,
        bilibili_provider,
        emby_provider,
        message_hub,
        cluster_manager,
        connection_manager,
        jwt_service,
        redis_publish_tx,
        oauth2_service,
        settings_service,
        settings_registry,
        email_service,
        publish_key_service,
        notification_service,
        live_streaming_infrastructure,
        rate_limiter,
        client_api,
        admin_api,
    };

    // Note: If admin_api is None, admin endpoints won't work
    // This is acceptable for configurations without email/settings services

    // Authentication routes — strict rate limiting (5 req/min)
    let auth_routes = Router::new()
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/refresh", post(auth::refresh_token))
        .route(
            "/api/oauth2/:provider/callback",
            get(oauth2::oauth2_callback_get),
        )
        .route(
            "/api/oauth2/:provider/callback",
            post(oauth2::oauth2_callback_post),
        )
        .route("/api/rooms/:room_id/password/verify", post(room::check_password))
        .route_layer(axum_middleware::from_fn_with_state(
            state.clone(),
            middleware::auth_rate_limit,
        ));

    // Media mutation routes — moderate rate limiting (20 req/min)
    let media_routes = Router::new()
        .route("/api/rooms/:room_id/media", post(room::add_media))
        .route("/api/rooms/:room_id/media", axum::routing::delete(room::clear_playlist))
        .route("/api/rooms/:room_id/media", axum::routing::patch(room::update_media_batch))
        .route(
            "/api/rooms/:room_id/media/batch",
            post(room::push_media_batch),
        )
        .route(
            "/api/rooms/:room_id/media/batch",
            axum::routing::delete(room::remove_media_batch),
        )
        .route(
            "/api/rooms/:room_id/media/reorder",
            post(room::reorder_media_batch),
        )
        .route(
            "/api/rooms/:room_id/media/swap",
            post(room::swap_media_items),
        )
        .route(
            "/api/rooms/:room_id/media/:media_id",
            axum::routing::delete(room::remove_media),
        )
        .route(
            "/api/rooms/:room_id/media/:media_id",
            axum::routing::patch(room::edit_media),
        )
        .route_layer(axum_middleware::from_fn_with_state(
            state.clone(),
            middleware::media_rate_limit,
        ));

    // Write routes — moderate rate limiting (30 req/min)
    let write_routes = Router::new()
        .route("/api/rooms", post(room::create_room))
        .route("/api/rooms/:room_id", axum::routing::delete(room::delete_room))
        .route("/api/rooms/:room_id/members/@me", axum::routing::put(room::join_room))
        .route("/api/rooms/:room_id/members/@me", axum::routing::delete(room::leave_room))
        .route("/api/rooms/:room_id/settings", axum::routing::patch(room::update_room_settings))
        .route("/api/rooms/:room_id/password", axum::routing::patch(room::set_room_password))
        // Individual playback control routes
        .route("/api/rooms/:room_id/playback/play", post(room::play))
        .route("/api/rooms/:room_id/playback/pause", post(room::pause))
        .route("/api/rooms/:room_id/playback/seek", post(room::seek))
        .route("/api/user", axum::routing::patch(user::update_user))
        .route("/api/auth/session", axum::routing::delete(user::logout))
        .route(
            "/api/user/rooms/:room_id",
            axum::routing::delete(user::delete_my_room),
        )
        .route("/api/oauth2/:provider/bind", post(oauth2::bind_provider))
        .route(
            "/api/oauth2/:provider/bind",
            axum::routing::delete(oauth2::unbind_provider),
        )
        .route(
            "/api/rooms/:room_id/members/:user_id",
            axum::routing::delete(room_extra::kick_member),
        )
        .route(
            "/api/rooms/:room_id/members/:user_id",
            axum::routing::patch(room_extra::set_member_permissions),
        )
        .route(
            "/api/rooms/:room_id/bans",
            post(room_extra::ban_member),
        )
        .route(
            "/api/rooms/:room_id/bans/:user_id",
            axum::routing::delete(room_extra::unban_member),
        )
        .route("/api/rooms/:room_id/playback", axum::routing::patch(room::update_playback))
        // Playlist CRUD (write)
        .route("/api/rooms/:room_id/playlists", post(room::create_playlist))
        .route(
            "/api/rooms/:room_id/playlists/:playlist_id",
            axum::routing::patch(room::update_playlist),
        )
        .route(
            "/api/rooms/:room_id/playlists/:playlist_id",
            axum::routing::delete(room::delete_playlist),
        )
        // Room settings reset
        .route("/api/rooms/:room_id/settings/reset", post(room::reset_room_settings))
        // Playback: set current media & speed
        .route("/api/rooms/:room_id/playback/current", post(room::set_current_media))
        .route("/api/rooms/:room_id/playback/speed", post(room::set_playback_speed))
        .route_layer(axum_middleware::from_fn_with_state(
            state.clone(),
            middleware::write_rate_limit,
        ));

    // Email routes — strict rate limiting (5 req/min) to prevent email spam and token brute-force
    let email_routes = email_verification::create_email_router()
        .route_layer(axum_middleware::from_fn_with_state(
            state.clone(),
            middleware::auth_rate_limit,
        ));

    let router = Router::new()
        // Health check endpoints (for monitoring probes)
        .merge(health::create_health_router())
        // Public endpoints (no authentication required)
        .merge(public::create_public_router())
        // Email verification and password reset (rate-limited)
        .merge(email_routes)
        // Publish key routes — strict rate limiting (5 req/min, same as auth)
        .merge(
            publish_key::create_publish_key_router()
                .route_layer(axum_middleware::from_fn_with_state(
                    state.clone(),
                    middleware::auth_rate_limit,
                ))
        )
        // Notification routes
        .merge(notifications::create_notification_router())
        // Rate-limited route groups
        .merge(auth_routes)
        .merge(media_routes)
        .merge(write_routes)
        // OAuth2 read-only routes (no rate limit)
        .route(
            "/api/oauth2/:provider/authorize",
            get(oauth2::get_authorize_url),
        )
        .route("/api/oauth2/providers", get(oauth2::list_providers))
        // User read routes
        .route("/api/user", get(user::get_me))
        .route("/api/user/rooms", get(user::get_joined_rooms))
        .route("/api/user/rooms/created", get(user::list_created_rooms))
        // Room discovery routes (public)
        .route("/api/rooms", get(room::list_or_get_rooms))
        .route("/api/rooms/hot", get(room::get_hot_rooms))
        .route("/api/rooms/:room_id/check", get(room::check_room))
        // Room read routes
        .route("/api/rooms/:room_id", get(room::get_room))
        .route("/api/rooms/:room_id/settings", get(room::get_room_settings))
        .route("/api/rooms/:room_id/members", get(room::get_room_members))
        // Playlist read routes
        .route("/api/rooms/:room_id/playlists", get(room::list_playlists))
        // Chat history
        .route("/api/rooms/:room_id/chat/history", get(room::get_chat_history))
        // Media read routes
        .route("/api/rooms/:room_id/media", get(room::get_playlist))
        // Movie info endpoint (resolves provider playback)
        .route(
            "/api/rooms/:room_id/movie/:media_id",
            get(room::get_movie_info),
        )
        // Dynamic playlist routes
        .route(
            "/api/rooms/:room_id/playlists/:playlist_id/items",
            get(media::list_playlist_items),
        )
        // Playback control - read
        .route("/api/rooms/:room_id/playback", get(room::get_playback_state))
        // Live streaming routes — rate limited (50 req/min)
        .merge(
            Router::new()
                .nest(
                    "/api/room/movie/live",
                    live::create_live_router(),
                )
                .route_layer(axum_middleware::from_fn_with_state(
                    state.clone(),
                    middleware::streaming_rate_limit,
                ))
        )
        // WebSocket endpoint — rate limited (10 connection attempts/min)
        .merge(
            Router::new()
                .route(
                    "/ws/rooms/:room_id",
                    axum::routing::get(websocket::websocket_handler),
                )
                .route_layer(axum_middleware::from_fn_with_state(
                    state.clone(),
                    middleware::websocket_rate_limit,
                ))
        )
        // WebRTC configuration endpoints
        .route(
            "/api/rooms/:room_id/webrtc/ice-servers",
            get(webrtc::get_ice_servers),
        )
        .route(
            "/api/rooms/:room_id/webrtc/network-quality",
            get(webrtc::get_network_quality),
        )
        // Admin routes (admin/root role required) — rate limited (30 req/min)
        .merge(
            Router::new()
                .nest("/api/admin", admin::create_admin_router())
                .route_layer(axum_middleware::from_fn_with_state(
                    state.clone(),
                    middleware::admin_rate_limit,
                ))
        )
        // Common provider routes (user-facing)
        .nest("/api/provider", provider_common::register_common_routes())
        // Provider-specific HTTP routes
        .nest("/api/providers/bilibili", providers::bilibili::bilibili_routes())
        .nest("/api/providers/alist", providers::alist::alist_routes())
        .nest("/api/providers/emby", providers::emby::emby_routes())
        .nest("/api/providers/direct_url", providers::direct_url::direct_url_routes())
        // OpenAPI/Swagger documentation
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", openapi::ApiDoc::openapi()));

    // Apply layers before state
    // CORS configuration: use allowed origins from config, or permissive in development mode
    let cors = if config.server.development_mode || config.server.cors_allowed_origins.is_empty() {
        // Development mode or no configured origins: allow all (with warning)
        if !config.server.development_mode && config.server.cors_allowed_origins.is_empty() {
            tracing::warn!(
                "CORS: No allowed origins configured, allowing all origins. \
                 Set server.cors_allowed_origins in production for security."
            );
        }
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        // Production: use configured origins only
        let origins: Vec<HeaderValue> = config
            .server
            .cors_allowed_origins
            .iter()
            .filter_map(|origin| origin.parse().ok())
            .collect();
        tracing::info!(
            origins = ?origins,
            "CORS: Configured with {} allowed origin(s)",
            origins.len()
        );
        let x_room_id: HeaderName = "x-room-id".parse().unwrap_or_else(|_| HeaderName::from_static("x-room-id"));
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::OPTIONS])
            .allow_headers([
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
                axum::http::header::ACCEPT,
                x_room_id,
            ])
            .allow_credentials(true)
    };

    let router = router
        // CORS support for cross-origin requests
        .layer(cors)
        // Limit request body size to prevent DoS attacks (10MB for general endpoints)
        .layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(axum_middleware::from_fn(middleware::security_headers_middleware))
        .layer(axum_middleware::from_fn(
            crate::observability::metrics_middleware::metrics_layer,
        ))
        .layer(TraceLayer::new_for_http());

    // Apply state to all routes (must be last)
    router.with_state(state)
}

/// Create the HTTP router from configuration struct
pub fn create_router_from_config(config: RouterConfig) -> axum::Router {
    create_router(
        config.config,
        config.user_service,
        config.room_service,
        config.provider_instance_manager,
        config.user_provider_credential_repository,
        config.alist_provider,
        config.bilibili_provider,
        config.emby_provider,
        config.message_hub,
        config.cluster_manager,
        config.connection_manager,
        config.jwt_service,
        config.redis_publish_tx,
        config.oauth2_service,
        config.settings_service,
        config.settings_registry,
        config.email_service,
        config.publish_key_service,
        config.notification_service,
        config.live_streaming_infrastructure,
        config.sfu_manager,
        config.rate_limiter,
    )
}

