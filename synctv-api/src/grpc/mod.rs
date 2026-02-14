// Re-export proto types from synctv-proto
pub use synctv_proto::{client, admin};

// Re-export cluster proto from synctv-cluster (internal)
pub use synctv_cluster::grpc::synctv::cluster;

pub mod admin_service;
pub mod client_service;
pub mod interceptors;

// Provider gRPC services (local implementations)
// Provider-specific gRPC services are registered from provider instances
pub mod providers;

pub use admin_service::AdminServiceImpl;
pub use client_service::ClientServiceImpl;
pub use interceptors::{
    AuthInterceptor, ClusterAuthInterceptor, LoggingInterceptor, TimeoutInterceptor,
    ValidationInterceptor,
};

// Use synctv_proto for all server traits and message types (single source of truth)
use crate::proto::admin_service_server::AdminServiceServer;
use crate::proto::client::{
    auth_service_server::AuthServiceServer, email_service_server::EmailServiceServer,
    media_service_server::MediaServiceServer, public_service_server::PublicServiceServer,
    room_service_server::RoomServiceServer, user_service_server::UserServiceServer,
};
use tonic::transport::Server;

use std::sync::Arc;
use synctv_cluster::sync::{ClusterManager, ConnectionManager, PublishRequest};
use synctv_core::provider::{AlistProvider, BilibiliProvider, EmbyProvider};
use synctv_core::service::auth::JwtService;
use synctv_core::service::{
    ContentFilter, EmailService, EmailTokenService, RemoteProviderManager, ProvidersManager,
    RateLimitConfig, RateLimiter, RoomService as CoreRoomService, SettingsRegistry,
    SettingsService, UserService as CoreUserService,
};
use synctv_core::Config;

/// Configuration for the gRPC server
#[derive(Clone)]
pub struct GrpcServerConfig<'a> {
    pub config: &'a Config,
    pub jwt_service: JwtService,
    pub user_service: Arc<CoreUserService>,
    pub room_service: Arc<CoreRoomService>,
    pub cluster_manager: Arc<ClusterManager>,
    pub redis_publish_tx: Option<tokio::sync::mpsc::Sender<PublishRequest>>,
    pub rate_limiter: RateLimiter,
    pub rate_limit_config: RateLimitConfig,
    pub content_filter: ContentFilter,
    pub connection_manager: ConnectionManager,
    pub providers_manager: Option<Arc<ProvidersManager>>,
    pub provider_instance_manager: Arc<RemoteProviderManager>,
    pub provider_instance_repository: Arc<synctv_core::repository::ProviderInstanceRepository>,
    pub user_provider_credential_repository: Arc<synctv_core::repository::UserProviderCredentialRepository>,
    pub settings_service: Arc<SettingsService>,
    pub settings_registry: Option<Arc<SettingsRegistry>>,
    pub email_service: Option<Arc<EmailService>>,
    pub email_token_service: Option<Arc<EmailTokenService>>,
    pub sfu_manager: Option<Arc<synctv_sfu::SfuManager>>,
    pub live_streaming_infrastructure: Option<Arc<synctv_livestream::api::LiveStreamingInfrastructure>>,
    pub publish_key_service: Option<Arc<synctv_core::service::PublishKeyService>>,
    pub shutdown_rx: Option<tokio::sync::watch::Receiver<bool>>,
}

