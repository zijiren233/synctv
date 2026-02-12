// External Publish Manager
//
// Manages external pull-to-publish streams (RTMP / HTTP-FLV → local StreamHub).
//
// From the system's perspective this is a **publisher**: frames are pushed into
// the local StreamHub and the stream is registered in Redis so other nodes can
// discover and relay it via gRPC.  The lifecycle mirrors PullStreamManager
// (lazy start on first viewer, idle cleanup after 5 min) but the two concerns
// are kept separate because external publish owns Redis registration/cleanup.

use crate::{
    error::StreamResult,
    livestream::external_puller::ExternalStreamPuller,
    relay::registry_trait::StreamRegistryTrait,
};
use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use std::time::{Duration, Instant};
use synctv_xiu::streamhub::define::{StreamHubEvent, StreamHubEventSender};
use synctv_xiu::streamhub::stream::StreamIdentifier;
use tokio::sync::Mutex;
use tracing::{self as log, Instrument};

/// Manages external pull-to-publish streams.
///
/// Each stream is lazily started on the first viewer request.  The manager
/// deduplicates concurrent requests (one puller per `room_id:media_id`),
/// registers the stream as a publisher in Redis, and automatically stops +
/// unregisters after 5 minutes with no subscribers.
pub struct ExternalPublishManager {
    streams: Arc<DashMap<String, Arc<ExternalPublishStream>>>,
    /// Per-key creation locks to prevent concurrent creation of the same stream
    creation_locks: Arc<DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    registry: Arc<dyn StreamRegistryTrait>,
    local_node_id: String,
    stream_hub_event_sender: StreamHubEventSender,
    cleanup_check_interval: Duration,
    idle_timeout: Duration,
}

