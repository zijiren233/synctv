mod rtmp_auth;
mod server;

use anyhow::Result;
use std::sync::Arc;
use tracing::{error, info, warn};

use synctv_core::{
    logging,
    bootstrap::{load_config, init_database, init_services, bootstrap_root_user},
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

    // 1.5. Validate configuration (fail fast on misconfigurations)
    if let Err(errors) = config.validate() {
        for e in &errors {
            eprintln!("Config validation error: {e}");
        }
        // JWT key warnings are non-fatal (keys may be generated later)
        let fatal: Vec<_> = errors
            .iter()
            .filter(|e| !e.contains("JWT"))
            .collect();
        if !fatal.is_empty() {
            return Err(anyhow::anyhow!(
                "Configuration validation failed with {} error(s)",
                fatal.len()
            ));
        }
        eprintln!("Continuing with JWT key warnings (keys may be generated at runtime)");
    }

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

    // 4.3. Bootstrap root user (if enabled and no root user exists)
    info!("Checking root user bootstrap...");
    if let Err(e) = bootstrap_root_user(&pool, &config.bootstrap).await {
        error!("Failed to bootstrap root user: {}", e);
        error!("You may need to manually create a root user");
        // Non-fatal: continue startup even if bootstrap fails
    }

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

    // 8. Initialize connection manager with configurable limits
    use synctv_cluster::sync::ConnectionLimits;
    use std::time::Duration;
    let connection_limits = ConnectionLimits {
        max_per_user: config.connection_limits.max_per_user,
        max_per_room: config.connection_limits.max_per_room,
        max_total: config.connection_limits.max_total,
        idle_timeout: Duration::from_secs(config.connection_limits.idle_timeout_seconds),
        max_duration: Duration::from_secs(config.connection_limits.max_duration_seconds),
    };
    let connection_manager = ConnectionManager::new(connection_limits);
    info!(
        max_per_user = config.connection_limits.max_per_user,
        max_per_room = config.connection_limits.max_per_room,
        max_total = config.connection_limits.max_total,
        "Connection manager initialized with configurable limits"
    );

    // Note: Redis Pub/Sub is now handled by ClusterManager
    // We get the publish_tx from cluster_manager for backward compatibility
    let redis_publish_tx = cluster_manager
        .as_ref()
        .and_then(|cm| cm.redis_publish_tx().cloned());

    // 9. Initialize livestream components (RTMP server and live streaming infrastructure)
    let (livestream_state, live_streaming_infrastructure) = if config.redis.url.is_empty() {
        info!("Redis not configured, livestream features disabled");
        (None, None)
    } else {
        info!("Initializing livestream infrastructure...");

        match redis::Client::open(config.redis.url.clone()) {
            Ok(redis_client) => {
                match redis_client.get_connection_manager().await {
                    Ok(redis_conn) => {
                        // Create two registries:
                        // 1. Stream state registry for HLS (DashMap-based)
                        let stream_registry: synctv_livestream::StreamRegistry = Arc::new(dashmap::DashMap::new());

                        // 2. Publisher registry for Redis (used by PullStreamManager)
                        let publisher_registry = Arc::new(synctv_livestream::relay::StreamRegistry::new(redis_conn)) as Arc<dyn synctv_livestream::relay::StreamRegistryTrait>;

                        // Create dummy StreamHub event sender (events not currently handled)
                        // TODO: Implement event handling via PublisherManager if needed
                        let (stream_hub_event_sender, _) =
                            tokio::sync::mpsc::unbounded_channel();

                        // Create PullStreamManager (uses publisher_registry for Redis)
                        let node_id = generate_node_id();
                        let pull_manager = Arc::new(synctv_livestream::livestream::PullStreamManager::new(
                            publisher_registry.clone(),
                            node_id.clone(),
                            stream_hub_event_sender.clone(),
                        ));

                        // Clone resources for RTMP server
                        let rtmp_event_sender = stream_hub_event_sender.clone();

                        // Shared tracker for user→stream mapping (kick-on-ban)
                        let user_stream_tracker: synctv_livestream::api::UserStreamTracker =
                            Arc::new(synctv_livestream::api::StreamTracker::new());

                        // Create LiveStreamingInfrastructure (uses publisher_registry for Redis)
                        let live_infra = Arc::new(synctv_livestream::api::LiveStreamingInfrastructure::new(
                            publisher_registry.clone(),
                            stream_hub_event_sender,
                            pull_manager.clone(),
                            user_stream_tracker.clone(),
                        ));

                        // Create RTMP authentication callback
                        let rtmp_auth: Arc<dyn synctv_xiu::rtmp::auth::AuthCallback> =
                            Arc::new(rtmp_auth::SyncTvRtmpAuth::new(
                                synctv_services.room_service.clone(),
                                synctv_services.user_service.clone(),
                                synctv_services.publish_key_service.clone(),
                                Some(synctv_services.settings_registry.clone()),
                                user_stream_tracker,
                                publisher_registry.clone(),
                                node_id,
                            ));

                        // Create and start RTMP server with auth integration
                        let rtmp_listen_addr = format!("{}:{}", config.server.host, config.livestream.rtmp_port);
                        let mut rtmp_server = synctv_xiu::rtmp::rtmp::RtmpServer::new(
                            rtmp_listen_addr.clone(),
                            rtmp_event_sender,
                            2, // gop_num
                            Some(rtmp_auth),
                        );

                        tokio::spawn(async move {
                            if let Err(e) = rtmp_server.run().await {
                                error!("RTMP server error: {}", e);
                            }
                        });

                        info!("Livestream infrastructure initialized, RTMP server listening on rtmp://{}", rtmp_listen_addr);

                        let state = Some(server::LivestreamState {
                            registry: stream_registry,
                            pull_manager,
                        });

                        (state, Some(live_infra))
                    }
                    Err(e) => {
                        error!("Failed to connect to Redis for livestream: {}", e);
                        (None, None)
                    }
                }
            }
            Err(e) => {
                error!("Failed to create Redis client for livestream: {}", e);
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
                // Validate external TURN configuration
                let turn_config = synctv_core::service::TurnConfig {
                    server_url: url.clone(),
                    static_secret: secret.clone(),
                    credential_ttl: std::time::Duration::from_secs(config.webrtc.turn_credential_ttl),
                    use_tls: false,
                };
                let turn_service = synctv_core::service::TurnCredentialService::new(turn_config);
                if let Err(e) = turn_service.validate_config() {
                    error!("External TURN configuration is invalid: {}", e);
                    error!("Please check webrtc.external_turn_server_url and webrtc.external_turn_static_secret");
                } else {
                    info!("External TURN server configured: {}", url);
                    info!("TURN credential TTL: {} seconds", config.webrtc.turn_credential_ttl);
                    info!("Note: Ensure coturn is deployed and static-auth-secret matches");
                }
            } else {
                warn!("External TURN mode selected but server URL or secret not configured");
                warn!("Set webrtc.external_turn_server_url and webrtc.external_turn_static_secret in config");
            }
            None
        }
        synctv_core::config::TurnMode::Disabled => {
            info!("TURN server disabled (P2P + STUN only)");
            info!("Connection success rate may be lower (~85-90%) without TURN");
            None
        }
    };

    // 9.7. Initialize SFU manager (if needed for WebRTC mode)
    let sfu_manager = if config.webrtc.mode == synctv_core::config::WebRTCMode::SFU
        || config.webrtc.mode == synctv_core::config::WebRTCMode::Hybrid
    {
        info!("Initializing SFU manager for mode: {:?}", config.webrtc.mode);
        let sfu_config = synctv_sfu::SfuConfig {
            sfu_threshold: config.webrtc.sfu_threshold,
            max_sfu_rooms: config.webrtc.max_sfu_rooms,
            max_peers_per_room: config.webrtc.max_peers_per_sfu_room,
            enable_simulcast: config.webrtc.enable_simulcast,
            simulcast_layers: vec!["high".to_string(), "medium".to_string(), "low".to_string()],
            max_bitrate_per_peer: 0, // 0 = no limit
            enable_bandwidth_estimation: true,
        };
        let manager = synctv_sfu::SfuManager::new(sfu_config);
        info!(
            "SFU manager initialized (threshold: {}, max_rooms: {}, max_peers_per_room: {})",
            config.webrtc.sfu_threshold,
            if config.webrtc.max_sfu_rooms == 0 { "unlimited".to_string() } else { config.webrtc.max_sfu_rooms.to_string() },
            config.webrtc.max_peers_per_sfu_room
        );
        Some(manager)
    } else {
        info!("SFU manager disabled (mode: {:?})", config.webrtc.mode);
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
        live_streaming_infrastructure,
        stun_server,
        turn_server,
        sfu_manager,
    };

    let server = SyncTvServer::new(config, services, livestream_state, pool);

    // 11. Start all servers
    server.start().await?;

    Ok(())
}
