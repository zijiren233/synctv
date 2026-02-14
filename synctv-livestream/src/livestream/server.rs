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
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
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
/// Owns the spawned tasks (`StreamHub` event loop, RTMP server, `PublisherManager`)
/// and exposes the shared infrastructure components.
pub struct LivestreamHandle {
    pub infrastructure: Arc<LiveStreamingInfrastructure>,
    pub pull_manager: Arc<PullStreamManager>,
    hub_handle: JoinHandle<()>,
    rtmp_handle: JoinHandle<()>,
    publisher_manager_handle: JoinHandle<()>,
    pull_manager_cleanup: JoinHandle<()>,
    external_publish_cleanup: JoinHandle<()>,
}

impl LivestreamHandle {
    /// Abort all spawned tasks in reverse startup order.
    ///
    /// This is a fast shutdown that immediately aborts all tasks.
    /// For graceful shutdown that waits for tasks to complete, use `shutdown_graceful`.
    pub fn shutdown(&self) {
        self.external_publish_cleanup.abort();
        self.pull_manager_cleanup.abort();
        self.publisher_manager_handle.abort();
        self.rtmp_handle.abort();
        self.hub_handle.abort();
    }

    /// Gracefully shutdown all spawned tasks.
    ///
    /// This method waits for each task to complete (with timeout) before
    /// proceeding to the next. This ensures proper cleanup of resources.
    ///
    /// # Arguments
    /// * `timeout_secs` - Maximum seconds to wait for each task to complete.
    ///
    /// # Returns
    /// `true` if all tasks shut down gracefully, `false` if any task was aborted due to timeout.
    pub async fn shutdown_graceful(&mut self, timeout_secs: u64) -> bool {
        use tokio::time::{timeout, Duration};
        let timeout_duration = Duration::from_secs(timeout_secs);
        let mut all_graceful = true;

        // Shutdown in reverse startup order
        info!("Starting graceful shutdown of livestream components...");

        // 1. Stop external publish cleanup
        self.external_publish_cleanup.abort();
        if timeout(timeout_duration, &mut self.external_publish_cleanup).await.is_ok() { info!("External publish cleanup stopped") } else {
            warn!("External publish cleanup shutdown timed out");
            all_graceful = false;
        }

        // 2. Stop pull manager cleanup
        self.pull_manager_cleanup.abort();
        if timeout(timeout_duration, &mut self.pull_manager_cleanup).await.is_ok() { info!("Pull manager cleanup stopped") } else {
            warn!("Pull manager cleanup shutdown timed out");
            all_graceful = false;
        }

        // 3. Stop publisher manager
        self.publisher_manager_handle.abort();
        if timeout(timeout_duration, &mut self.publisher_manager_handle).await.is_ok() { info!("Publisher manager stopped") } else {
            warn!("Publisher manager shutdown timed out");
            all_graceful = false;
        }

        // 4. Stop RTMP server
        self.rtmp_handle.abort();
        if timeout(timeout_duration, &mut self.rtmp_handle).await.is_ok() { info!("RTMP server stopped") } else {
            warn!("RTMP server shutdown timed out");
            all_graceful = false;
        }

        // 5. Stop StreamHub (last, as other components depend on it)
        self.hub_handle.abort();
        if timeout(timeout_duration, &mut self.hub_handle).await.is_ok() { info!("StreamHub stopped") } else {
            warn!("StreamHub shutdown timed out");
            all_graceful = false;
        }

        if all_graceful {
            info!("Graceful shutdown completed successfully");
        } else {
            warn!("Graceful shutdown completed with some timeouts");
        }

        all_graceful
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
    /// Creates `StreamHub`, RTMP server, `PullStreamManager`,
    /// `ExternalPublishManager`, `PublisherManager`, and `LiveStreamingInfrastructure`.
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

        // Clone registry for cleanup on StreamHub restart
        let registry_for_cleanup = self.publisher_registry.clone();
        let node_id_for_cleanup = self.config.node_id.clone();

        // Cancellation token for RTMP sessions — cancelled on StreamHub restart
        // to actively terminate all sessions instead of waiting for broken pipe detection.
        // The RTMP server's shutdown_token is a child of this, so cancelling it
        // propagates to the server and all its sessions.
        let rtmp_session_token = CancellationToken::new();
        let rtmp_session_token_for_server = rtmp_session_token.clone();
        let rtmp_session_token_for_hub = rtmp_session_token;

        // 2. Spawn StreamHub event loop with automatic recovery
        let hub_handle = tokio::spawn(async move {
            const MAX_RESTARTS: u32 = 10;
            const INITIAL_BACKOFF_SECS: u64 = 1;
            const MAX_BACKOFF_SECS: u64 = 30;

            let mut restart_count: u32 = 0;

            loop {
                info!("Starting StreamHub event loop...");
                streams_hub.run().await;
                restart_count += 1;
                warn!(
                    restart_count,
                    max_restarts = MAX_RESTARTS,
                    "StreamHub event loop exited unexpectedly, cleaning up local state before restart..."
                );

                // CRITICAL-1: Cancel all active RTMP sessions immediately.
                // Without this, sessions hang waiting for broken pipe detection.
                rtmp_session_token_for_hub.cancel();
                info!("Cancelled all active RTMP sessions due to StreamHub restart");

                // Clean up all local publisher registrations from Redis
                // This ensures stale state doesn't persist after restart
                if let Err(e) = registry_for_cleanup.cleanup_all_publishers_for_node(&node_id_for_cleanup).await {
                    error!("Failed to cleanup publishers on StreamHub restart: {}", e);
                }

                if restart_count >= MAX_RESTARTS {
                    error!(
                        "StreamHub has restarted {} times, giving up to avoid infinite restart loop",
                        restart_count
                    );
                    break;
                }

                // Exponential backoff: 1s, 2s, 4s, 8s, 16s, 30s, 30s, ...
                let backoff_secs = INITIAL_BACKOFF_SECS
                    .saturating_mul(1u64 << (restart_count - 1).min(16))
                    .min(MAX_BACKOFF_SECS);
                info!("Waiting {} seconds before restarting StreamHub...", backoff_secs);
                tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
            }
        });

        // 3. Create and start RTMP server, connected to rtmp_session_token so
        //    cancelling that token (on StreamHub crash) terminates all RTMP sessions.
        let mut rtmp_server = synctv_xiu::rtmp::rtmp::RtmpServer::new(
            self.config.rtmp_address.clone(),
            event_sender.clone(),
            self.config.gop_cache_size,
            self.auth,
        )
        .with_cancellation_token(rtmp_session_token_for_server);
        let rtmp_handle = tokio::spawn(async move {
            if let Err(e) = rtmp_server.run().await {
                error!("RTMP server error: {}", e);
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
        // Start periodic cleanup of stale creation locks to prevent memory leaks
        let pull_manager_cleanup = pull_manager.start_cleanup_task();

        // 5. Create ExternalPublishManager
        let external_publish_manager = Arc::new(ExternalPublishManager::with_timeouts(
            self.publisher_registry.clone(),
            self.config.node_id.clone(),
            event_sender.clone(),
            self.config.cleanup_check_interval_seconds,
            self.config.stream_timeout_seconds,
        ));
        // Start periodic cleanup of stale creation locks to prevent memory leaks
        let external_publish_cleanup = external_publish_manager.start_cleanup_task();

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

        info!(
            "Livestream infrastructure initialized, RTMP server listening on rtmp://{}",
            self.config.rtmp_address,
        );

        Ok(LivestreamHandle {
            infrastructure,
            pull_manager,
            hub_handle,
            rtmp_handle,
            publisher_manager_handle,
            pull_manager_cleanup,
            external_publish_cleanup,
        })
    }
}