impl ExternalPublishManager {
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
            creation_locks: Arc::new(DashMap::new()),
            registry,
            local_node_id,
            stream_hub_event_sender,
            cleanup_check_interval: Duration::from_secs(cleanup_check_interval_secs),
            idle_timeout: Duration::from_secs(idle_timeout_secs),
        }
    }

    /// Get or create an external publish stream.
    ///
    /// If a healthy stream already exists for this `(room_id, media_id)` pair,
    /// the subscriber count is incremented and the existing stream is returned.
    /// Otherwise a new `ExternalStreamPuller` is spawned and the stream is
    /// registered as a publisher in Redis.
    pub async fn get_or_create(
        &self,
        room_id: &str,
        media_id: &str,
        source_url: &str,
    ) -> StreamResult<Arc<ExternalPublishStream>> {
        let stream_key = format!("{room_id}:{media_id}");

        // Fast path: Reuse healthy existing stream (no lock needed)
        if let Some(stream) = self.streams.get(&stream_key) {
            if stream.is_healthy().await {
                stream.increment_subscriber_count();
                stream.update_last_active_time().await;
                return Ok(stream.clone());
            }
            drop(stream);
            self.streams.remove(&stream_key);
        }

        // Acquire per-key creation lock to prevent concurrent creation for the same stream
        let lock = self.creation_locks
            .entry(stream_key.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        // Re-check after acquiring lock (another task may have created it while we waited)
        if let Some(stream) = self.streams.get(&stream_key) {
            if stream.is_healthy().await {
                log::debug!(
                    "Reusing external publish stream created by concurrent request for {}/{}",
                    room_id,
                    media_id,
                );
                stream.increment_subscriber_count();
                stream.update_last_active_time().await;
                return Ok(stream.clone());
            }
            drop(stream);
            self.streams.remove(&stream_key);
        }

        log::info!(
            "Lazy-load: Creating external publish stream for {}/{} from {}",
            room_id,
            media_id,
            source_url
        );

        let stream = Arc::new(ExternalPublishStream::new(
            room_id.to_string(),
            media_id.to_string(),
            source_url.to_string(),
            self.stream_hub_event_sender.clone(),
        ));

        // Start the puller (pushes frames into local StreamHub)
        stream.start().await?;

        // Register as publisher in Redis so other nodes can discover this stream
        if let Err(e) = self
            .registry
            .try_register_publisher(room_id, media_id, &self.local_node_id, "external_puller")
            .await
        {
            log::error!("Failed to register external publisher in Redis, rolling back: {e}");
            stream.stop().await.ok();
            return Err(crate::error::StreamError::RegistryError(
                format!("Failed to register publisher in Redis: {e}"),
            ));
        }

        stream.increment_subscriber_count();

        // Spawn idle-cleanup task
        Self::register_cleanup_task(
            stream_key.clone(),
            Arc::clone(&stream),
            Arc::clone(&self.streams),
            Arc::clone(&self.registry),
            self.local_node_id.clone(),
            self.cleanup_check_interval,
            self.idle_timeout,
        );

        self.streams.insert(stream_key, Arc::clone(&stream));

        Ok(stream)
    }

    /// Automatic cleanup: stops the puller and unregisters from Redis when idle.
    fn register_cleanup_task(
        stream_key: String,
        stream: Arc<ExternalPublishStream>,
        streams: Arc<DashMap<String, Arc<ExternalPublishStream>>>,
        registry: Arc<dyn StreamRegistryTrait>,
        local_node_id: String,
        check_interval: Duration,
        idle_timeout: Duration,
    ) {
        let span = tracing::info_span!("ext_publish_cleanup", stream_key = %stream_key);
        tokio::spawn(async move {
            let result = Self::cleanup_loop(&stream_key, &stream, &streams, &registry, &local_node_id, check_interval, idle_timeout).await;
            if let Err(e) = result {
                log::error!("Cleanup task failed for external publish stream {}: {}", stream_key, e);
                stream.stop().await.ok();
                streams.remove(&stream_key);
            }
        }.instrument(span));
    }

    async fn cleanup_loop(
        stream_key: &str,
        stream: &Arc<ExternalPublishStream>,
        streams: &Arc<DashMap<String, Arc<ExternalPublishStream>>>,
        registry: &Arc<dyn StreamRegistryTrait>,
        local_node_id: &str,
        check_interval: Duration,
        idle_timeout: Duration,
    ) -> Result<()> {
        let mut interval = tokio::time::interval(check_interval);

        loop {
            interval.tick().await;

            if stream.subscriber_count() == 0 {
                let idle_time = stream.last_active_time().await.elapsed();

                if idle_time > idle_timeout {
                    log::info!(
                        "Auto cleanup: Stopping external publish stream {} (idle for {:?})",
                        stream_key,
                        idle_time
                    );

                    // Unregister from Redis FIRST so other nodes stop routing
                    // viewers to us before we tear down the local stream.
                    if let Some((room_id, media_id)) = stream_key.split_once(':') {
                        match registry.get_publisher(room_id, media_id).await {
                            Ok(Some(info)) if info.node_id == local_node_id => {
                                if let Err(e) = registry.unregister_publisher(room_id, media_id).await {
                                    log::warn!("Failed to unregister external publisher from Redis: {e}");
                                }
                            }
                            Ok(Some(_)) => {
                                log::info!("Skipping Redis unregister for {} — publisher owned by another node", stream_key);
                            }
                            _ => {}
                        }
                    }

                    // Now stop the local stream after Redis no longer advertises it
                    if let Err(e) = stream.stop().await {
                        log::error!("Failed to stop external publish stream {}: {}", stream_key, e);
                    }

                    streams.remove(stream_key);
                    break;
                }
            } else {
                stream.update_last_active_time().await;
            }
        }
        Ok(())
    }
}

/// A single external publish stream instance.
///
/// Pulls from an external RTMP or HTTP-FLV source and publishes frames into
/// the local `StreamHub` under `live/{room_id}/{media_id}`.
pub struct ExternalPublishStream {
    room_id: String,
    media_id: String,
    source_url: String,
    stream_hub_event_sender: StreamHubEventSender,
    subscriber_count: AtomicUsize,
    last_active: Mutex<Instant>,
    is_running: AtomicBool,
    task_handle: Mutex<Option<tokio::task::JoinHandle<Result<()>>>>,
}

