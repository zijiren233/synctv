// Generated protobuf code
pub mod proto {
    #![allow(clippy::all)]
    #![allow(warnings)]

    pub mod client {
        include!("proto/synctv.client.rs");
    }

    pub mod cluster {
        include!("proto/synctv.cluster.rs");
    }
}

pub mod client_service;
pub mod interceptors;

pub use client_service::ClientServiceImpl;
// pub use interceptors::{AuthInterceptor, LoggingInterceptor};

use proto::client::client_service_server::ClientServiceServer;
use tonic::transport::Server;
use tonic_reflection::server::Builder as ReflectionBuilder;

use synctv_core::service::{UserService, RoomService, RateLimiter, RateLimitConfig, ContentFilter};
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
) -> anyhow::Result<()> {
    let addr = config.grpc_address().parse()?;

    tracing::info!("Starting gRPC server on {}", addr);

    // Create service instance
    let client_service = ClientServiceImpl::new(
        Arc::try_unwrap(user_service).unwrap_or_else(|arc| (*arc).clone()),
        Arc::try_unwrap(room_service).unwrap_or_else(|arc| (*arc).clone()),
        (*message_hub).clone(),
        redis_publish_tx,
        rate_limiter,
        rate_limit_config,
        content_filter,
        connection_manager,
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
        .add_service(ClientServiceServer::new(client_service));

    if let Some(reflection) = reflection_service {
        router = router.add_service(reflection);
    }

    // Start server
    router
        .serve(addr)
        .await
        .map_err(|e| anyhow::anyhow!("gRPC server error: {}", e))?;

    Ok(())
}