/// Build and start the gRPC server
#[allow(clippy::too_many_arguments, clippy::result_large_err)]
pub async fn serve(
    config: &Config,
    jwt_service: JwtService,
    user_service: Arc<CoreUserService>,
    room_service: Arc<CoreRoomService>,
    cluster_manager: Arc<ClusterManager>,
    redis_publish_tx: Option<tokio::sync::mpsc::Sender<PublishRequest>>,
    rate_limiter: RateLimiter,
    rate_limit_config: RateLimitConfig,
    content_filter: ContentFilter,
    connection_manager: ConnectionManager,
    providers_manager: Option<Arc<ProvidersManager>>,
    provider_instance_manager: Arc<RemoteProviderManager>,
    _provider_instance_repository: Arc<synctv_core::repository::ProviderInstanceRepository>,
    user_provider_credential_repository: Arc<
        synctv_core::repository::UserProviderCredentialRepository,
    >,
    settings_service: Arc<SettingsService>,
    settings_registry: Option<Arc<SettingsRegistry>>,
    email_service: Option<Arc<EmailService>>,
    email_token_service: Option<Arc<EmailTokenService>>,
    sfu_manager: Option<Arc<synctv_sfu::SfuManager>>,
    live_streaming_infrastructure: Option<Arc<synctv_livestream::api::LiveStreamingInfrastructure>>,
    publish_key_service: Option<Arc<synctv_core::service::PublishKeyService>>,
    shutdown_rx: Option<tokio::sync::watch::Receiver<bool>>,
) -> anyhow::Result<()> {
    let addr = config.grpc_address().parse()?;

    tracing::info!("Starting gRPC server on {}", addr);

    // Clone services for all uses before unwrapping
    let user_service_for_client = user_service.clone();
    let user_service_for_admin = user_service.clone();
    let user_service_for_provider = user_service.clone();

    let room_service_for_client = room_service.clone();
    let room_service_for_provider = room_service.clone();

    let jwt_service_for_provider = jwt_service.clone();

    // Create service instances
    let user_service_clone =
        Arc::try_unwrap(user_service_for_client).unwrap_or_else(|arc| (*arc).clone());
    let room_service_clone =
        Arc::try_unwrap(room_service_for_client).unwrap_or_else(|arc| (*arc).clone());

    // Extract message_hub reference before moving cluster_manager
    let message_hub_from_cluster = cluster_manager.message_hub().clone();

    // Clone connection_manager for later use
    let connection_manager_for_provider = connection_manager.clone();

    let email_service_for_admin = email_service.clone();
    let providers_manager_for_client = providers_manager.clone();
    let rate_limiter_for_provider = rate_limiter.clone();

    // Build the shared ClientApiImpl for gRPC handlers
    let client_api = Arc::new(crate::impls::ClientApiImpl::new(
        user_service.clone(),
        room_service.clone(),
        Arc::new(connection_manager.clone()),
        Arc::new(config.clone()),
        sfu_manager.clone(),
        publish_key_service,
        jwt_service.clone(),
        live_streaming_infrastructure.clone(),
        providers_manager_for_client.clone(),
        settings_registry.clone(),
    ).with_redis_publish_tx(redis_publish_tx.clone()));

    let client_service = ClientServiceImpl::new(
        user_service_clone,
        room_service_clone,
        cluster_manager,
        rate_limiter,
        rate_limit_config,
        content_filter,
        connection_manager,
        email_service,
        email_token_service,
        settings_registry.clone(),
        providers_manager_for_client,
        Arc::new(config.clone()),
        sfu_manager.clone(),
        client_api.clone(),
    );

    // Build the shared AdminApiImpl for gRPC handlers (same impls layer used by HTTP)
    // AdminApiImpl requires EmailService; if not configured, create with None config
    // so send_test_email fails gracefully.
    let email_svc_for_admin_api = email_service_for_admin
        .unwrap_or_else(|| Arc::new(EmailService::new(None).expect("EmailService::new(None) should not fail")));

    let admin_api = Arc::new(crate::impls::AdminApiImpl::new(
        room_service.clone(),
        user_service_for_admin.clone(),
        settings_service.clone(),
        settings_registry.clone(),
        email_svc_for_admin_api,
        Arc::new(connection_manager_for_provider.clone()),
        provider_instance_manager,
        live_streaming_infrastructure,
        redis_publish_tx.clone(),
    ));

    let admin_service = AdminServiceImpl::new(
        user_service_for_admin,
        admin_api,
    );

    // Create server builder
    let mut server_builder = Server::builder();

    // Note: gRPC reflection is disabled - proto definitions are in synctv-proto crate
    // To enable reflection in the future, we would need to re-export descriptor from synctv-proto

    // Create auth interceptor for authenticated services
    let auth_interceptor = AuthInterceptor::new(jwt_service);

    // Clone interceptors for different services
    let user_interceptor = auth_interceptor.clone();
    let admin_interceptor = auth_interceptor.clone();
    let room_interceptor1 = auth_interceptor.clone();
    let room_interceptor2 = auth_interceptor.clone();

    // Build router - register all client services with appropriate interceptors
    let client_service_clone1 = client_service.clone();
    let client_service_clone2 = client_service.clone();
    let client_service_clone3 = client_service.clone();
    let client_service_clone4 = client_service.clone();
    let client_service_clone5 = client_service.clone();

    let mut router = server_builder
        // AuthService - no authentication required (public: register, login, refresh_token)
        .add_service(AuthServiceServer::new(client_service))
        // UserService - requires JWT authentication (inject UserContext)
        .add_service(UserServiceServer::with_interceptor(
            client_service_clone1,
            move |req| user_interceptor.inject_user(req),
        ))
        // RoomService - requires JWT + room_id (inject RoomContext)
        .add_service(RoomServiceServer::with_interceptor(
            client_service_clone2,
            move |req| room_interceptor1.inject_room(req),
        ))
        // MediaService - requires JWT + room_id (inject RoomContext)
        .add_service(MediaServiceServer::with_interceptor(
            client_service_clone3,
            move |req| room_interceptor2.inject_room(req),
        ))
        // PublicService - no authentication required (public room discovery)
        .add_service(PublicServiceServer::new(client_service_clone4))
        // EmailService - no authentication required (send codes, confirm with token)
        .add_service(EmailServiceServer::new(client_service_clone5))
        // AdminService - requires JWT authentication (inject UserContext)
        .add_service(AdminServiceServer::with_interceptor(
            admin_service,
            move |req| admin_interceptor.inject_user(req),
        ));

    // Register provider gRPC services
    if let Some(_providers_mgr) = providers_manager {
        tracing::info!("Registering provider gRPC services");

        // Create AppState for provider gRPC services
        let provider_instance_manager_for_provider = _providers_mgr.instance_manager().clone();
        let alist_provider = Arc::new(AlistProvider::new(
            provider_instance_manager_for_provider.clone(),
        ));
        let bilibili_provider = Arc::new(BilibiliProvider::new(
            provider_instance_manager_for_provider.clone(),
        ));
        let emby_provider = Arc::new(EmbyProvider::new(
            provider_instance_manager_for_provider,
        ));

        let app_state = Arc::new(crate::http::AppState {
            config: Arc::new(config.clone()),
            user_service: user_service_for_provider.clone(),
            room_service: room_service_for_provider.clone(),
            provider_instance_manager: _providers_mgr.instance_manager().clone(),
            user_provider_credential_repository: user_provider_credential_repository.clone(),
            alist_provider,
            bilibili_provider,
            emby_provider,
            cluster_manager: None, // gRPC doesn't expose cluster_manager to HTTP
            connection_manager: Arc::new(connection_manager_for_provider.clone()),
            message_hub: message_hub_from_cluster,
            jwt_service: jwt_service_for_provider.clone(),
            redis_publish_tx: redis_publish_tx.clone(),
            oauth2_service: None,
            settings_service: Some(settings_service.clone()),
            settings_registry: None,
            email_service: None,
            publish_key_service: None,
            notification_service: None,
            live_streaming_infrastructure: None,
            rate_limiter: rate_limiter_for_provider,
            ws_ticket_service: None, // WebSocket ticket is HTTP-only
            client_api: Arc::new(crate::impls::ClientApiImpl::new(
                user_service_for_provider,
                room_service_for_provider,
                Arc::new(connection_manager_for_provider.clone()),
                Arc::new(config.clone()),
                sfu_manager,
                None, // No publish_key_service for provider gRPC
                jwt_service_for_provider.clone(),
                None, // No live_streaming_infrastructure for provider gRPC
                None, // No providers_manager for provider gRPC
                None, // No settings_registry for provider gRPC
            ).with_redis_publish_tx(redis_publish_tx.clone())),
            admin_api: None,
        });

        // Register provider gRPC services with auth interceptor
        use synctv_proto::providers::alist::alist_provider_service_server::AlistProviderServiceServer;
        use synctv_proto::providers::bilibili::bilibili_provider_service_server::BilibiliProviderServiceServer;
        use synctv_proto::providers::emby::emby_provider_service_server::EmbyProviderServiceServer;

        let provider_interceptor1 = auth_interceptor.clone();
        let provider_interceptor2 = auth_interceptor.clone();
        let provider_interceptor3 = auth_interceptor.clone();

        router = router.add_service(AlistProviderServiceServer::with_interceptor(
            providers::alist::AlistProviderGrpcService::new(app_state.clone()),
            move |req| provider_interceptor1.inject_user(req),
        ));
        router = router.add_service(BilibiliProviderServiceServer::with_interceptor(
            providers::bilibili::BilibiliProviderGrpcService::new(app_state.clone()),
            move |req| provider_interceptor2.inject_user(req),
        ));
        router = router.add_service(EmbyProviderServiceServer::with_interceptor(
            providers::emby::EmbyProviderGrpcService::new(app_state),
            move |req| provider_interceptor3.inject_user(req),
        ));
    }

    // Register cluster gRPC service (requires cluster_secret to be configured)
    if !config.server.cluster_secret.is_empty() {
        let redis_url = if config.redis.url.is_empty() {
            None
        } else {
            Some(config.redis.url.clone())
        };
        match synctv_cluster::discovery::NodeRegistry::new(redis_url, "self".to_string(), 30) {
            Ok(node_registry) => {
                let cluster_server = synctv_cluster::grpc::ClusterServer::new(
                    std::sync::Arc::new(node_registry),
                    "self".to_string(),
                ).with_connection_manager(
                    std::sync::Arc::new(connection_manager_for_provider.clone()),
                );
                let cluster_interceptor = ClusterAuthInterceptor::new(config.server.cluster_secret.clone());
                router = router.add_service(
                    synctv_cluster::grpc::ClusterServiceServer::with_interceptor(
                        cluster_server,
                        move |req| cluster_interceptor.validate(req),
                    ),
                );
                tracing::info!("Cluster gRPC service registered with shared-secret auth");
            }
            Err(e) => {
                tracing::warn!("Failed to create NodeRegistry for cluster gRPC: {e}");
            }
        }
    }

    // Start server with graceful shutdown support
    router
        .serve_with_shutdown(addr, async move {
            if let Some(mut rx) = shutdown_rx {
                // Use centralized shutdown signal from the server
                let _ = rx.changed().await;
            } else {
                // Fallback: listen for Ctrl+C
                tokio::signal::ctrl_c().await.ok();
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("gRPC server error: {e}"))?;

    Ok(())
}

/// Build and start the gRPC server from configuration struct
pub async fn serve_from_config(config: GrpcServerConfig<'_>) -> anyhow::Result<()> {
    serve(
        config.config,
        config.jwt_service,
        config.user_service,
        config.room_service,
        config.cluster_manager,
        config.redis_publish_tx,
        config.rate_limiter,
        config.rate_limit_config,
        config.content_filter,
        config.connection_manager,
        config.providers_manager,
        config.provider_instance_manager,
        config.provider_instance_repository,
        config.user_provider_credential_repository,
        config.settings_service,
        config.settings_registry,
        config.email_service,
        config.email_token_service,
        config.sfu_manager,
        config.live_streaming_infrastructure,
        config.publish_key_service,
        config.shutdown_rx,
    )
    .await
}
