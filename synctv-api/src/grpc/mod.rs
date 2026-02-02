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
    AuthInterceptor, LoggingInterceptor, TimeoutInterceptor, ValidationInterceptor,
};

// Use synctv_proto for all server traits and message types (single source of truth)
use crate::proto::admin_service_server::AdminServiceServer;
use crate::proto::client::{
    auth_service_server::AuthServiceServer, email_service_server::EmailServiceServer,
    media_service_server::MediaServiceServer, public_service_server::PublicServiceServer,
    room_service_server::RoomServiceServer, user_service_server::UserServiceServer,
};
use tonic::transport::Server;
use tonic_reflection::server::Builder as ReflectionBuilder;

use std::sync::Arc;
use synctv_cluster::sync::{ClusterManager, ConnectionManager, PublishRequest, RoomMessageHub};
use synctv_core::provider::{AlistProvider, BilibiliProvider, EmbyProvider};
use synctv_core::service::auth::JwtService;
use synctv_core::service::{
    ContentFilter, EmailService, EmailTokenService, ProviderInstanceManager, ProvidersManager,
    RateLimitConfig, RateLimiter, RoomService as CoreRoomService, SettingsRegistry,
    SettingsService, UserService as CoreUserService,
};
use synctv_core::Config;

/// Build and start the gRPC server
pub async fn serve(
    config: &Config,
    jwt_service: JwtService,
    user_service: Arc<CoreUserService>,
    room_service: Arc<CoreRoomService>,
    cluster_manager: Arc<ClusterManager>,
    redis_publish_tx: Option<tokio::sync::mpsc::UnboundedSender<PublishRequest>>,
    rate_limiter: RateLimiter,
    rate_limit_config: RateLimitConfig,
    content_filter: ContentFilter,
    connection_manager: ConnectionManager,
    providers_manager: Option<Arc<ProvidersManager>>,
    provider_instance_manager: Arc<ProviderInstanceManager>,
    provider_instance_repository: Arc<synctv_core::repository::ProviderInstanceRepository>,
    user_provider_credential_repository: Arc<
        synctv_core::repository::UserProviderCredentialRepository,
    >,
    settings_service: Arc<SettingsService>,
    settings_registry: Option<Arc<SettingsRegistry>>,
    email_service: Option<Arc<EmailService>>,
    email_token_service: Option<Arc<EmailTokenService>>,
) -> anyhow::Result<()> {
    let addr = config.grpc_address().parse()?;

    tracing::info!("Starting gRPC server on {}", addr);

    // Clone services for all uses before unwrapping
    let user_service_for_client = user_service.clone();
    let user_service_for_admin = user_service.clone();
    let user_service_for_provider = user_service.clone();

    let room_service_for_client = room_service.clone();
    let room_service_for_admin = room_service.clone();
    let room_service_for_provider = room_service.clone();

    let jwt_service_for_provider = jwt_service.clone();

    // Create service instances
    let user_service_clone =
        Arc::try_unwrap(user_service_for_client).unwrap_or_else(|arc| (*arc).clone());
    let room_service_clone =
        Arc::try_unwrap(room_service_for_client).unwrap_or_else(|arc| (*arc).clone());

    // Extract message_hub reference before moving cluster_manager
    let message_hub_from_cluster = cluster_manager.message_hub().clone();

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
    );

    let admin_service = AdminServiceImpl::new(
        Arc::try_unwrap(user_service_for_admin).unwrap_or_else(|arc| (*arc).clone()),
        Arc::try_unwrap(room_service_for_admin).unwrap_or_else(|arc| (*arc).clone()),
        provider_instance_manager,
        settings_service.clone(),
        settings_registry,
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
            provider_instance_manager_for_provider.clone(),
        ));

        let app_state = Arc::new(crate::http::AppState {
            user_service: user_service_for_provider.clone(),
            room_service: room_service_for_provider.clone(),
            provider_instance_manager: _providers_mgr.instance_manager().clone(),
            user_provider_credential_repository: user_provider_credential_repository.clone(),
            alist_provider,
            bilibili_provider,
            emby_provider,
            cluster_manager: None, // gRPC doesn't expose cluster_manager to HTTP
            message_hub: message_hub_from_cluster,
            jwt_service: jwt_service_for_provider,
            redis_publish_tx: redis_publish_tx.clone(),
            oauth2_service: None,
            settings_service: Some(settings_service.clone()),
            settings_registry: None,
            email_service: None,
            publish_key_service: None,
            notification_service: None,
            client_api: Arc::new(crate::impls::ClientApiImpl::new(
                user_service_for_provider,
                room_service_for_provider,
            )),
            admin_api: None,
        });

        // Manually register provider gRPC services
        use synctv_proto::providers::alist::alist_provider_service_server::AlistProviderServiceServer;
        use synctv_proto::providers::bilibili::bilibili_provider_service_server::BilibiliProviderServiceServer;
        use synctv_proto::providers::emby::emby_provider_service_server::EmbyProviderServiceServer;

        router = router.add_service(AlistProviderServiceServer::new(
            providers::alist::AlistProviderGrpcService::new(app_state.clone())
        ));
        router = router.add_service(BilibiliProviderServiceServer::new(
            providers::bilibili::BilibiliProviderGrpcService::new(app_state.clone())
        ));
        router = router.add_service(EmbyProviderServiceServer::new(
            providers::emby::EmbyProviderGrpcService::new(app_state)
        ));
    }

    // Start server
    router
        .serve(addr)
        .await
        .map_err(|e| anyhow::anyhow!("gRPC server error: {}", e))?;

    Ok(())
}
