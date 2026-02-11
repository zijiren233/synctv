//! Server lifecycle management
//!
//! Manages the startup and shutdown of all server components:
//! - gRPC API server
//! - HTTP/REST server
//! - RTMP livestream server

use std::sync::Arc;
use std::time::Duration;
use sqlx::PgPool;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use synctv_core::{
    service::{RoomService, UserService},
    repository::UserProviderCredentialRepository,
    provider::{AlistProvider, BilibiliProvider, EmbyProvider},
    Config,
};
use synctv_cluster::sync::ClusterEvent;
use synctv_livestream::StreamRegistry;

/// Livestream server state (held for health checks and graceful shutdown).
///
/// Fields are not read directly -- ownership keeps the `StreamRegistry` and
/// `PullStreamManager` alive for the lifetime of the server (RAII).
#[allow(dead_code)]
pub struct LivestreamState {
    pub registry: StreamRegistry,
    pub pull_manager: Arc<synctv_livestream::livestream::PullStreamManager>,
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
    pub provider_instance_manager: Arc<synctv_core::service::RemoteProviderManager>,
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
    pub live_streaming_infrastructure: Option<Arc<synctv_livestream::api::LiveStreamingInfrastructure>>,
    pub stun_server: Option<Arc<synctv_core::service::StunServer>>,
    pub turn_server: Option<Arc<synctv_core::service::TurnServer>>,
    pub sfu_manager: Option<Arc<synctv_sfu::SfuManager>>,
}

