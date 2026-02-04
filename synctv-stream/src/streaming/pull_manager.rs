// Pull Stream Manager for lazy-load FLV streaming
//
// Based on design doc 02-整体架构.md § 2.2.2
// Key feature: Create pull streams only when clients request FLV (not on publisher events)

use crate::{
    libraries::gop_cache::GopCache,
    relay::registry_trait::StreamRegistryTrait,
    error::StreamResult,
    grpc::GrpcStreamPuller,
};
use streamhub::define::StreamHubEventSender;
use tracing as log;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use anyhow::Result;

pub struct PullStreamManager {
    // stream_key -> PullStream
    streams: Arc<DashMap<String, Arc<PullStream>>>,
    gop_cache: Arc<GopCache>,
    registry: Arc<dyn StreamRegistryTrait>,
    local_node_id: String,
    stream_hub_event_sender: StreamHubEventSender,
}

impl PullStreamManager {
    pub fn new(
        gop_cache: Arc<GopCache>,
        registry: Arc<dyn StreamRegistryTrait>,
        local_node_id: String,
        stream_hub_event_sender: StreamHubEventSender,
    ) -> Self {
        Self {
            streams: Arc::new(DashMap::new()),
            gop_cache,
            registry,
            local_node_id,
            stream_hub_event_sender,
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
                Arc::clone(&self.gop_cache),
                Arc::clone(&self.registry),
                self.stream_hub_event_sender.clone(),
            )
        );

        // Start pull stream (connects via gRPC to publisher)
        pull_stream.start().await?;

        // Initial subscriber
        pull_stream.increment_subscriber_count();

        // Register auto-cleanup task (stop after 5 min with no subscribers)
        Self::register_cleanup_task(
            stream_key.clone(),
            Arc::clone(&pull_stream),
            Arc::clone(&self.streams),
        );

        // Store in manager
        self.streams.insert(stream_key.clone(), Arc::clone(&pull_stream));

        Ok(pull_stream)
    }

    /// Register automatic cleanup task (stops pull stream when idle for 5 minutes)
    fn register_cleanup_task(
        stream_key: String,
        pull_stream: Arc<PullStream>,
        streams: Arc<DashMap<String, Arc<PullStream>>>,
    ) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));

            loop {
                interval.tick().await;

                if pull_stream.subscriber_count() == 0 {
                    let idle_time = pull_stream.last_active_time().elapsed();

                    // No subscribers for 5 minutes - stop pull stream
                    if idle_time > Duration::from_secs(300) {
                        log::info!(
                            "Auto cleanup: Stopping pull stream {} (idle for {:?})",
                            stream_key,
                            idle_time
                        );

                        pull_stream.stop().await.ok();
                        streams.remove(&stream_key);
                        break;
                    }
                } else {
                    // Has subscribers - update last active time
                    pull_stream.update_last_active_time();
                }
            }
        });
    }
}

/// Pull stream instance (pulls RTMP from publisher via gRPC, serves FLV to local clients)
pub struct PullStream {
    room_id: String,
    media_id: String,
    publisher_node: String,
    local_node_id: String,
    gop_cache: Arc<GopCache>,
    registry: Arc<dyn StreamRegistryTrait>,
    stream_hub_event_sender: StreamHubEventSender,
    subscriber_count: Arc<RwLock<usize>>,
    last_active: Arc<RwLock<Instant>>,
    is_running: Arc<RwLock<bool>>,
    puller_handle: Arc<Mutex<Option<tokio::task::JoinHandle<Result<()>>>>>,
}

impl PullStream {
    pub fn new(
        room_id: String,
        media_id: String,
        publisher_node: String,
        local_node_id: String,
        gop_cache: Arc<GopCache>,
        registry: Arc<dyn StreamRegistryTrait>,
        stream_hub_event_sender: StreamHubEventSender,
    ) -> Self {
        Self {
            room_id,
            media_id,
            publisher_node,
            local_node_id,
            gop_cache,
            registry,
            stream_hub_event_sender,
            subscriber_count: Arc::new(RwLock::new(0)),
            last_active: Arc::new(RwLock::new(Instant::now())),
            is_running: Arc::new(RwLock::new(false)),
            puller_handle: Arc::new(Mutex::new(None)),
        }
    }

