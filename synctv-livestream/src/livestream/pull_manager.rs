// Pull Stream Manager for lazy-load FLV streaming
//
// Key feature: Create pull streams only when clients request FLV (not on publisher events)
// GOP cache is handled by xiu's StreamHub internally.
//
// NOTE: This manager handles **gRPC relay** pull streams only.
// External pull-to-publish streams are managed by `ExternalPublishManager`.

use crate::{
    relay::registry_trait::StreamRegistryTrait,
    error::StreamResult,
    grpc::GrpcStreamPuller,
    livestream::managed_stream::{ManagedStream, StreamLifecycle, StreamPool},
};
use synctv_xiu::streamhub::define::{StreamHubEvent, StreamHubEventSender};
use synctv_xiu::streamhub::stream::StreamIdentifier;
use tracing::{self as log};
use std::sync::Arc;
use std::time::Duration;

pub struct PullStreamManager {
    pool: StreamPool<PullStream>,
    registry: Arc<dyn StreamRegistryTrait>,
    local_node_id: String,
    stream_hub_event_sender: StreamHubEventSender,
}

impl PullStreamManager {
    pub fn new(
        registry: Arc<dyn StreamRegistryTrait>,
        local_node_id: String,
        stream_hub_event_sender: StreamHubEventSender,
    ) -> Self {
        Self::with_timeouts(registry, local_node_id, stream_hub_event_sender, 60, 300)
    }

    /// Start the background cleanup task for stale creation locks.
    ///
    /// Should be called once after creating the manager to prevent memory leaks
    /// from failed stream creation attempts.
    pub fn start_cleanup_task(&self) -> tokio::task::JoinHandle<()> {
        self.pool.start_creation_lock_cleanup()
    }

    pub fn with_timeouts(
        registry: Arc<dyn StreamRegistryTrait>,
        local_node_id: String,
        stream_hub_event_sender: StreamHubEventSender,
        cleanup_check_interval_secs: u64,
        idle_timeout_secs: u64,
    ) -> Self {
        Self {
            pool: StreamPool::new(
                Duration::from_secs(cleanup_check_interval_secs),
                Duration::from_secs(idle_timeout_secs),
            ),
            registry,
            local_node_id,
            stream_hub_event_sender,
        }
    }

    /// Lazy-load: Get or create pull stream (only triggered by client FLV request)
    ///
    /// Uses double-checked locking to prevent duplicate pull streams for the same key
    /// when multiple viewers request the same stream concurrently.
    pub async fn get_or_create_pull_stream(
        &self,
        room_id: &str,
        media_id: &str,
    ) -> StreamResult<Arc<PullStream>> {
        let stream_key = format!("{room_id}:{media_id}");

        // Fast path: Check if healthy pull stream already exists (no lock needed)
        if let Some(stream) = self.pool.get_existing(&stream_key).await {
            log::debug!(
                "Reusing existing pull stream for {}/{}, subscribers: {}",
                room_id,
                media_id,
                stream.lifecycle().subscriber_count()
            );
            return Ok(stream);
        }

        // Acquire per-key creation lock
        let _guard = self.pool.acquire_creation_lock(&stream_key).await;

        // Re-check after acquiring lock
        if let Some(stream) = self.pool.get_existing(&stream_key).await {
            log::debug!(
                "Reusing pull stream created by concurrent request for {}/{}",
                room_id,
                media_id,
            );
            return Ok(stream);
        }

        // Lazy-load: Create new pull stream on first FLV request
        log::info!(
            "Lazy-load: Creating pull stream for room {} / media {} from publisher",
            room_id,
            media_id
        );

        // Get publisher node address from registry
        let publisher_info = self.registry.get_publisher(room_id, media_id).await
            .map_err(|e| {
                log::error!("Failed to get publisher for {} / {}: {}", room_id, media_id, e);
                crate::error::StreamError::RegistryError(format!("Failed to get publisher: {e}"))
            })?
            .ok_or_else(|| {
                log::warn!("No publisher found for {} / {}", room_id, media_id);
                crate::error::StreamError::NoPublisher(format!("{room_id} / {media_id}"))
            })?;

        // Create pull stream with gRPC puller
        // Store the epoch from publisher info for split-brain detection
        let epoch = publisher_info.epoch;
        let pull_stream = Arc::new(
            PullStream::new(
                room_id.to_string(),
                media_id.to_string(),
                publisher_info.node_id.clone(),
                self.local_node_id.clone(),
                Arc::clone(&self.registry),
                self.stream_hub_event_sender.clone(),
                epoch,
            )
        );

        // Start pull stream (connects via gRPC to publisher)
        pull_stream.start().await?;

        // Initial subscriber
        pull_stream.lifecycle().increment_subscriber_count();

        // Store in pool with idle cleanup (no extra cleanup needed for pull streams)
        self.pool.insert_and_cleanup(
            stream_key,
            pull_stream.clone(),
            |_stream_key| Box::pin(async {}),
        );

        Ok(pull_stream)
    }
}

