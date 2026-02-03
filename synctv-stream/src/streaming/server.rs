// Complete integration of streaming servers into SyncTV architecture
//
// Architecture:
// 1. Single shared StreamHub (xiu's event bus) for all protocols
// 2. RTMP server for push (creates ServerSessions per connection)
// 3. HLS server for pull (xiu handles HLS transcoding)
// 4. HTTP-FLV server for pull (lazy-load pattern)
// 5. All communicate via StreamHub events
//
// Based on design docs at /Volumes/workspace/rust/synctv-rs-design/02-整体架构.md
// and /Volumes/workspace/rust/synctv-rs-design/17-数据流设计.md

use crate::{
    cache::gop_cache::GopCache,
    relay::registry_trait::StreamRegistryTrait,
    storage::{HlsStorage, StorageBackend, FileStorage, MemoryStorage},
    streaming::{
        pull_manager::PullStreamManager,
        segment_manager::{SegmentManager, CleanupConfig},
        rtmp::RtmpServer,
        // httpflv::HttpFlvServer, // Now integrated into synctv-api as router
        hls::HlsServer,
    },
    error::StreamResult,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing as log;
use streamhub::StreamsHub;

pub struct StreamingServer {
    // Configuration
    rtmp_address: String,
    hls_address: String,
    hls_storage_path: String,
    storage_backend: StorageBackend,

    // Shared components
    gop_cache: Arc<GopCache>,
    registry: Arc<dyn StreamRegistryTrait>,
    node_id: String,
    segment_manager: Option<Arc<SegmentManager>>,
}

impl StreamingServer {
    pub fn new(
        rtmp_address: String,
        hls_address: String,
        hls_storage_path: String,
        storage_backend: StorageBackend,
        gop_cache: Arc<GopCache>,
        registry: Arc<dyn StreamRegistryTrait>,
        node_id: String,
    ) -> Self {
        // Create pull manager for lazy-load FLV
        let _pull_manager = Arc::new(PullStreamManager::new(
            Arc::clone(&gop_cache),
            registry.clone(),
            node_id.clone(),
        ));

        Self {
            rtmp_address,
            hls_address,
            hls_storage_path,
            storage_backend,
            gop_cache,
            registry,
            node_id,
            segment_manager: None,
        }
    }

    pub async fn start(mut self) -> StreamResult<()> {
        // Create shared StreamHub for all streaming protocols
        let stream_hub = Arc::new(Mutex::new(StreamsHub::new(None)));

        // Initialize HLS storage backend
        let storage: Arc<dyn HlsStorage> = match self.storage_backend {
            StorageBackend::File => {
                log::info!("Using file storage backend: {}", self.hls_storage_path);
                Arc::new(FileStorage::new(&self.hls_storage_path))
            }
            StorageBackend::Memory => {
                log::info!("Using memory storage backend (data lost on restart)");
                Arc::new(MemoryStorage::new())
            }
            StorageBackend::Oss => {
                log::warn!("OSS storage not yet fully implemented, falling back to file storage");
                Arc::new(FileStorage::new(&self.hls_storage_path))
            }
        };

        // Create segment manager with default cleanup config
        let cleanup_config = CleanupConfig {
            interval: std::time::Duration::from_secs(10),
            retention: std::time::Duration::from_secs(60),
        };
        let segment_manager = Arc::new(SegmentManager::new(storage, cleanup_config));

        // Start segment cleanup task
        Arc::clone(&segment_manager).start_cleanup_task();
        log::info!("HLS segment cleanup task started");

        // Store segment manager for later use
        self.segment_manager = Some(Arc::clone(&segment_manager));

        // Start RTMP server (creates its own StreamHub internally)
        self.start_rtmp_server().await?;

        // Note: HTTP-FLV is now integrated into synctv-api as a router
        // It will be registered via create_live_router() in synctv-api

        // Start HLS server
        self.start_hls_server(Arc::clone(&stream_hub), segment_manager).await?;

        // Start StreamHub event loop
        tokio::spawn(async move {
            let mut hub = stream_hub.lock().await;
            hub.run().await;
            log::info!("StreamHub event loop ended");
        });

        Ok(())
    }

    async fn start_rtmp_server(&self) -> StreamResult<()> {
        let mut rtmp_server = RtmpServer::new(
            self.rtmp_address.clone(),
            Arc::clone(&self.gop_cache),
            self.registry.clone(),
            self.node_id.clone(),
            2, // gop_num: cache last 2 GOPs
        );

        tokio::spawn(async move {
            if let Err(e) = rtmp_server.start().await {
                log::error!("RTMP server error: {}", e);
            }
        });

        Ok(())
    }

    async fn start_hls_server(
        &self,
        stream_hub: Arc<Mutex<StreamsHub>>,
        segment_manager: Arc<SegmentManager>,
    ) -> StreamResult<()> {
        // Create stream registry for HLS (shared between remuxer and HTTP server)
        let stream_registry = Arc::new(dashmap::DashMap::new());

        let hls_server = HlsServer::new(
            self.hls_address.clone(),
            stream_hub,
            segment_manager,
            stream_registry,
        );

        tokio::spawn(async move {
            if let Err(e) = hls_server.start().await {
                log::error!("HLS server error: {}", e);
            }
        });

        Ok(())
    }
}
