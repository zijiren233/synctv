// Pull Stream Manager for lazy-load FLV streaming
//
// Based on design doc 02-整体架构.md § 2.2.2
// Key feature: Create pull streams only when clients request FLV (not on publisher events)

use crate::{
    cache::gop_cache::GopCache,
    relay::registry::StreamRegistry,
    error::StreamResult,
};
use tracing as log;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub struct PullStreamManager {
    // stream_key -> PullStream
    streams: Arc<DashMap<String, Arc<PullStream>>>,
    gop_cache: Arc<GopCache>,
    registry: StreamRegistry,
    local_node_id: String,
}

impl PullStreamManager {
    pub fn new(
        gop_cache: Arc<GopCache>,
        registry: StreamRegistry,
        local_node_id: String,
    ) -> Self {
        Self {
            streams: Arc::new(DashMap::new()),
            gop_cache,
            registry,
            local_node_id,
        }
    }

    /// Lazy-load: Get or create pull stream (only triggered by client FLV request)
    pub async fn get_or_create_pull_stream(
        &self,
        room_id: &str,
        publisher_node: &str,
    ) -> StreamResult<Arc<PullStream>> {
        let stream_key = format!("room_{}", room_id);

        // Check if healthy pull stream already exists
        if let Some(stream) = self.streams.get(&stream_key) {
            if stream.is_healthy().await {
                log::debug!(
                    "Reusing existing pull stream for room {}, subscribers: {}",
                    room_id,
                    stream.subscriber_count()
                );
                return Ok(stream.clone());
            } else {
                // Remove unhealthy stream
                self.streams.remove(&stream_key);
            }
        }

        // Lazy-load: Create new pull stream on first FLV request
        log::info!(
            "Lazy-load: Creating pull stream for room {} from publisher {}",
            room_id,
            publisher_node
        );

        // TODO: Create RTMP client connection to publisher
        // let rtmp_url = format!("rtmp://{}:1935/live/{}", publisher_node, stream_key);
        // let rtmp_client = create_rtmp_client(&rtmp_url).await?;

        let pull_stream = Arc::new(PullStream::new(
            room_id.to_string(),
            Arc::clone(&self.gop_cache),
        ));

        // Start pull task
        pull_stream.start().await?;

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

/// Pull stream instance (pulls RTMP from publisher, serves FLV to local clients)
pub struct PullStream {
    room_id: String,
    gop_cache: Arc<GopCache>,
    subscriber_count: Arc<RwLock<usize>>,
    last_active: Arc<RwLock<Instant>>,
    is_running: Arc<RwLock<bool>>,
}

impl PullStream {
    pub fn new(room_id: String, gop_cache: Arc<GopCache>) -> Self {
        Self {
            room_id,
            gop_cache,
            subscriber_count: Arc::new(RwLock::new(0)),
            last_active: Arc::new(RwLock::new(Instant::now())),
            is_running: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn start(&self) -> StreamResult<()> {
        *self.is_running.write().await = true;

        // TODO: Start RTMP client task
        // - Connect to publisher node
        // - Receive RTMP packets
        // - Add to GOP cache
        // - Broadcast to subscribers

        log::info!("Pull stream started for room {}", self.room_id);
        Ok(())
    }

    pub async fn stop(&self) -> StreamResult<()> {
        *self.is_running.write().await = false;
        log::info!("Pull stream stopped for room {}", self.room_id);
        Ok(())
    }

    pub async fn is_healthy(&self) -> bool {
        *self.is_running.read().await
    }

    pub fn subscriber_count(&self) -> usize {
        // TODO: Return actual subscriber count
        0
    }

    pub fn last_active_time(&self) -> Instant {
        // TODO: Return actual last active time
        Instant::now()
    }

    pub fn update_last_active_time(&self) {
        // TODO: Update last active time
    }
}
