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
};
use synctv_xiu::streamhub::define::StreamHubEventSender;
use tracing::{self as log, Instrument};
use dashmap::DashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use anyhow::Result;

pub struct PullStreamManager {
    // stream_key -> PullStream
    streams: Arc<DashMap<String, Arc<PullStream>>>,
    registry: Arc<dyn StreamRegistryTrait>,
    local_node_id: String,
    stream_hub_event_sender: StreamHubEventSender,
    cleanup_check_interval: Duration,
    idle_timeout: Duration,
}

impl PullStreamManager {
    pub fn new(
        registry: Arc<dyn StreamRegistryTrait>,
        local_node_id: String,
        stream_hub_event_sender: StreamHubEventSender,
    ) -> Self {
        Self::with_timeouts(registry, local_node_id, stream_hub_event_sender, 60, 300)
    }

    pub fn with_timeouts(
        registry: Arc<dyn StreamRegistryTrait>,
        local_node_id: String,
        stream_hub_event_sender: StreamHubEventSender,
        cleanup_check_interval_secs: u64,
        idle_timeout_secs: u64,
    ) -> Self {
        Self {
            streams: Arc::new(DashMap::new()),
            registry,
            local_node_id,
            stream_hub_event_sender,
            cleanup_check_interval: Duration::from_secs(cleanup_check_interval_secs),
            idle_timeout: Duration::from_secs(idle_timeout_secs),
        }
    }

    /// Lazy-load: Get or create pull stream (only triggered by client FLV request)
    pub async fn get_or_create_pull_stream(
        &self,
        room_id: &str,
        media_id: &str,
    ) -> StreamResult<Arc<PullStream>> {
        let stream_key = format!("{room_id}:{media_id}");

        // Check if healthy pull stream already exists
        if let Some(stream) = self.streams.get(&stream_key) {
            if stream.is_healthy().await {
                log::debug!(
                    "Reusing existing pull stream for {}/{}, subscribers: {}",
                    room_id,
                    media_id,
                    stream.subscriber_count()
                );
                stream.increment_subscriber_count();
                stream.update_last_active_time().await;
                return Ok(stream.clone());
            }
            // Remove unhealthy stream
            self.streams.remove(&stream_key);
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
        let pull_stream = Arc::new(
            PullStream::new(
                room_id.to_string(),
                media_id.to_string(),
                publisher_info.node_id.clone(),
                self.local_node_id.clone(),
                Arc::clone(&self.registry),
                self.stream_hub_event_sender.clone(),
            )
        );

        // Start pull stream (connects via gRPC to publisher)
        pull_stream.start().await?;

        // Initial subscriber
        pull_stream.increment_subscriber_count();

        // Register auto-cleanup task
        Self::register_cleanup_task(
            stream_key.clone(),
            Arc::clone(&pull_stream),
            Arc::clone(&self.streams),
            self.cleanup_check_interval,
            self.idle_timeout,
        );

        // Store in manager
        self.streams.insert(stream_key.clone(), Arc::clone(&pull_stream));

        Ok(pull_stream)
    }

    /// Register automatic cleanup task (stops pull stream when idle)
    fn register_cleanup_task(
        stream_key: String,
        pull_stream: Arc<PullStream>,
        streams: Arc<DashMap<String, Arc<PullStream>>>,
        check_interval: Duration,
        idle_timeout: Duration,
    ) {
        let span = tracing::info_span!("pull_cleanup", stream_key = %stream_key);
        tokio::spawn(async move {
            let result = Self::cleanup_loop(&stream_key, &pull_stream, &streams, check_interval, idle_timeout).await;
            if let Err(e) = result {
                log::error!("Cleanup task panicked for {}: {}", stream_key, e);
                pull_stream.stop().await.ok();
                streams.remove(&stream_key);
            }
        }.instrument(span));
    }

    async fn cleanup_loop(
        stream_key: &str,
        pull_stream: &Arc<PullStream>,
        streams: &Arc<DashMap<String, Arc<PullStream>>>,
        check_interval: Duration,
        idle_timeout: Duration,
    ) -> Result<()> {
        let mut interval = tokio::time::interval(check_interval);

        loop {
            interval.tick().await;

            if pull_stream.subscriber_count() == 0 {
                let idle_time = pull_stream.last_active_time().await.elapsed();

                if idle_time > idle_timeout {
                    log::info!(
                        "Auto cleanup: Stopping pull stream {} (idle for {:?})",
                        stream_key,
                        idle_time
                    );

                    if let Err(e) = pull_stream.stop().await {
                        log::error!("Failed to stop pull stream {}: {}", stream_key, e);
                    }
                    streams.remove(stream_key);
                    break;
                }
            } else {
                pull_stream.update_last_active_time().await;
            }
        }
        Ok(())
    }
}

/// Pull stream instance (pulls RTMP from publisher via gRPC, serves FLV to local clients)
///
/// GOP cache is handled by xiu's `StreamHub` — when the gRPC puller publishes
/// frames to the local `StreamHub`, and a new subscriber joins, `StreamHub`
/// automatically sends cached GOP frames via `send_prior_data`.
pub struct PullStream {
    room_id: String,
    media_id: String,
    publisher_node: String,
    local_node_id: String,
    registry: Arc<dyn StreamRegistryTrait>,
    stream_hub_event_sender: StreamHubEventSender,
    subscriber_count: AtomicUsize,
    last_active: Mutex<Instant>,
    is_running: AtomicBool,
    puller_handle: Mutex<Option<tokio::task::JoinHandle<Result<()>>>>,
}

impl PullStream {
    pub fn new(
        room_id: String,
        media_id: String,
        publisher_node: String,
        local_node_id: String,
        registry: Arc<dyn StreamRegistryTrait>,
        stream_hub_event_sender: StreamHubEventSender,
    ) -> Self {
        Self {
            room_id,
            media_id,
            publisher_node,
            local_node_id,
            registry,
            stream_hub_event_sender,
            subscriber_count: AtomicUsize::new(0),
            last_active: Mutex::new(Instant::now()),
            is_running: AtomicBool::new(false),
            puller_handle: Mutex::new(None),
        }
    }