    /// Start the pull stream - connects to publisher via gRPC
    pub async fn start(&self) -> StreamResult<()> {
        *self.is_running.write().await = true;
        self.update_last_active_time();

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
        *self.is_running.write().await = false;

        // Abort the gRPC puller task
        let mut puller_handle = self.puller_handle.lock().await;
        if let Some(handle) = puller_handle.take() {
            handle.abort();
            log::info!("Aborted gRPC puller task for {} / {}", self.room_id, self.media_id);
        }

        log::info!("Pull stream stopped for room {} / media {}", self.room_id, self.media_id);
        Ok(())
    }

    /// Check if the pull stream is healthy (running and receiving data)
    pub async fn is_healthy(&self) -> bool {
        *self.is_running.read().await
    }

    /// Get the current subscriber count
    #[must_use] 
    pub fn subscriber_count(&self) -> usize {
        // Use try_read for non-blocking access
        if let Ok(count) = self.subscriber_count.try_read() {
            *count
        } else {
            0
        }
    }

    /// Increment subscriber count
    pub fn increment_subscriber_count(&self) {
        if let Ok(mut count) = self.subscriber_count.try_write() {
            *count += 1;
        }
    }

    /// Decrement subscriber count
    pub fn decrement_subscriber_count(&self) {
        if let Ok(mut count) = self.subscriber_count.try_write() {
            if *count > 0 {
                *count -= 1;
            }
        }
    }

    /// Get the last active time
    #[must_use] 
    pub fn last_active_time(&self) -> Instant {
        if let Ok(time) = self.last_active.try_read() {
            *time
        } else {
            Instant::now()
        }
    }

    /// Update the last active time
    pub fn update_last_active_time(&self) {
        if let Ok(mut time) = self.last_active.try_write() {
            *time = Instant::now();
        }
    }

    /// Get the stream key for this pull stream
    #[must_use] 
    pub fn stream_key(&self) -> String {
        format!("{}:{}", self.room_id, self.media_id)
    }

    /// Get cached GOP frames for instant playback
    #[must_use] 
    pub fn get_cached_frames(&self) -> Vec<crate::libraries::gop_cache::GopFrame> {
        let stream_key = self.stream_key();
        self.gop_cache.get_frames(&stream_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::libraries::GopCacheConfig;
    use crate::relay::MockStreamRegistry;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_pull_stream_manager_creation() {
        let gop_cache = Arc::new(GopCache::new(GopCacheConfig::default()));
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let manager = PullStreamManager::new(
            gop_cache,
            registry,
            "node-1".to_string(),
            stream_hub_event_sender,
        );

        assert_eq!(manager.local_node_id, "node-1");
        assert_eq!(manager.streams.len(), 0);
    }

    #[tokio::test]
    async fn test_pull_stream_creation() {
        let gop_cache = Arc::new(GopCache::new(GopCacheConfig::default()));
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let pull_stream = PullStream::new(
            "room-123".to_string(),
            "media-456".to_string(),
            "publisher-node".to_string(),
            "puller-node".to_string(),
            gop_cache,
            registry,
            stream_hub_event_sender,
        );

        assert_eq!(pull_stream.room_id, "room-123");
        assert_eq!(pull_stream.media_id, "media-456");
        assert_eq!(pull_stream.publisher_node, "publisher-node");
    }

    #[tokio::test]
    async fn test_subscriber_count() {
        let gop_cache = Arc::new(GopCache::new(GopCacheConfig::default()));
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let pull_stream = PullStream::new(
            "room-123".to_string(),
            "media-456".to_string(),
            "publisher-node".to_string(),
            "puller-node".to_string(),
            gop_cache,
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
        let gop_cache = Arc::new(GopCache::new(GopCacheConfig::default()));
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let pull_stream = PullStream::new(
            "room-123".to_string(),
            "media-456".to_string(),
            "publisher-node".to_string(),
            "puller-node".to_string(),
            gop_cache,
            registry,
            stream_hub_event_sender,
        );

        assert_eq!(pull_stream.stream_key(), "room-123:media-456");
    }
}
