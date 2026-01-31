mod grpc;
mod http;
mod observability;

use anyhow::Result;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use synctv_core::{Config, logging, service::{JwtService, UserService, RateLimiter, RateLimitConfig, ContentFilter}};
use tracing::{info, error};

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration
    let config = Config::from_env().unwrap_or_else(|e| {
        eprintln!("Failed to load config: {}", e);
        eprintln!("Using default configuration");
        Config {
            server: synctv_core::config::ServerConfig::default(),
            database: synctv_core::config::DatabaseConfig::default(),
            redis: synctv_core::config::RedisConfig::default(),
            jwt: synctv_core::config::JwtConfig::default(),
            logging: synctv_core::config::LoggingConfig::default(),
            streaming: synctv_core::config::StreamingConfig::default(),
        }
    });

    // Initialize logging
    logging::init_logging(&config.logging)?;

    info!("SyncTV API Server starting...");
    info!("gRPC address: {}", config.grpc_address());
    info!("HTTP address: {}", config.http_address());

    // Initialize database pool
    info!("Connecting to database: {}", config.database_url());
    let pool: sqlx::PgPool = PgPoolOptions::new()
        .max_connections(config.database.max_connections)
        .min_connections(config.database.min_connections)
        .acquire_timeout(Duration::from_secs(config.database.connect_timeout_seconds))
        .idle_timeout(Duration::from_secs(config.database.idle_timeout_seconds))
        .connect(config.database_url())
        .await
        .map_err(|e| {
            error!("Failed to connect to database: {}", e);
            anyhow::anyhow!("Database connection failed: {}", e)
        })?;

    info!("Database connected successfully");

    // Run migrations
    info!("Running database migrations...");
    sqlx::migrate!("../migrations")
        .run(&pool as &sqlx::PgPool)
        .await
        .map_err(|e| {
            error!("Failed to run migrations: {}", e);
            anyhow::anyhow!("Migration failed: {}", e)
        })?;
    info!("Migrations completed");

    // Initialize JWT service
    info!("Loading JWT keys...");
    let jwt_service = load_jwt_service(&config)?;
    info!("JWT service initialized");

    // Initialize UserService
    let user_service = UserService::new(pool.clone(), jwt_service);
    info!("UserService initialized");

    // Initialize RoomService
    let room_service = synctv_core::service::RoomService::new(pool);
    info!("RoomService initialized");

    // Initialize RoomMessageHub for real-time messaging
    let message_hub = std::sync::Arc::new(synctv_cluster::sync::RoomMessageHub::new());
    info!("RoomMessageHub initialized");

    // Initialize rate limiter
    let redis_url = if !config.redis.url.is_empty() {
        Some(config.redis.url.clone())
    } else {
        None
    };
    let rate_limiter = RateLimiter::new(redis_url.clone(), config.redis.key_prefix.clone())?;
    let rate_limit_config = RateLimitConfig::default();
    info!(
        "Rate limiter initialized (chat: {}/s, danmaku: {}/s)",
        rate_limit_config.chat_per_second,
        rate_limit_config.danmaku_per_second
    );

    // Initialize content filter
    let content_filter = ContentFilter::new();
    info!(
        "Content filter initialized (max chat: {} chars, max danmaku: {} chars)",
        content_filter.max_chat_length,
        content_filter.max_danmaku_length
    );

    // Initialize connection manager
    let connection_manager = synctv_cluster::sync::ConnectionManager::default();
    info!("Connection manager initialized");

    // Initialize Redis Pub/Sub for multi-replica synchronization
    let redis_publish_tx = if !config.redis.url.is_empty() {
        match synctv_cluster::sync::RedisPubSub::new(
            &config.redis.url,
            message_hub.clone(),
            generate_node_id(),
        ) {
            Ok(redis_pubsub) => {
                let redis_pubsub = std::sync::Arc::new(redis_pubsub);
                match redis_pubsub.start().await {
                    Ok(tx) => {
                        info!("Redis Pub/Sub initialized successfully");
                        Some(tx)
                    }
                    Err(e) => {
                        error!("Failed to start Redis Pub/Sub: {}", e);
                        error!("Continuing in single-node mode without Redis");
                        None
                    }
                }
            }
            Err(e) => {
                error!("Failed to create Redis Pub/Sub client: {}", e);
                error!("Continuing in single-node mode without Redis");
                None
            }
        }
    } else {
        info!("Redis URL not configured, running in single-node mode");
        None
    };

    // Wrap services in Arc for sharing between gRPC and HTTP servers
    let user_service = std::sync::Arc::new(user_service);
    let room_service = std::sync::Arc::new(room_service);

    // Clone Arc for HTTP server
    let user_service_http = user_service.clone();
    let room_service_http = room_service.clone();

    // Start gRPC server in background task
    info!("Starting gRPC server on {}...", config.grpc_address());
    let grpc_config = config.clone();
    let grpc_handle = tokio::spawn(async move {
        if let Err(e) = grpc::serve(
            &grpc_config,
            user_service,
            room_service,
            message_hub,
            redis_publish_tx,
            rate_limiter,
            rate_limit_config,
            content_filter,
            connection_manager,
        ).await {
            error!("gRPC server error: {}", e);
        }
    });

    // Start HTTP/REST server
    let http_address = config.http_address();
    info!("Starting HTTP server on {}...", http_address);
    let http_router = http::create_router(user_service_http, room_service_http);

    let http_handle = tokio::spawn(async move {
        let http_addr: std::net::SocketAddr = http_address.parse()
            .expect("Invalid HTTP address");

        let listener = tokio::net::TcpListener::bind(http_addr)
            .await
            .expect("Failed to bind HTTP address");

        info!("HTTP server listening on {}", http_addr);

        if let Err(e) = axum::serve(listener, http_router).await {
            error!("HTTP server error: {}", e);
        }
    });

    // Wait for both servers
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

/// Load JWT service from key files or generate keys for development
fn load_jwt_service(config: &Config) -> Result<JwtService> {
    // Try to load keys from files
    let private_key = std::fs::read(&config.jwt.private_key_path);
    let public_key = std::fs::read(&config.jwt.public_key_path);

    match (private_key, public_key) {
        (Ok(priv_key), Ok(pub_key)) => {
            info!("Loaded JWT keys from files");
            JwtService::new(&priv_key, &pub_key)
                .map_err(|e| anyhow::anyhow!("Failed to initialize JWT service: {}", e))
        }
        _ => {
            // In development, generate temporary keys
            error!("JWT key files not found. Generating temporary keys for development.");
            error!("WARNING: These keys will not persist across restarts!");
            error!("For production, generate keys with: openssl genrsa -out jwt_private.pem 2048");
            error!("                                  openssl rsa -in jwt_private.pem -pubout -out jwt_public.pem");

            // For now, return error - keys must be provided
            Err(anyhow::anyhow!(
                "JWT keys not found at {} and {}. Please generate keys with:\n  openssl genrsa -out {} 2048\n  openssl rsa -in {} -pubout -out {}",
                config.jwt.private_key_path,
                config.jwt.public_key_path,
                config.jwt.private_key_path,
                config.jwt.private_key_path,
                config.jwt.public_key_path
            ))
        }
    }
}
