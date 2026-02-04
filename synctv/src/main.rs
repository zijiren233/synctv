mod rtmp;
mod server;

use anyhow::Result;
use std::sync::Arc;
use tracing::{error, info, warn};

use synctv_core::{
    logging,
    bootstrap::{load_config, init_database, init_services},
    provider::{AlistProvider, BilibiliProvider, EmbyProvider},
};
use synctv_cluster::sync::{RoomMessageHub, ConnectionManager, ClusterManager, ClusterConfig};

use server::{SyncTvServer, Services};

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
        .and_then(|s| s.connect("8.8.8.8:80").map(|()| s))
        .and_then(|s| s.local_addr()).map_or_else(|_| "0.0.0.0".to_string(), |addr| addr.ip().to_string());

    // Add random suffix for uniqueness
    let suffix = nanoid::nanoid!(6);

    format!("{hostname}_{local_ip}-{suffix}")
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
            anyhow::anyhow!("Migration failed: {e}")
        })?;
    info!("Migrations completed");

    // 4.5. Initialize audit log partitions
    info!("Initializing audit log partitions...");
    synctv_core::service::ensure_audit_partitions_on_startup(&pool)
        .await
        .map_err(|e| {
            warn!("Failed to initialize audit partitions (non-fatal): {}", e);
            // Non-fatal: continue startup even if partition creation fails
            e
        })
        .ok();

    // Start automatic partition management (check every 24 hours)
    let audit_manager = synctv_core::service::AuditPartitionManager::new(pool.clone());
    let _audit_task = audit_manager.start_auto_management(24);
    info!("Audit log partition management started");

    // 5. Initialize services
    let synctv_services = init_services(pool.clone(), &config).await?;

    // 6. Initialize RoomMessageHub
    let message_hub = Arc::new(RoomMessageHub::new());
    info!("RoomMessageHub initialized");

    // 7. Initialize ClusterManager (unified cluster management)
    let cluster_manager = if config.redis.url.is_empty() {
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
    } else {
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
    };

    // 8. Initialize connection manager
    let connection_manager = ConnectionManager::default();
    info!("Connection manager initialized");

    // Note: Redis Pub/Sub is now handled by ClusterManager
    // We get the publish_tx from cluster_manager for backward compatibility
    let redis_publish_tx = cluster_manager
        .as_ref()
        .and_then(|cm| cm.redis_publish_tx().cloned());

    // 9. Initialize streaming components (RTMP server and live streaming infrastructure)
    let (streaming_state, live_streaming_infrastructure) = if config.redis.url.is_empty() {
        info!("Redis not configured, streaming features disabled");
        (None, None)
    } else {
        info!("Initializing streaming infrastructure...");

        match redis::Client::open(config.redis.url.clone()) {
            Ok(redis_client) => {
                match redis_client.get_connection_manager().await {
                    Ok(redis_conn) => {
                        // Create two registries:
                        // 1. Stream state registry for HLS (DashMap-based)
                        let stream_registry: synctv_stream::StreamRegistry = Arc::new(dashmap::DashMap::new());

                        // 2. Publisher registry for Redis (used by PullStreamManager)
                        let publisher_registry = Arc::new(synctv_stream::relay::StreamRegistry::new(redis_conn)) as Arc<dyn synctv_stream::relay::StreamRegistryTrait>;

                        // Create GOP cache
                        let gop_cache_config = synctv_stream::libraries::gop_cache::GopCacheConfig::default();
                        let gop_cache = Arc::new(synctv_stream::GopCache::new(gop_cache_config));

                        // Create StreamHub event sender (needed by PullStreamManager)
                        let (stream_hub_event_sender, _stream_hub_event_receiver) =
                            tokio::sync::mpsc::unbounded_channel::<streamhub::define::StreamHubEvent>();

                        // Create PullStreamManager (uses publisher_registry for Redis)
                        let node_id = generate_node_id();
                        let pull_manager = Arc::new(synctv_stream::streaming::PullStreamManager::new(
                            gop_cache.clone(),
                            publisher_registry.clone(),
                            node_id,
                            stream_hub_event_sender.clone(),
                        ));

                        // Create LiveStreamingInfrastructure (uses publisher_registry for Redis)
                        let live_infra = Arc::new(synctv_stream::api::LiveStreamingInfrastructure::new(
                            publisher_registry.clone(),
                            stream_hub_event_sender,
                            gop_cache,
                            pull_manager.clone(),
                        ));

                        // Create RTMP authentication callback
                        let _rtmp_auth = Arc::new(rtmp::SyncTvRtmpAuth::new(
                            synctv_services.room_service.clone(),
                            synctv_services.publish_key_service.clone(),
                        ));

                        // Create RTMP server configuration
                        let rtmp_listen_addr = format!("{}:{}", config.server.host, config.streaming.rtmp_port);
                        let rtmp_config = synctv_stream::xiu_integration::RtmpConfig {
                            listen_addr: rtmp_listen_addr.parse().expect("Invalid RTMP address"),
                            max_streams: 1000,
                            chunk_size: 4096,
                            gop_num: 2,
                        };
                        let _rtmp_server = synctv_stream::RtmpServer::new(rtmp_config);

                        // TODO: Start RTMP server (需要实现 build() 和 run() 方法)
                        info!("Streaming infrastructure initialized (RTMP server not started yet)");

                        let state = Some(server::StreamingState {
                            registry: stream_registry,
                            pull_manager,
                        });

                        (state, Some(live_infra))
                    }
                    Err(e) => {
                        error!("Failed to connect to Redis for streaming: {}", e);
                        (None, None)
                    }
                }
            }
            Err(e) => {
                error!("Failed to create Redis client for streaming: {}", e);
                (None, None)
            }
        }
    };

    // 9.5. Initialize STUN server (if enabled)
    let stun_server = if config.webrtc.enable_builtin_stun {
        info!("Starting built-in STUN server...");
        let stun_config = synctv_core::service::StunServerConfig {
            bind_addr: format!("{}:{}", config.webrtc.builtin_stun_host, config.webrtc.builtin_stun_port),
            max_packet_size: 1500,
        };
        match synctv_core::service::StunServer::start(stun_config).await {
            Ok(server) => {
                let addr = server.local_addr()?;
                info!("Built-in STUN server started on {}", addr);
                Some(server)
            }
            Err(e) => {
                error!("Failed to start STUN server: {}", e);
                error!("WebRTC P2P connectivity may be limited without STUN");
                None
            }
        }
    } else {
        info!("Built-in STUN server disabled");
        None
    };

    // 9.6. Initialize TURN server (if enabled)
    let turn_server = match config.webrtc.turn_mode {
        synctv_core::config::TurnMode::Builtin => {
            if config.webrtc.enable_builtin_turn {
                info!("Starting built-in TURN server...");
                let turn_config = synctv_core::service::TurnBuiltinServerConfig {
                    bind_addr: format!("{}:{}", config.webrtc.builtin_stun_host, config.webrtc.builtin_turn_port),
                    relay_min_port: config.webrtc.builtin_turn_min_port,
                    relay_max_port: config.webrtc.builtin_turn_max_port,
                    max_allocations: config.webrtc.builtin_turn_max_allocations,
                    default_lifetime: 600,
                    max_lifetime: 3600,
                    static_secret: config.webrtc.external_turn_static_secret
                        .clone()
                        .unwrap_or_else(|| {
                            warn!("No TURN static_secret configured, using default (INSECURE!)");
                            "insecure_default_secret".to_string()
                        }),
                    realm: "synctv.local".to_string(),
                };
                match synctv_core::service::TurnServer::start(turn_config).await {
                    Ok(server) => {
                        let addr = server.local_addr()?;
                        info!("Built-in TURN server started on {}", addr);
                        info!("TURN relay port range: {}-{}",
                            config.webrtc.builtin_turn_min_port,
                            config.webrtc.builtin_turn_max_port);
                        Some(server)
                    }
                    Err(e) => {
                        error!("Failed to start TURN server: {}", e);
                        error!("WebRTC connectivity may fail in restrictive networks without TURN");
                        None
                    }
                }
            } else {
                info!("Built-in TURN server available but disabled in config");
                None
            }
        }
        synctv_core::config::TurnMode::External => {
            info!("Using external TURN server (coturn)");
            if let (Some(url), Some(secret)) = (
                &config.webrtc.external_turn_server_url,
                &config.webrtc.external_turn_static_secret,
            ) {
                info!("External TURN server configured: {}", url);
                info!("Note: Ensure coturn is deployed and static-auth-secret matches");
            } else {
                warn!("External TURN mode selected but server URL or secret not configured");
            }
            None
        }
        synctv_core::config::TurnMode::Disabled => {
            info!("TURN server disabled (P2P + STUN only)");
            info!("Connection success rate may be lower (~85-90%) without TURN");
            None
        }
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
        live_streaming_infrastructure,
        stun_server,
        turn_server,
    };

    let server = SyncTvServer::new(config, services, streaming_state);

    // 11. Start all servers
    server.start().await?;

    Ok(())
}
