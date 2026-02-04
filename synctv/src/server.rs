//! Server lifecycle management
//!
//! Manages the startup and shutdown of all server components:
//! - gRPC API server
//! - HTTP/REST server
//! - RTMP streaming server

use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::{error, info};

use synctv_core::{
    service::{RoomService, UserService},
    repository::UserProviderCredentialRepository,
    provider::{AlistProvider, BilibiliProvider, EmbyProvider},
    Config,
};
use synctv_stream::StreamRegistry;

/// Streaming server state
pub struct StreamingState {
    pub registry: StreamRegistry,
    pub pull_manager: Arc<synctv_stream::streaming::PullStreamManager>,
}

/// Container for shared services
#[derive(Clone)]
pub struct Services {
    pub user_service: Arc<UserService>,
    pub room_service: Arc<RoomService>,
    pub jwt_service: synctv_core::service::JwtService,
    pub message_hub: Arc<synctv_cluster::sync::RoomMessageHub>,
    pub cluster_manager: Option<Arc<synctv_cluster::sync::ClusterManager>>,
    pub redis_publish_tx: Option<tokio::sync::mpsc::UnboundedSender<synctv_cluster::sync::PublishRequest>>,
    pub rate_limiter: synctv_core::service::RateLimiter,
    pub rate_limit_config: synctv_core::service::RateLimitConfig,
    pub content_filter: synctv_core::service::ContentFilter,
    pub connection_manager: synctv_cluster::sync::ConnectionManager,
    pub providers_manager: Arc<synctv_core::service::ProvidersManager>,
    pub provider_instance_manager: Arc<synctv_core::service::ProviderInstanceManager>,
    pub provider_instance_repository: Arc<synctv_core::repository::ProviderInstanceRepository>,
    pub user_provider_credential_repository: Arc<UserProviderCredentialRepository>,
    pub alist_provider: Arc<AlistProvider>,
    pub bilibili_provider: Arc<BilibiliProvider>,
    pub emby_provider: Arc<EmbyProvider>,
    pub oauth2_service: Option<Arc<synctv_core::service::OAuth2Service>>,
    pub settings_service: Arc<synctv_core::service::SettingsService>,
    pub settings_registry: Arc<synctv_core::service::SettingsRegistry>,
    pub email_service: Option<Arc<synctv_core::service::EmailService>>,
    pub email_token_service: Option<Arc<synctv_core::service::EmailTokenService>>,
    pub publish_key_service: Arc<synctv_core::service::PublishKeyService>,
    pub notification_service: Option<Arc<synctv_core::service::UserNotificationService>>,
    pub live_streaming_infrastructure: Option<Arc<synctv_stream::api::LiveStreamingInfrastructure>>,
}

/// `SyncTV` server - manages all server components
pub struct SyncTvServer {
    config: Config,
    services: Services,
    streaming_state: Option<StreamingState>,
    grpc_handle: Option<JoinHandle<()>>,
    http_handle: Option<JoinHandle<()>>,
}

impl Services {
    /// Get the message hub for HTTP server use
    pub fn message_hub(&self) -> Arc<synctv_cluster::sync::RoomMessageHub> {
        self.message_hub.clone()
    }
}

impl SyncTvServer {
    /// Create a new server instance
    pub const fn new(
        config: Config,
        services: Services,
        streaming_state: Option<StreamingState>,
    ) -> Self {
        Self {
            config,
            services,
            streaming_state,
            grpc_handle: None,
            http_handle: None,
        }
    }

    /// Start all servers
    pub async fn start(mut self) -> anyhow::Result<()> {
        info!("Starting SyncTV server...");

        // Start gRPC server
        let grpc_handle = self.start_grpc_server().await?;
        self.grpc_handle = Some(grpc_handle);

        // Start HTTP server
        let http_handle = self.start_http_server().await?;
        self.http_handle = Some(http_handle);

        info!("All servers started successfully");

        // Wait for both servers
        let grpc_handle = self.grpc_handle.unwrap();
        let http_handle = self.http_handle.unwrap();

        tokio::select! {
            _ = grpc_handle => {
                error!("gRPC server stopped unexpectedly");
            }
            _ = http_handle => {
                error!("HTTP server stopped unexpectedly");
            }
        }

        Ok(())
    }

