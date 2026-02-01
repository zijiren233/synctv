// Generated protobuf code
pub mod proto {
    #![allow(clippy::all)]
    #![allow(warnings)]

    pub mod client {
        include!("proto/synctv.client.rs");
    }

    pub mod admin {
        include!("proto/synctv.admin.rs");
    }

    pub mod cluster {
        include!("proto/synctv.cluster.rs");
    }
}

pub mod client_service;
pub mod admin_service;
pub mod interceptors;

// Provider gRPC service extensions (decoupled via trait)
pub mod providers;

pub use client_service::ClientServiceImpl;
pub use admin_service::AdminServiceImpl;
// pub use interceptors::{AuthInterceptor, LoggingInterceptor};

use proto::client::client_service_server::ClientServiceServer;
use proto::admin::admin_service_server::AdminServiceServer;
use tonic::transport::Server;
use tonic_reflection::server::Builder as ReflectionBuilder;

use synctv_core::service::{UserService, RoomService, RateLimiter, RateLimitConfig, ContentFilter, ProvidersManager, ProviderInstanceManager};
use synctv_core::repository::UserProviderCredentialRepository;
use synctv_core::Config;
use synctv_cluster::sync::{RoomMessageHub, PublishRequest, ConnectionManager};
use std::sync::Arc;

/// Build and start the gRPC server
pub async fn serve(
    config: &Config,
    user_service: Arc<UserService>,
    room_service: Arc<RoomService>,
    message_hub: Arc<RoomMessageHub>,
    redis_publish_tx: Option<tokio::sync::mpsc::UnboundedSender<PublishRequest>>,
    rate_limiter: RateLimiter,
    rate_limit_config: RateLimitConfig,
    content_filter: ContentFilter,
    connection_manager: ConnectionManager,
    providers_manager: Option<Arc<ProvidersManager>>,
    provider_instance_manager: Arc<ProviderInstanceManager>,
    credential_repository: Arc<UserProviderCredentialRepository>,
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

    // Create service instances
    let user_service_clone = Arc::try_unwrap(user_service_for_client).unwrap_or_else(|arc| (*arc).clone());
    let room_service_clone = Arc::try_unwrap(room_service_for_client).unwrap_or_else(|arc| (*arc).clone());

    let client_service = ClientServiceImpl::new(
        user_service_clone,
        room_service_clone,
        (*message_hub).clone(),
        redis_publish_tx,
        rate_limiter,
        rate_limit_config,
        content_filter,
        connection_manager,
    );

    let admin_service = AdminServiceImpl::new(
        Arc::try_unwrap(user_service_for_admin).unwrap_or_else(|arc| (*arc).clone()),
        Arc::try_unwrap(room_service_for_admin).unwrap_or_else(|arc| (*arc).clone()),
        provider_instance_manager,
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

    // Build router
    let mut router = server_builder
        .add_service(ClientServiceServer::new(client_service))
        .add_service(AdminServiceServer::new(admin_service));

    if let Some(reflection) = reflection_service {
        router = router.add_service(reflection);
    }

    // Register provider gRPC services via registry pattern
    // Each provider module self-registers, achieving complete decoupling!
    if let Some(providers_mgr) = providers_manager {
        tracing::info!("Initializing provider gRPC service modules");

        // Create AppState for provider extensions
        let app_state = Arc::new(crate::http::AppState {
            user_service: user_service_for_provider,
            room_service: room_service_for_provider,
            provider_instance_manager: providers_mgr.instance_manager().clone(),
            providers_manager: providers_mgr,
            credential_repository,
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
