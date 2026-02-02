// Generated protobuf code
// Note: All proto files are now in synctv-proto crate
pub mod proto {
    #![allow(clippy::all)]
    #![allow(warnings)]

    // Only cluster.proto is internal (from synctv-cluster)
    pub mod cluster {
        include!("proto/synctv.cluster.rs");
    }
}

pub mod admin_service;
pub mod client_service;
pub mod interceptors;
pub mod message_handler;

// Provider gRPC service extensions (decoupled via trait)
pub mod providers;

pub use admin_service::AdminServiceImpl;
pub use client_service::ClientServiceImpl;
pub use interceptors::{AuthInterceptor, LoggingInterceptor, ValidationInterceptor, TimeoutInterceptor};
pub use message_handler::{MessageHandler, cluster_event_to_server_message};

// Use synctv_proto for all server traits and message types (single source of truth)
use synctv_proto::admin_service_server::AdminServiceServer;
use synctv_proto::client::{
    auth_service_server::AuthServiceServer, email_service_server::EmailServiceServer,
    media_service_server::MediaServiceServer, public_service_server::PublicServiceServer,
    room_service_server::RoomServiceServer, user_service_server::UserServiceServer,
};
use tonic::transport::Server;
use tonic_reflection::server::Builder as ReflectionBuilder;

use std::sync::Arc;
use synctv_cluster::sync::{ConnectionManager, PublishRequest, RoomMessageHub};
use synctv_core::service::auth::JwtService;
use synctv_core::service::{
    ContentFilter, ProviderInstanceManager, ProvidersManager, RateLimitConfig, RateLimiter,
    RoomService as CoreRoomService, UserService as CoreUserService, SettingsService,
    SettingsRegistry, EmailService, EmailTokenService,
};
use synctv_core::Config;

/// Build and start the gRPC server
pub async fn serve(
    config: &Config,
    jwt_service: JwtService,
    user_service: Arc<CoreUserService>,
    room_service: Arc<CoreRoomService>,
    message_hub: Arc<RoomMessageHub>,
    redis_publish_tx: Option<tokio::sync::mpsc::UnboundedSender<PublishRequest>>,
    rate_limiter: RateLimiter,
    rate_limit_config: RateLimitConfig,
    content_filter: ContentFilter,
    connection_manager: ConnectionManager,
    providers_manager: Option<Arc<ProvidersManager>>,
    provider_instance_manager: Arc<ProviderInstanceManager>,
    provider_instance_repository: Arc<synctv_core::repository::ProviderInstanceRepository>,
    user_provider_credential_repository: Arc<synctv_core::repository::UserProviderCredentialRepository>,
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

    let client_service = ClientServiceImpl::new(
        user_service_clone,
        room_service_clone,
        (*message_hub).clone(),
        redis_publish_tx.clone(),
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

    // Add reflection if enabled
    let reflection_service = if config.server.enable_reflection {
        // Load file descriptor set from generated binary
        let descriptor_bytes = include_bytes!("proto/descriptor.bin");
        let reflection = ReflectionBuilder::configure()
            .register_encoded_file_descriptor_set(descriptor_bytes.as_ref())
            .build_v1()
            .map_err(|e| anyhow::anyhow!("Failed to build reflection service: {}", e))?;

        tracing::info!("gRPC reflection enabled");
        Some(reflection)
    } else {
        None
    };

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

    if let Some(reflection) = reflection_service {
        router = router.add_service(reflection);
    }

    // Register provider gRPC services via registry pattern
    // Each provider module self-registers, achieving complete decoupling!
    if let Some(providers_mgr) = providers_manager {
        tracing::info!("Initializing provider gRPC service modules");

        // Create AppState for provider extensions with all required dependencies
        let app_state = Arc::new(crate::http::AppState {
            user_service: user_service_for_provider,
            room_service: room_service_for_provider,
            provider_instance_manager: providers_mgr.instance_manager().clone(),
            provider_instance_repository: provider_instance_repository.clone(),
            user_provider_credential_repository: user_provider_credential_repository.clone(),
            message_hub: message_hub.clone(),
            jwt_service: jwt_service_for_provider,
            redis_publish_tx: redis_publish_tx.clone(),
            oauth2_service: None, // Not used in gRPC provider services
            settings_service: Some(settings_service.clone()),
            settings_registry: None, // Not used in gRPC provider services
            email_service: None, // Not used in gRPC provider services
            publish_key_service: None, // Not used in gRPC provider services
            notification_service: None, // Not used in gRPC provider services
        });

        // Initialize all provider modules (triggers self-registration)
        providers::bilibili::init();
        providers::alist::init();
        providers::emby::init();

        // Build services from registry (no knowledge of specific types!)
        router = providers::build_provider_services(app_state, router);
    }

    // Start server
    router
        .serve(addr)
        .await
        .map_err(|e| anyhow::anyhow!("gRPC server error: {}", e))?;

    Ok(())
}