    /// Start the pull stream - connects to publisher via gRPC
    pub async fn start(&self) -> StreamResult<()> {
        self.is_running.store(true, Ordering::SeqCst);
        self.update_last_active_time().await;

        // Clone values before moving into async block
        let room_id = self.room_id.clone();
        let media_id = self.media_id.clone();

        // Create gRPC puller
        let grpc_puller = GrpcStreamPuller::new(
            self.room_id.clone(),
            self.media_id.clone(),
            self.publisher_node.clone(),
            self.local_node_id.clone(),
            self.stream_hub_event_sender.clone(),
            self.registry.clone(),
        );

        // Spawn gRPC puller task in background
        let handle = tokio::spawn(async move {
            log::info!("gRPC puller task started for {} / {}", room_id, media_id);
            let result = grpc_puller.run().await;
            if let Err(ref e) = result {
                log::error!("gRPC puller task failed for {} / {}: {}", room_id, media_id, e);
            }
            result
        });

        // Store the handle
        let mut puller_handle = self.puller_handle.lock().await;
        *puller_handle = Some(handle);

        log::info!("Pull stream started for room {} / media {}", self.room_id, self.media_id);
        Ok(())
    }

    /// Stop the pull stream
    pub async fn stop(&self) -> StreamResult<()> {
        self.is_running.store(false, Ordering::SeqCst);

        // Abort the gRPC puller task and await to ensure cleanup
        let mut puller_handle = self.puller_handle.lock().await;
        if let Some(handle) = puller_handle.take() {
            handle.abort();
            let _ = handle.await;
            log::info!("Aborted gRPC puller task for {} / {}", self.room_id, self.media_id);
        }

        log::info!("Pull stream stopped for room {} / media {}", self.room_id, self.media_id);
        Ok(())
    }

    /// Check if the pull stream is healthy (running and receiving data)
    pub async fn is_healthy(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    /// Get the current subscriber count
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.subscriber_count.load(Ordering::SeqCst)
    }

    /// Increment subscriber count
    pub fn increment_subscriber_count(&self) {
        self.subscriber_count.fetch_add(1, Ordering::SeqCst);
    }

    /// Decrement subscriber count
    pub fn decrement_subscriber_count(&self) {
        // Use fetch_update to avoid underflow
        let _ = self.subscriber_count.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
            if v > 0 { Some(v - 1) } else { None }
        });
    }

    /// Get the last active time
    pub async fn last_active_time(&self) -> Instant {
        *self.last_active.lock().await
    }

    /// Update the last active time
    pub async fn update_last_active_time(&self) {
        *self.last_active.lock().await = Instant::now();
    }

    /// Get the stream key for this pull stream
    #[must_use]
    pub fn stream_key(&self) -> String {
        format!("{}:{}", self.room_id, self.media_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::MockStreamRegistry;

    #[tokio::test]
    async fn test_pull_stream_manager_creation() {
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let manager = PullStreamManager::new(
            registry,
            "node-1".to_string(),
            stream_hub_event_sender,
        );

        assert_eq!(manager.local_node_id, "node-1");
        assert_eq!(manager.streams.len(), 0);
    }

    #[tokio::test]
    async fn test_pull_stream_creation() {
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let pull_stream = PullStream::new(
            "room-123".to_string(),
            "media-456".to_string(),
            "publisher-node".to_string(),
            "puller-node".to_string(),
            registry,
            stream_hub_event_sender,
        );

        assert_eq!(pull_stream.room_id, "room-123");
        assert_eq!(pull_stream.media_id, "media-456");
        assert_eq!(pull_stream.publisher_node, "publisher-node");
    }

    #[tokio::test]
    async fn test_subscriber_count() {
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let pull_stream = PullStream::new(
            "room-123".to_string(),
            "media-456".to_string(),
            "publisher-node".to_string(),
            "puller-node".to_string(),
            registry,
            stream_hub_event_sender,
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
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let pull_stream = PullStream::new(
            "room-123".to_string(),
            "media-456".to_string(),
            "publisher-node".to_string(),
            "puller-node".to_string(),
            registry,
            stream_hub_event_sender,
        );

        assert_eq!(pull_stream.stream_key(), "room-123:media-456");
    }
}
