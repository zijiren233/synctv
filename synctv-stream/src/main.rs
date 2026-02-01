use synctv_stream::{
    cache::gop_cache::{GopCache, GopCacheConfig},
    relay::registry::StreamRegistry,
    streaming::server::StreamingServer,
    storage::StorageBackend,
};

use anyhow::Result;
use clap::Parser;
use redis::aio::ConnectionManager as RedisConnectionManager;
use std::sync::Arc;
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(name = "synctv-stream")]
#[command(about = "SyncTV Live Streaming Server", long_about = None)]
struct Args {
    /// RTMP listen address
    #[arg(long, env = "RTMP_ADDR", default_value = "0.0.0.0:1935")]
    rtmp_addr: String,

    /// HTTP-FLV listen address
    #[arg(long, env = "HTTPFLV_ADDR", default_value = "0.0.0.0:8080")]
    httpflv_addr: String,

    /// HLS listen address
    #[arg(long, env = "HLS_ADDR", default_value = "0.0.0.0:8081")]
    hls_addr: String,

    /// HLS storage path
    #[arg(long, env = "HLS_STORAGE", default_value = "./hls_storage")]
    hls_storage: String,

    /// Redis connection URL
    #[arg(long, env = "REDIS_URL", default_value = "redis://localhost:6379")]
    redis_url: String,

    /// Enable GOP cache
    #[arg(long, env = "ENABLE_GOP_CACHE", default_value = "true")]
    enable_gop_cache: bool,

    /// Maximum number of GOPs to cache
    #[arg(long, env = "MAX_GOPS", default_value = "2")]
    max_gops: usize,

    /// Maximum GOP cache size in MB
    #[arg(long, env = "MAX_GOP_CACHE_SIZE_MB", default_value = "100")]
    max_gop_cache_size_mb: usize,

    /// Node ID (auto-generated from hostname if not provided)
    #[arg(long, env = "NODE_ID")]
    node_id: Option<String>,

    /// Storage backend (file, memory, oss)
    #[arg(long, env = "STORAGE_BACKEND", default_value = "file")]
    storage_backend: String,

    /// OSS endpoint (required if storage_backend=oss)
    #[arg(long, env = "OSS_ENDPOINT")]
    oss_endpoint: Option<String>,

    /// OSS access key ID (required if storage_backend=oss)
    #[arg(long, env = "OSS_ACCESS_KEY_ID")]
    oss_access_key_id: Option<String>,

    /// OSS secret access key (required if storage_backend=oss)
    #[arg(long, env = "OSS_SECRET_ACCESS_KEY")]
    oss_secret_access_key: Option<String>,

    /// OSS bucket name (required if storage_backend=oss)
    #[arg(long, env = "OSS_BUCKET")]
    oss_bucket: Option<String>,

    /// OSS region (optional, for S3)
    #[arg(long, env = "OSS_REGION")]
    oss_region: Option<String>,

    /// OSS base path prefix (e.g., "hls/")
    #[arg(long, env = "OSS_BASE_PATH", default_value = "")]
    oss_base_path: String,

    /// OSS public URL prefix for CDN (e.g., "https://cdn.example.com/hls/")
    #[arg(long, env = "OSS_PUBLIC_URL")]
    oss_public_url: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .compact()
        .init();

    info!("SyncTV Stream Server starting...");

    // Parse command line arguments
    let args = Args::parse();

    // Determine node ID
    let node_id = args.node_id.unwrap_or_else(|| {
        hostname::get()
            .ok()
            .and_then(|h| h.to_str().map(|s| format!("stream-{}", s)))
            .unwrap_or_else(|| format!("stream-{}", uuid::Uuid::new_v4()))
    });
    info!("Node ID: {}", node_id);

    // Initialize Redis connection
    info!("Connecting to Redis at {}", args.redis_url);
    let redis_client = redis::Client::open(args.redis_url.as_str())?;
    let redis_conn = RedisConnectionManager::new(redis_client).await?;
    info!("Connected to Redis successfully");

    // Initialize stream registry
    let stream_registry = StreamRegistry::new(redis_conn);

    // Initialize GOP cache
    let gop_cache_config = GopCacheConfig {
        max_gops: args.max_gops,
        max_cache_size: args.max_gop_cache_size_mb * 1024 * 1024,
        enabled: args.enable_gop_cache,
    };
    let gop_cache = Arc::new(GopCache::new(gop_cache_config));
    info!(
        "GOP cache initialized (max_gops={}, max_size={}MB, enabled={})",
        args.max_gops, args.max_gop_cache_size_mb, args.enable_gop_cache
    );

    // Initialize storage backend
    let storage_backend = match args.storage_backend.as_str() {
        "file" => {
            info!("Using file storage backend: {}", args.hls_storage);
            StorageBackend::File
        }
        "memory" => {
            info!("Using memory storage backend (data will be lost on restart)");
            StorageBackend::Memory
        }
        "oss" => {
            // Validate OSS configuration
            let endpoint = args.oss_endpoint
                .ok_or_else(|| anyhow::anyhow!("OSS endpoint required when storage_backend=oss"))?;
            let _access_key_id = args.oss_access_key_id
                .ok_or_else(|| anyhow::anyhow!("OSS access key ID required when storage_backend=oss"))?;
            let _secret_access_key = args.oss_secret_access_key
                .ok_or_else(|| anyhow::anyhow!("OSS secret access key required when storage_backend=oss"))?;
            let bucket = args.oss_bucket
                .ok_or_else(|| anyhow::anyhow!("OSS bucket required when storage_backend=oss"))?;
            let _public_url = args.oss_public_url
                .ok_or_else(|| anyhow::anyhow!("OSS public URL required when storage_backend=oss"))?;

            info!("Using OSS storage backend: bucket={}, endpoint={}", bucket, endpoint);
            info!("WARNING: OSS storage is not yet fully implemented");

            StorageBackend::Oss
        }
        other => {
            return Err(anyhow::anyhow!(
                "Invalid storage backend: {}. Valid options: file, memory, oss",
                other
            ));
        }
    };

    // Create streaming server
    let streaming_server = StreamingServer::new(
        args.rtmp_addr.clone(),
        args.httpflv_addr.clone(),
        args.hls_addr.clone(),
        args.hls_storage.clone(),
        storage_backend,
        gop_cache,
        stream_registry,
        node_id.clone(),
    );

    info!("Starting streaming servers:");
    info!("  RTMP:      rtmp://{}", args.rtmp_addr);
    info!("  HTTP-FLV:  http://{}", args.httpflv_addr);
    info!("  HLS:       http://{}", args.hls_addr);

    // Start streaming server
    tokio::select! {
        result = streaming_server.start() => {
            match result {
                Ok(_) => info!("Streaming server stopped gracefully"),
                Err(e) => {
                    error!("Streaming server error: {}", e);
                    return Err(e.into());
                }
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received shutdown signal");
        }
    }

    info!("SyncTV Stream Server shutting down");
    Ok(())
}
