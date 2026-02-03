mod rtmp;
mod server;

use anyhow::Result;
use std::sync::Arc;
use tracing::{error, info};
use tokio::sync::mpsc;

use synctv_core::{
    logging,
    bootstrap::{load_config, init_database, init_services},
    provider::{AlistProvider, BilibiliProvider, EmbyProvider},
};
use synctv_cluster::sync::{RoomMessageHub, ConnectionManager, ClusterManager, ClusterConfig};

use server::{SyncTvServer, Services, StreamingState};

/// Generate a unique node ID for this server instance
fn generate_node_id() -> String {
    use std::net::UdpSocket;

    // Try to get hostname, fallback to "unknown"
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());

    // Get local IP address if available
    let local_ip = UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("8.8.8.8:80").map(|_| s))
        .and_then(|s| s.local_addr())
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|_| "0.0.0.0".to_string());

    // Add random suffix for uniqueness
    let suffix = nanoid::nanoid!(6);

    format!("{}_{}-{}", hostname, local_ip, suffix)
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Load configuration
    let config = load_config()?;

    // 2. Initialize logging
    logging::init_logging(&config.logging)?;
    info!("SyncTV server starting...");
    info!("gRPC address: {}", config.grpc_address());
    info!("HTTP address: {}", config.http_address());

    // 3. Initialize database
    let pool = init_database(&config).await?;

    // 4. Run migrations
    info!("Running database migrations...");
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .map_err(|e| {
            error!("Failed to run migrations: {}", e);
            anyhow::anyhow!("Migration failed: {}", e)
        })?;
    info!("Migrations completed");

    // 5. Initialize services
    let synctv_services = init_services(pool.clone(), &config).await?;

    // 6. Initialize RoomMessageHub
    let message_hub = Arc::new(RoomMessageHub::new());
    info!("RoomMessageHub initialized");

    // 7. Initialize ClusterManager (unified cluster management)
    let cluster_manager = if !config.redis.url.is_empty() {
        let cluster_config = ClusterConfig {
            redis_url: config.redis.url.clone(),
            node_id: generate_node_id(),
            dedup_window: std::time::Duration::from_secs(5),
            cleanup_interval: std::time::Duration::from_secs(30),
        };
        match ClusterManager::new(cluster_config).await {
            Ok(manager) => {
                info!("ClusterManager initialized successfully");
                Some(Arc::new(manager))
            }
            Err(e) => {
                error!("Failed to create ClusterManager: {}", e);
                error!("Continuing in single-node mode");
                None
            }
        }
    } else {
        info!("Redis not configured, using single-node ClusterManager");
        // Create a single-node ClusterManager (no Redis)
        let cluster_config = ClusterConfig {
            redis_url: String::new(),
            node_id: generate_node_id(),
            dedup_window: std::time::Duration::from_secs(5),
            cleanup_interval: std::time::Duration::from_secs(30),
        };
        match ClusterManager::new(cluster_config).await {
            Ok(manager) => Some(Arc::new(manager)),
            Err(e) => {
                error!("Failed to create single-node ClusterManager: {}", e);
                None
            }
        }
    };

    // 8. Initialize connection manager
    let connection_manager = ConnectionManager::default();
    info!("Connection manager initialized");

    // Note: Redis Pub/Sub is now handled by ClusterManager
    // We get the publish_tx from cluster_manager for backward compatibility
    let redis_publish_tx = cluster_manager
        .as_ref()
        .and_then(|cm| cm.redis_publish_tx().cloned());

    // 9. Initialize streaming components (RTMP server)
    let streaming_state = if !config.redis.url.is_empty() {
        info!("Initializing RTMP server...");

        match redis::Client::open(config.redis.url.clone()) {
            Ok(redis_client) => {
                match redis_client.get_connection_manager().await {
                    Ok(redis_conn) => {
                        // Create StreamRegistry
                        let registry = synctv_stream::relay::StreamRegistry::new(redis_conn);

                        // Create GOP cache for pull streams
                        let gop_cache_config = synctv_stream::cache::GopCacheConfig::default();
                        let gop_cache = Arc::new(synctv_stream::cache::GopCache::new(
                            gop_cache_config,
                        ));

                        // Create PullStreamManager
                        let node_id = generate_node_id();
                        let pull_manager = Arc::new(synctv_stream::streaming::PullStreamManager::new(
                            gop_cache.clone(),
                            registry.clone(),
                            node_id.clone(),
                        ));

                        // Create RTMP authentication callback
                        let rtmp_auth = Arc::new(rtmp::SyncTvRtmpAuth::new(
                            synctv_services.room_service.clone(),
                            synctv_services.jwt_service.clone(),
                        ));

                        // Create StreamHub event sender for RTMP server
                        let (stream_hub_event_sender, _stream_hub_event_receiver) =
                            mpsc::unbounded_channel::<streamhub::define::StreamHubEvent>();

                        // Start RTMP server in background
                        let rtmp_address = format!("{}:{}", config.server.host, config.streaming.rtmp_port);
                        let mut rtmp_server = synctv_stream::RtmpStreamingServer::new(
                            rtmp_address.clone(),
                            gop_cache,
                            registry.clone(),
                            node_id,
                            rtmp_auth,
                            stream_hub_event_sender,
                        );

                        tokio::spawn(async move {
                            info!("Starting RTMP server on rtmp://{}...", rtmp_address);
                            if let Err(e) = rtmp_server.start().await {
                                error!("RTMP server error: {}", e);
                            }
                        });

                        info!("RTMP server initialized successfully");

                        Some(StreamingState {
                            registry,
                            pull_manager,
                        })
                    }
                    Err(e) => {
                        error!("Failed to create Redis connection manager for streaming: {}", e);
                        info!("Streaming routes disabled");
                        None
                    }
                }
            }
            Err(e) => {
                error!("Failed to create Redis client for streaming: {}", e);
                info!("Streaming routes disabled");
                None
            }
        }
    } else {
        info!("Redis not configured, streaming routes disabled");
        None
    };

    // 10. Create server with all services
    let provider_instance_manager = synctv_services.provider_instance_manager.clone();
    let alist_provider = Arc::new(AlistProvider::new(provider_instance_manager.clone()));
    let bilibili_provider = Arc::new(BilibiliProvider::new(provider_instance_manager.clone()));
    let emby_provider = Arc::new(EmbyProvider::new(provider_instance_manager.clone()));

    let services = Services {
        user_service: synctv_services.user_service.clone(),
        room_service: synctv_services.room_service.clone(),
        jwt_service: synctv_services.jwt_service.clone(),
        message_hub,
        cluster_manager,
        redis_publish_tx,
        rate_limiter: synctv_services.rate_limiter.clone(),
        rate_limit_config: synctv_services.rate_limit_config.clone(),
        content_filter: synctv_services.content_filter.clone(),
        connection_manager,
        providers_manager: synctv_services.providers_manager.clone(),
        provider_instance_manager,
        provider_instance_repository: synctv_services.provider_instance_repo.clone(),
        user_provider_credential_repository: synctv_services.user_provider_credential_repo.clone(),
        alist_provider,
        bilibili_provider,
        emby_provider,
        oauth2_service: synctv_services.oauth2_service.clone(),
        settings_service: synctv_services.settings_service.clone(),
        settings_registry: synctv_services.settings_registry.clone(),
        email_service: synctv_services.email_service.clone(),
        email_token_service: synctv_services.email_token_service.clone(),
        publish_key_service: synctv_services.publish_key_service.clone(),
        notification_service: Some(synctv_services.notification_service.clone()),
    };

    let server = SyncTvServer::new(config, services, streaming_state);

    // 11. Start all servers
    server.start().await?;

    Ok(())
}