/// `SyncTV` server - manages all server components
pub struct SyncTvServer {
    config: Config,
    services: Services,
    livestream_state: Option<LivestreamState>,
    pool: PgPool,
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
        livestream_state: Option<LivestreamState>,
        pool: PgPool,
    ) -> Self {
        Self {
            config,
            services,
            livestream_state,
            pool,
            grpc_handle: None,
            http_handle: None,
        }
    }

    /// Start all servers and wait for shutdown signal
    pub async fn start(mut self) -> anyhow::Result<()> {
        info!("Starting SyncTV server...");

        // Create shutdown signal channel
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Log infrastructure state
        if self.livestream_state.is_some() {
            info!("Livestream infrastructure: enabled");
        }
        if self.services.stun_server.is_some() {
            info!("STUN server: enabled");
        }
        if self.services.turn_server.is_some() {
            info!("TURN server: enabled");
        }
        if self.services.sfu_manager.is_some() {
            info!("SFU manager: enabled");
        }

        // Start gRPC server
        let grpc_handle = self.start_grpc_server().await?;
        self.grpc_handle = Some(grpc_handle);

        // Start HTTP server with graceful shutdown
        let http_handle = self.start_http_server(shutdown_rx.clone()).await?;
        self.http_handle = Some(http_handle);

        // Spawn streaming event listener for cluster-wide kicks
        if let (Some(cluster_mgr), Some(infra)) = (&self.services.cluster_manager, &self.services.live_streaming_infrastructure) {
            let mut admin_rx = cluster_mgr.subscribe_admin_events();
            let infra = infra.clone();
            tokio::spawn(async move {
                loop {
                    match admin_rx.recv().await {
                        Ok(event) => {
                            if let ClusterEvent::KickPublisher { ref room_id, ref media_id, ref reason, .. } = event {
                                info!(
                                    room_id = %room_id.as_str(),
                                    media_id = %media_id.as_str(),
                                    reason = %reason,
                                    "Received cluster-wide kick event"
                                );
                                let _ = infra.kick_publisher(room_id.as_str(), media_id.as_str());
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!("Admin event listener lagged by {} events", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            info!("Admin event channel closed, stopping listener");
                            break;
                        }
                    }
                }
            });
            info!("Admin event listener spawned for cluster-wide stream kicks");
        }

        info!("All servers started successfully");

        // Wait for either a server to stop or a shutdown signal
        let grpc_handle = self.grpc_handle.take()
            .ok_or_else(|| anyhow::anyhow!("gRPC server handle missing after startup"))?;
        let http_handle = self.http_handle.take()
            .ok_or_else(|| anyhow::anyhow!("HTTP server handle missing after startup"))?;

        tokio::select! {
            _ = grpc_handle => {
                error!("gRPC server stopped unexpectedly");
            }
            _ = http_handle => {
                error!("HTTP server stopped unexpectedly");
            }
            () = shutdown_signal() => {
                info!("Shutdown signal received, starting graceful shutdown...");
            }
        }

        // Signal all components to shut down
        let _ = shutdown_tx.send(true);

        // Run graceful shutdown
        self.shutdown().await;

        Ok(())
    }

    /// Gracefully shut down all server components
    async fn shutdown(&self) {
        info!("Shutting down SyncTV server...");

        // 1. Wait for active connections to drain (with timeout)
        let drain_timeout = Duration::from_secs(30);
        let drain_poll_interval = Duration::from_millis(500);
        let active = self.services.connection_manager.connection_count();
        if active > 0 {
            info!(
                "Waiting up to {}s for {} active connection(s) to drain...",
                drain_timeout.as_secs(),
                active
            );
            let deadline = tokio::time::Instant::now() + drain_timeout;
            loop {
                let remaining = self.services.connection_manager.connection_count();
                if remaining == 0 {
                    info!("All connections drained");
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    warn!(
                        "Drain timeout reached with {} connection(s) still active, proceeding with shutdown",
                        remaining
                    );
                    break;
                }
                tokio::time::sleep(drain_poll_interval).await;
            }
        }

        // 2. Shutdown SFU manager (close all SFU rooms)
        if let Some(ref sfu) = self.services.sfu_manager {
            info!("Shutting down SFU manager...");
            sfu.shutdown().await;
        }

        // 3. STUN/TURN servers shut down when their Arc references are dropped
        if self.services.stun_server.is_some() {
            info!("STUN server shutting down");
        }
        if self.services.turn_server.is_some() {
            info!("TURN server shutting down");
        }

        // 4. Stop livestream: drain active pull streams and clear the registry
        if let Some(ref state) = self.livestream_state {
            let stream_count = state.registry.len();
            info!("Stopping livestream infrastructure ({} active stream(s))...", stream_count);

            // Clear the HLS stream registry so no new segments are served
            state.registry.clear();

            // Stop all active pull streams managed by the PullStreamManager
            // (PullStreamManager.streams is private, but dropping the Arc will
            //  clean up; the registry clear above prevents new pull requests.)
            info!("Livestream infrastructure shut down");
        }

        // 5. Redis publish channel closes when sender is dropped
        if self.services.redis_publish_tx.is_some() {
            info!("Closing Redis publish channel");
        }

        // 6. Close the database connection pool
        info!("Closing database connection pool...");
        self.pool.close().await;
        info!("Database pool closed");

        info!("SyncTV server shut down complete");
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
        let sfu_manager = self.services.sfu_manager.clone();
        let live_streaming_infrastructure = self.services.live_streaming_infrastructure.clone();
        let publish_key_service = self.services.publish_key_service.clone();

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
                sfu_manager,
                live_streaming_infrastructure,
                Some(publish_key_service),
            )
            .await
            {
                error!("gRPC server error: {}", e);
            }
        });

        Ok(handle)
    }

    /// Start HTTP server with graceful shutdown support
    async fn start_http_server(&self, shutdown_rx: watch::Receiver<bool>) -> anyhow::Result<JoinHandle<()>> {
        let http_address = self.config.http_address();
        let user_service = self.services.user_service.clone();
        let room_service = self.services.room_service.clone();
        let provider_instance_manager = self.services.provider_instance_manager.clone();
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
        let sfu_manager = self.services.sfu_manager.clone();

        let http_router = synctv_api::http::create_router(
            Arc::new(self.config.clone()),
            user_service,
            room_service,
            provider_instance_manager,
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
            sfu_manager,
        );

        let handle = tokio::spawn(async move {
            let http_addr: std::net::SocketAddr = match http_address.parse() {
                Ok(addr) => addr,
                Err(e) => {
                    error!("Invalid HTTP address '{}': {}", http_address, e);
                    return;
                }
            };

            let listener = match tokio::net::TcpListener::bind(http_addr).await {
                Ok(listener) => listener,
                Err(e) => {
                    error!("Failed to bind HTTP address {}: {}", http_addr, e);
                    return;
                }
            };

            info!("HTTP server listening on {}", http_addr);

            let mut rx = shutdown_rx;
            let graceful = async move {
                let _ = rx.changed().await;
            };

            if let Err(e) = axum::serve(listener, http_router)
                .with_graceful_shutdown(graceful)
                .await
            {
                error!("HTTP server error: {}", e);
            }

            info!("HTTP server shut down gracefully");
        });

        Ok(handle)
    }
}

/// Wait for a shutdown signal (SIGTERM or SIGINT/Ctrl+C)
async fn shutdown_signal() {
    let ctrl_c = async {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                info!("Received Ctrl+C signal");
            }
            Err(e) => {
                error!("Failed to install Ctrl+C handler: {}", e);
            }
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
                info!("Received SIGTERM signal");
            }
            Err(e) => {
                error!("Failed to install SIGTERM handler: {}", e);
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => { info!("Received Ctrl+C"); }
        () = terminate => { info!("Received SIGTERM"); }
    }
}