/// Pull stream instance (pulls RTMP from publisher via gRPC, serves FLV to local clients)
///
/// GOP cache is handled by xiu's `StreamHub` — when the gRPC puller publishes
/// frames to the local `StreamHub`, and a new subscriber joins, `StreamHub`
/// automatically sends cached GOP frames via `send_prior_data`.
pub struct PullStream {
    pub(crate) room_id: String,
    pub(crate) media_id: String,
    pub(crate) publisher_node: String,
    local_node_id: String,
    registry: Arc<dyn StreamRegistryTrait>,
    stream_hub_event_sender: StreamHubEventSender,
    lifecycle: StreamLifecycle,
    /// Fencing token (epoch) from when the stream was created.
    /// Used to detect split-brain when publisher changes during network partition.
    epoch: u64,
}

impl ManagedStream for PullStream {
    fn lifecycle(&self) -> &StreamLifecycle {
        &self.lifecycle
    }

    fn stream_key(&self) -> String {
        format!("{}:{}", self.room_id, self.media_id)
    }
}

impl PullStream {
    pub fn new(
        room_id: String,
        media_id: String,
        publisher_node: String,
        local_node_id: String,
        registry: Arc<dyn StreamRegistryTrait>,
        stream_hub_event_sender: StreamHubEventSender,
        epoch: u64,
    ) -> Self {
        Self {
            room_id,
            media_id,
            publisher_node,
            local_node_id,
            registry,
            stream_hub_event_sender,
            lifecycle: StreamLifecycle::new(),
            epoch,
        }
    }

    /// Start the pull stream - connects to publisher via gRPC
    pub async fn start(&self) -> StreamResult<()> {
        // Validate epoch before starting to detect split-brain
        match self.registry.validate_epoch(&self.room_id, &self.media_id, self.epoch).await {
            Ok(true) => {
                log::debug!(
                    "Epoch {} validated for pull stream {}/{}",
                    self.epoch,
                    self.room_id,
                    self.media_id
                );
            }
            Ok(false) => {
                log::warn!(
                    "Epoch {} is stale for pull stream {}/{}, publisher may have changed. Stopping.",
                    self.epoch,
                    self.room_id,
                    self.media_id
                );
                return Err(crate::error::StreamError::StaleEpoch(format!(
                    "{} / {}",
                    self.room_id, self.media_id
                )));
            }
            Err(e) => {
                log::warn!(
                    "Failed to validate epoch for pull stream {}/{}: {}. Continuing optimistically.",
                    self.room_id,
                    self.media_id,
                    e
                );
                // Continue on error - fail open to avoid blocking streams during Redis issues
            }
        }

        self.lifecycle.set_running();
        self.lifecycle.update_last_active_time().await;

        let room_id = self.room_id.clone();
        let media_id = self.media_id.clone();
        // Clone the is_running flag to mark failure in the spawned task
        let is_running_flag = self.lifecycle.is_running_clone();

        let grpc_puller = GrpcStreamPuller::new(
            self.room_id.clone(),
            self.media_id.clone(),
            self.publisher_node.clone(),
            self.local_node_id.clone(),
            self.stream_hub_event_sender.clone(),
            self.registry.clone(),
        );

        let handle = tokio::spawn(async move {
            log::info!("gRPC puller task started for {} / {}", room_id, media_id);
            let result = grpc_puller.run().await;
            if let Err(ref e) = result {
                log::error!("gRPC puller task failed for {} / {}: {}", room_id, media_id, e);
                // Mark is_running as false so is_healthy() returns false
                // This ensures the stream will be removed from the pool on next access
                is_running_flag.store(false, std::sync::atomic::Ordering::SeqCst);
            }
            result
        });

        self.lifecycle.set_task_handle(handle).await;

        log::info!("Pull stream started for room {} / media {}", self.room_id, self.media_id);
        Ok(())
    }