    /// Start gRPC server
    async fn start_grpc_server(&self) -> anyhow::Result<JoinHandle<()>> {
        let config = self.config.clone();
        let user_service = self.services.user_service.clone();
        let room_service = self.services.room_service.clone();
        let jwt_service = self.services.jwt_service.clone();
        let cluster_manager = self.services.cluster_manager.clone()
            .ok_or_else(|| anyhow::anyhow!("ClusterManager is required for gRPC server"))?;
        let redis_publish_tx = self.services.redis_publish_tx.clone();
        let rate_limiter = self.services.rate_limiter.clone();
        let rate_limit_config = self.services.rate_limit_config.clone();
        let content_filter = self.services.content_filter.clone();
        let connection_manager = self.services.connection_manager.clone();
        let providers_manager = self.services.providers_manager.clone();
        let provider_instance_manager = self.services.provider_instance_manager.clone();
        let provider_instance_repository = self.services.provider_instance_repository.clone();
        let user_provider_credential_repository = self.services.user_provider_credential_repository.clone();
        let settings_service = self.services.settings_service.clone();
        let settings_registry = self.services.settings_registry.clone();
        let email_service = self.services.email_service.clone();
        let email_token_service = self.services.email_token_service.clone();

        let handle = tokio::spawn(async move {
            info!("Starting gRPC server on {}...", config.grpc_address());
            if let Err(e) = synctv_api::grpc::serve(
                &config,
                jwt_service,
                user_service,
                room_service,
                cluster_manager,
                redis_publish_tx,
                rate_limiter,
                rate_limit_config,
                content_filter,
                connection_manager,
                Some(providers_manager),
                provider_instance_manager,
                provider_instance_repository,
                user_provider_credential_repository,
                settings_service,
                Some(settings_registry),
                email_service,
                email_token_service,
            )
            .await
            {
                error!("gRPC server error: {}", e);
            }
        });

        Ok(handle)
    }

    /// Start HTTP server
    async fn start_http_server(&self) -> anyhow::Result<JoinHandle<()>> {
        let http_address = self.config.http_address();
        let user_service = self.services.user_service.clone();
        let room_service = self.services.room_service.clone();
        let provider_instance_manager = self.services.provider_instance_manager.clone();
        let provider_instance_repository = self.services.provider_instance_repository.clone();
        let user_provider_credential_repository = self.services.user_provider_credential_repository.clone();
        let message_hub = self.services.message_hub();
        let cluster_manager = self.services.cluster_manager.clone();
        let jwt_service = self.services.jwt_service.clone();
        let redis_publish_tx = self.services.redis_publish_tx.clone();
        let oauth2_service = self.services.oauth2_service.clone();
        let settings_service = self.services.settings_service.clone();
        let settings_registry = self.services.settings_registry.clone();
        let email_service = self.services.email_service.clone();
        let publish_key_service = self.services.publish_key_service.clone();
        let notification_service = self.services.notification_service.clone();
        let connection_manager = self.services.connection_manager.clone();

        let live_streaming_infrastructure = self.services.live_streaming_infrastructure.clone();

        let http_router = synctv_api::http::create_router(
            user_service,
            room_service,
            provider_instance_manager,
            provider_instance_repository,
            user_provider_credential_repository,
            self.services.alist_provider.clone(),
            self.services.bilibili_provider.clone(),
            self.services.emby_provider.clone(),
            message_hub,
            cluster_manager,
            Arc::new(connection_manager),
            jwt_service,
            redis_publish_tx,
            oauth2_service,
            Some(settings_service),
            Some(settings_registry),
            email_service,
            Some(publish_key_service),
            notification_service,
            live_streaming_infrastructure,
        );

        let handle = tokio::spawn(async move {
            let http_addr: std::net::SocketAddr = http_address.parse().expect("Invalid HTTP address");

            let listener = tokio::net::TcpListener::bind(http_addr)
                .await
                .expect("Failed to bind HTTP address");

            info!("HTTP server listening on {}", http_addr);

            if let Err(e) = axum::serve(listener, http_router).await {
                error!("HTTP server error: {}", e);
            }
        });

        Ok(handle)
    }
}