impl ExternalPublishStream {
    #[must_use] 
    pub fn new(
        room_id: String,
        media_id: String,
        source_url: String,
        stream_hub_event_sender: StreamHubEventSender,
    ) -> Self {
        Self {
            room_id,
            media_id,
            source_url,
            stream_hub_event_sender,
            subscriber_count: AtomicUsize::new(0),
            last_active: Mutex::new(Instant::now()),
            is_running: AtomicBool::new(false),
            task_handle: Mutex::new(None),
        }
    }

    /// Start the external puller task.
    pub async fn start(&self) -> StreamResult<()> {
        self.is_running.store(true, Ordering::SeqCst);
        self.update_last_active_time().await;

        let room_id = self.room_id.clone();
        let media_id = self.media_id.clone();
        let source_url = self.source_url.clone();
        let stream_hub_sender = self.stream_hub_event_sender.clone();

        let handle = tokio::spawn(async move {
            log::info!("External publish task started for {}/{}", room_id, media_id);
            let puller = ExternalStreamPuller::new(
                room_id.clone(),
                media_id.clone(),
                source_url,
                stream_hub_sender,
            )?;
            let result = puller.run().await;
            if let Err(ref e) = result {
                log::error!(
                    "External publish task failed for {}/{}: {}",
                    room_id,
                    media_id,
                    e
                );
            }
            result
        });

        *self.task_handle.lock().await = Some(handle);
        log::info!(
            "External publish stream started for {}/{}",
            self.room_id,
            self.media_id
        );
        Ok(())
    }

    /// Stop the external puller task.
    ///
    /// Sends `UnPublish` to the local `StreamHub` BEFORE aborting, since the
    /// puller's own cleanup path won't run on abort.
    pub async fn stop(&self) -> StreamResult<()> {
        self.is_running.store(false, Ordering::SeqCst);

        // Clean up the local StreamHub publisher before aborting
        let stream_name = format!("{}/{}", self.room_id, self.media_id);
        let identifier = StreamIdentifier::Rtmp {
            app_name: "live".to_string(),
            stream_name,
        };
        if let Err(e) = self.stream_hub_event_sender.send(StreamHubEvent::UnPublish { identifier }) {
            log::warn!("Failed to send UnPublish for {}/{}: {}", self.room_id, self.media_id, e);
        }

        if let Some(handle) = self.task_handle.lock().await.take() {
            handle.abort();
            log::info!(
                "Aborted external publish task for {}/{}",
                self.room_id,
                self.media_id
            );
        }
        Ok(())
    }

    pub async fn is_healthy(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.subscriber_count.load(Ordering::SeqCst)
    }

    pub fn increment_subscriber_count(&self) {
        self.subscriber_count.fetch_add(1, Ordering::SeqCst);
    }

    pub fn decrement_subscriber_count(&self) {
        let result = self.subscriber_count.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
            if v > 0 { Some(v - 1) } else { None }
        });
        if result.is_err() {
            tracing::warn!("Attempted to decrement subscriber count below zero");
        }
    }

    pub async fn last_active_time(&self) -> Instant {
        *self.last_active.lock().await
    }

    pub async fn update_last_active_time(&self) {
        *self.last_active.lock().await = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::MockStreamRegistry;

    #[tokio::test]
    async fn test_external_publish_manager_creation() {
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;
        let (sender, _) = tokio::sync::mpsc::unbounded_channel();

        let manager = ExternalPublishManager::new(registry, "node-1".to_string(), sender);
        assert_eq!(manager.streams.len(), 0);
    }

    #[tokio::test]
    async fn test_external_publish_stream_subscriber_count() {
        let (sender, _) = tokio::sync::mpsc::unbounded_channel();
        let stream = ExternalPublishStream::new(
            "room-1".to_string(),
            "media-1".to_string(),
            "rtmp://example.com/live/stream".to_string(),
            sender,
        );

        assert_eq!(stream.subscriber_count(), 0);
        stream.increment_subscriber_count();
        assert_eq!(stream.subscriber_count(), 1);
        stream.decrement_subscriber_count();
        assert_eq!(stream.subscriber_count(), 0);
    }
}
