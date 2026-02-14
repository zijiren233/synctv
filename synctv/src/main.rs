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
        if !errors.is_empty() {
            return Err(anyhow::anyhow!(
                "Configuration validation failed with {} error(s)",
                errors.len()
            ));
        }
    }

    // 2. Initialize logging
    // Hold the guard so buffered log entries are flushed on shutdown
    let _log_guard = logging::init_logging(&config.logging)?;
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
    if let Err(e) = bootstrap_root_user(&pool, &config.bootstrap, config.server.development_mode).await {
        warn!("Failed to bootstrap root user: {}", e);
        warn!("You may need to manually create a root user");
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
    // Extract permission service for cross-replica cache invalidation
    let permission_service = Some(synctv_services.room_service.permission_service().clone());

    let cluster_manager = if config.redis.url.is_empty() {
        info!("Redis not configured, using single-node ClusterManager");
        // Create a single-node ClusterManager (no Redis)
        let cluster_config = ClusterConfig {
            redis_url: String::new(),
            node_id: generate_node_id(),
            dedup_window: std::time::Duration::from_secs(5),
            cleanup_interval: std::time::Duration::from_secs(30),
        };
        match ClusterManager::new(cluster_config, None).await {
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
        match ClusterManager::new(cluster_config, permission_service).await {
            Ok(manager) => {
                info!("ClusterManager initialized with cross-replica permission cache invalidation");
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
                        // Publisher registry for Redis (used by PullStreamManager + PublisherManager)
                        let publisher_registry = Arc::new(synctv_livestream::relay::StreamRegistry::new(redis_conn)) as Arc<dyn synctv_livestream::relay::StreamRegistryTrait>;

                        // Shared tracker for user→stream mapping (kick-on-ban)
                        let user_stream_tracker: synctv_livestream::api::UserStreamTracker =
                            Arc::new(synctv_livestream::api::StreamTracker::new());

                        // Stream lifecycle event channel (app-level logging)
                        let (stream_lifecycle_tx, mut stream_lifecycle_rx) =
                            tokio::sync::broadcast::channel::<rtmp_auth::StreamLifecycleEvent>(64);

                        tokio::spawn(async move {
                            while let Ok(event) = stream_lifecycle_rx.recv().await {
                                match event {
                                    rtmp_auth::StreamLifecycleEvent::Started { room_id, media_id, user_id } => {
                                        info!(
                                            room_id = %room_id,
                                            media_id = %media_id,
                                            user_id = %user_id,
                                            "Stream started"
                                        );
                                    }
                                    rtmp_auth::StreamLifecycleEvent::Stopped { room_id, media_id, user_id } => {
                                        info!(
                                            room_id = %room_id,
                                            media_id = %media_id,
                                            user_id = %user_id,
                                            "Stream stopped"
                                        );
                                    }
                                }
                            }
                        });

                        // RTMP auth callback (needs synctv-core services)
                        let node_id = generate_node_id();
                        let rtmp_auth: Arc<dyn synctv_livestream::AuthCallback> =
                            Arc::new(rtmp_auth::SyncTvRtmpAuth::new(
                                synctv_services.room_service.clone(),
                                synctv_services.user_service.clone(),
                                synctv_services.publish_key_service.clone(),
                                user_stream_tracker.clone(),
                                publisher_registry.clone(),
                                node_id.clone(),
                                Some(stream_lifecycle_tx),
                            ));

                        // One-shot facade: start all xiu components
                        let rtmp_listen_addr = format!("{}:{}", config.server.host, config.livestream.rtmp_port);
                        let handle = synctv_livestream::LivestreamServer::new(
                            synctv_livestream::LivestreamConfig {
                                rtmp_address: rtmp_listen_addr,
                                gop_cache_size: config.livestream.gop_cache_size as usize,
                                node_id,
                                cleanup_check_interval_seconds: config.livestream.cleanup_check_interval_seconds,
                                stream_timeout_seconds: config.livestream.stream_timeout_seconds,
                            },
                            publisher_registry,
                            user_stream_tracker,
                        )
                        .with_auth(rtmp_auth)
                        .start()
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to start livestream: {e}"))?;

                        let live_infra = handle.infrastructure.clone();
                        let state = Some(server::LivestreamState { handle });

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
            max_packet_size: config.webrtc.stun_max_packet_size,
        };
        match synctv_core::service::StunServer::start(stun_config).await {
            Ok(server) => {
                let addr = server.local_addr()?;
                info!("Built-in STUN server started on {}", addr);
                Some(server)
            }
            Err(e) => {
                warn!("Failed to start STUN server: {}", e);
                warn!("WebRTC P2P connectivity may be limited without STUN");
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
                    default_lifetime: config.webrtc.builtin_turn_default_lifetime,
                    max_lifetime: config.webrtc.builtin_turn_max_lifetime,
                    static_secret: config.webrtc.external_turn_static_secret
                        .clone()
                        .unwrap_or_else(|| {
                            warn!("No TURN static_secret configured! Please set webrtc.external_turn_static_secret in config. Using ephemeral random secret (will change on restart).");
                            nanoid::nanoid!(48)
                        }),
                    realm: config.webrtc.turn_realm
                        .clone()
                        .unwrap_or_else(|| "synctv.local".to_string()),
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
                        warn!("Failed to start TURN server: {}", e);
                        warn!("WebRTC connectivity may fail in restrictive networks without TURN");
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
                    warn!("External TURN configuration is invalid: {}", e);
                    warn!("Please check webrtc.external_turn_server_url and webrtc.external_turn_static_secret");
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
            simulcast_layers: config.webrtc.simulcast_layers.clone(),
            max_bitrate_per_peer: config.webrtc.max_bitrate_per_peer,
            enable_bandwidth_estimation: config.webrtc.enable_bandwidth_estimation,
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
