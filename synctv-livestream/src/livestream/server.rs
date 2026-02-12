// Livestream server facade
//
// Single entry point for starting the entire livestream infrastructure:
// StreamHub, RTMP server, PullStreamManager, ExternalPublishManager,
// PublisherManager, and LiveStreamingInfrastructure.
//
// The synctv binary never touches synctv_xiu directly — all xiu interaction
// is encapsulated here.

use crate::{
    relay::{registry_trait::StreamRegistryTrait, PublisherManager},
    livestream::{
        pull_manager::PullStreamManager,
        external_publish_manager::ExternalPublishManager,
    },
    api::{LiveStreamingInfrastructure, UserStreamTracker},
    error::StreamResult,
};
use synctv_xiu::rtmp::auth::AuthCallback;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing as log;
use synctv_xiu::streamhub::StreamsHub;

pub struct LivestreamConfig {
    pub rtmp_address: String,
    pub gop_cache_size: usize,
    pub node_id: String,
    pub cleanup_check_interval_seconds: u64,
    pub stream_timeout_seconds: u64,
}

/// Handle returned by [`LivestreamServer::start`].
///
/// Owns the spawned tasks (StreamHub event loop, RTMP server, PublisherManager)
/// and exposes the shared infrastructure components.
pub struct LivestreamHandle {
    pub infrastructure: Arc<LiveStreamingInfrastructure>,
    pub pull_manager: Arc<PullStreamManager>,
    hub_handle: JoinHandle<()>,
    rtmp_handle: JoinHandle<()>,
    publisher_manager_handle: JoinHandle<()>,
}

impl LivestreamHandle {
    /// Abort all spawned tasks in reverse startup order.
    pub fn shutdown(&self) {
        self.publisher_manager_handle.abort();
        self.rtmp_handle.abort();
        self.hub_handle.abort();
    }
}

pub struct LivestreamServer {
    config: LivestreamConfig,
    publisher_registry: Arc<dyn StreamRegistryTrait>,
    user_stream_tracker: UserStreamTracker,
    auth: Option<Arc<dyn AuthCallback>>,
}

impl LivestreamServer {
    pub fn new(
        config: LivestreamConfig,
        publisher_registry: Arc<dyn StreamRegistryTrait>,
        user_stream_tracker: UserStreamTracker,
    ) -> Self {
        Self {
            config,
            publisher_registry,
            user_stream_tracker,
            auth: None,
        }
    }

    /// Set RTMP auth callback
    #[must_use]
    pub fn with_auth(mut self, auth: Arc<dyn AuthCallback>) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Start the entire livestream infrastructure.
    ///
    /// Creates StreamHub, RTMP server, PullStreamManager,
    /// ExternalPublishManager, PublisherManager, and LiveStreamingInfrastructure.
    /// Returns a handle with public components.
    pub async fn start(self) -> StreamResult<LivestreamHandle> {
        // 1. Create StreamHub channels and hub (bounded to prevent OOM under load)
        let (event_sender, event_receiver) =
            mpsc::channel(synctv_xiu::streamhub::define::STREAM_HUB_EVENT_CHANNEL_CAPACITY);
        let mut streams_hub = StreamsHub::new(
            event_sender.clone(),
            event_receiver,
        );

        // Get broadcast receiver BEFORE spawning the hub
        let broadcast_receiver = streams_hub.get_client_event_consumer();

        // 2. Spawn StreamHub event loop
        let hub_handle = tokio::spawn(async move {
            streams_hub.run().await;
            log::info!("StreamHub event loop ended");
        });

        // 3. Create and start RTMP server
        let mut rtmp_server = synctv_xiu::rtmp::rtmp::RtmpServer::new(
            self.config.rtmp_address.clone(),
            event_sender.clone(),
            self.config.gop_cache_size,
            self.auth,
        );
        let rtmp_handle = tokio::spawn(async move {
            if let Err(e) = rtmp_server.run().await {
                log::error!("RTMP server error: {}", e);
            }
        });

        // 4. Create PullStreamManager
        let pull_manager = Arc::new(PullStreamManager::with_timeouts(
            self.publisher_registry.clone(),
            self.config.node_id.clone(),
            event_sender.clone(),
            self.config.cleanup_check_interval_seconds,
            self.config.stream_timeout_seconds,
        ));

        // 5. Create ExternalPublishManager
        let external_publish_manager = Arc::new(ExternalPublishManager::with_timeouts(
            self.publisher_registry.clone(),
            self.config.node_id.clone(),
            event_sender.clone(),
            self.config.cleanup_check_interval_seconds,
            self.config.stream_timeout_seconds,
        ));

        // 6. Start PublisherManager — listens to StreamHub broadcast events
        // and registers/unregisters publishers in Redis for multi-node relay
        let publisher_manager = Arc::new(PublisherManager::new(
            self.publisher_registry.clone(),
            self.config.node_id,
        ));
        let publisher_manager_handle = tokio::spawn({
            let pm = Arc::clone(&publisher_manager);
            async move {
                pm.start(broadcast_receiver).await;
            }
        });

        // 7. Create LiveStreamingInfrastructure
        let infrastructure = Arc::new(LiveStreamingInfrastructure::new(
            self.publisher_registry,
            event_sender,
            pull_manager.clone(),
            external_publish_manager,
            self.user_stream_tracker,
        ));

        log::info!(
            "Livestream infrastructure initialized, RTMP server listening on rtmp://{}",
            self.config.rtmp_address,
        );

        Ok(LivestreamHandle {
            infrastructure,
            pull_manager,
            hub_handle,
            rtmp_handle,
            publisher_manager_handle,
        })
    }
}
