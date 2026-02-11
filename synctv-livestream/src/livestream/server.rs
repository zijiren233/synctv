// Complete integration of livestream servers into SyncTV architecture
//
// Architecture:
// 1. Single shared StreamHub (xiu's event bus) for all protocols
// 2. RTMP server for push (xiu handles protocol, auth via AuthCallback)
// 3. HLS server for pull (xiu handles HLS transcoding)
// 4. HTTP-FLV server for pull (lazy-load pattern)
// 5. All communicate via StreamHub events

use crate::{
    libraries::storage::{HlsStorage, StorageBackend, FileStorage, MemoryStorage, OssStorage, OssConfig},
    relay::registry_trait::StreamRegistryTrait,
    livestream::{
        pull_manager::PullStreamManager,
        segment_manager::{SegmentManager, CleanupConfig},
    },
    protocols::hls::HlsServer,
    error::StreamResult,
};
use synctv_xiu::rtmp::auth::AuthCallback;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing as log;
use synctv_xiu::streamhub::StreamsHub;

pub struct LivestreamServer {
    // Configuration
    rtmp_address: String,
    hls_address: String,
    hls_storage_path: String,
    storage_backend: StorageBackend,
    oss_config: Option<OssConfig>,

    // Shared components
    registry: Arc<dyn StreamRegistryTrait>,
    node_id: String,
    segment_manager: Option<Arc<SegmentManager>>,
    auth: Option<Arc<dyn AuthCallback>>,
}

impl LivestreamServer {
    pub fn new(
        rtmp_address: String,
        hls_address: String,
        hls_storage_path: String,
        storage_backend: StorageBackend,
        registry: Arc<dyn StreamRegistryTrait>,
        node_id: String,
    ) -> Self {
        Self {
            rtmp_address,
            hls_address,
            hls_storage_path,
            storage_backend,
            oss_config: None,
            registry,
            node_id,
            segment_manager: None,
            auth: None,
        }
    }

    /// Set OSS configuration for object storage backend
    #[must_use]
    pub fn with_oss_config(mut self, config: OssConfig) -> Self {
        self.oss_config = Some(config);
        self
    }

    /// Set RTMP auth callback
    #[must_use]
    pub fn with_auth(mut self, auth: Arc<dyn AuthCallback>) -> Self {
        self.auth = Some(auth);
        self
    }

    pub async fn start(&mut self) -> StreamResult<()> {
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
                if let Some(oss_config) = self.oss_config.take() {
                    log::info!(
                        "Using OSS storage backend: bucket={}, endpoint={}",
                        oss_config.bucket,
                        oss_config.endpoint
                    );
                    match OssStorage::new(oss_config) {
                        Ok(oss) => Arc::new(oss),
                        Err(e) => {
                            log::error!("Failed to initialize OSS storage: {}, falling back to file storage", e);
                            Arc::new(FileStorage::new(&self.hls_storage_path))
                        }
                    }
                } else {
                    log::warn!("OSS storage selected but no config provided, falling back to file storage");
                    Arc::new(FileStorage::new(&self.hls_storage_path))
                }
            }
        };

        // Create segment manager with default cleanup config
        let cleanup_config = CleanupConfig {
            interval: std::time::Duration::from_secs(10),
            retention: std::time::Duration::from_mins(1),
        };
        let segment_manager = Arc::new(SegmentManager::new(storage, cleanup_config));

        // Start segment cleanup task
        Arc::clone(&segment_manager).start_cleanup_task();
        log::info!("HLS segment cleanup task started");

        // Store segment manager for later use
        self.segment_manager = Some(Arc::clone(&segment_manager));

        // Create StreamHub channels and hub
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        let stream_hub = Arc::new(Mutex::new(StreamsHub::new(
            event_sender.clone(),
            event_receiver,
        )));

        // Start RTMP server with the event sender
        self.start_rtmp_server(event_sender.clone()).await?;

        // Create PullStreamManager with the event sender from StreamHub
        let _pull_manager = Arc::new(PullStreamManager::new(
            self.registry.clone(),
            self.node_id.clone(),
            event_sender,
        ));

        // Start HLS server
        self.start_hls_server(Arc::clone(&stream_hub), segment_manager).await?;

        // Start StreamHub event loop
        let hub_clone = Arc::clone(&stream_hub);
        tokio::spawn(async move {
            let mut hub = hub_clone.lock().await;
            hub.run().await;
            log::info!("StreamHub event loop ended");
        });

        Ok(())
    }

    async fn start_rtmp_server(
        &self,
        event_sender: synctv_xiu::streamhub::define::StreamHubEventSender,
    ) -> StreamResult<()> {
        let auth = self.auth.clone();
        let mut xiu_rtmp_server = synctv_xiu::rtmp::rtmp::RtmpServer::new(
            self.rtmp_address.clone(),
            event_sender,
            2, // gop_num
            auth,
        );

        tokio::spawn(async move {
            if let Err(e) = xiu_rtmp_server.run().await {
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