    /// Stop the pull stream
    ///
    /// Sends `UnPublish` to the local `StreamHub` BEFORE aborting the puller task,
    /// because the puller's own cleanup path won't run on abort.
    pub async fn stop(&self) -> StreamResult<()> {
        self.lifecycle.mark_stopping();

        let stream_name = format!("{}/{}", self.room_id, self.media_id);
        let identifier = StreamIdentifier::Rtmp {
            app_name: "live".to_string(),
            stream_name,
        };
        if let Err(e) = self.stream_hub_event_sender.try_send(StreamHubEvent::UnPublish { identifier }) {
            log::warn!("Failed to send UnPublish to StreamHub for {} / {}: {}", self.room_id, self.media_id, e);
        }

        self.lifecycle.abort_task().await;
        log::info!("Pull stream stopped for room {} / media {}", self.room_id, self.media_id);
        Ok(())
    }

    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.lifecycle.subscriber_count()
    }

    pub fn increment_subscriber_count(&self) {
        self.lifecycle.increment_subscriber_count();
    }

    pub fn decrement_subscriber_count(&self) {
        self.lifecycle.decrement_subscriber_count();
    }

    pub async fn is_healthy(&self) -> bool {
        self.lifecycle.is_healthy().await
    }

    pub async fn last_active_time(&self) -> std::time::Instant {
        self.lifecycle.last_active_time().await
    }

    pub async fn update_last_active_time(&self) {
        self.lifecycle.update_last_active_time().await;
    }

    pub fn mark_stopping(&self) {
        self.lifecycle.mark_stopping();
    }

    pub fn restore_running(&self) {
        self.lifecycle.restore_running();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::MockStreamRegistry;

    #[tokio::test]
    async fn test_pull_stream_manager_creation() {
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::channel(64);

        let manager = PullStreamManager::new(
            registry,
            "node-1".to_string(),
            stream_hub_event_sender,
        );

        assert_eq!(manager.local_node_id, "node-1");
        assert_eq!(manager.pool.streams.len(), 0);
    }

    #[tokio::test]
    async fn test_pull_stream_creation() {
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::channel(64);

        let pull_stream = PullStream::new(
            "room-123".to_string(),
            "media-456".to_string(),
            "publisher-node".to_string(),
            "puller-node".to_string(),
            registry,
            stream_hub_event_sender,
            1, // epoch
        );

        assert_eq!(pull_stream.room_id, "room-123");
        assert_eq!(pull_stream.media_id, "media-456");
        assert_eq!(pull_stream.publisher_node, "publisher-node");
        assert_eq!(pull_stream.epoch, 1);
    }

    #[tokio::test]
    async fn test_subscriber_count() {
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::channel(64);

        let pull_stream = PullStream::new(
            "room-123".to_string(),
            "media-456".to_string(),
            "publisher-node".to_string(),
            "puller-node".to_string(),
            registry,
            stream_hub_event_sender,
            1, // epoch
        );

        assert_eq!(pull_stream.subscriber_count(), 0);

        pull_stream.increment_subscriber_count();
        assert_eq!(pull_stream.subscriber_count(), 1);

        pull_stream.increment_subscriber_count();
        assert_eq!(pull_stream.subscriber_count(), 2);

        pull_stream.decrement_subscriber_count();
        assert_eq!(pull_stream.subscriber_count(), 1);
    }

    #[tokio::test]
    async fn test_stream_key() {
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::channel(64);

        let pull_stream = PullStream::new(
            "room-123".to_string(),
            "media-456".to_string(),
            "publisher-node".to_string(),
            "puller-node".to_string(),
            registry,
            stream_hub_event_sender,
            1, // epoch
        );

        assert_eq!(pull_stream.stream_key(), "room-123:media-456");
    }
}
