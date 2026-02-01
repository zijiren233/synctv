use anyhow::Result;
use std::sync::Arc;
use tracing::{info, warn};
use crate::cache::{GopCache, GopFrame, FrameType};
use crate::relay::StreamRegistry;

/// Publisher node - accepts RTMP push and serves to Pullers
pub struct Publisher {
    room_id: String,
    media_id: String,
    node_id: String,
    gop_cache: Arc<GopCache>,
    registry: StreamRegistry,
}

impl Publisher {
    /// Create a new Publisher
    pub fn new(
        room_id: String,
        media_id: String,
        node_id: String,
        gop_cache: Arc<GopCache>,
        registry: StreamRegistry,
    ) -> Self {
        Self {
            room_id,
            media_id,
            node_id,
            gop_cache,
            registry,
        }
    }

    /// Handle incoming RTMP data
    /// This will be called by xiu when data is received
    pub async fn on_rtmp_data(
        &mut self,
        data: bytes::Bytes,
        timestamp: u32,
        is_keyframe: bool,
        is_video: bool,
    ) -> Result<()> {
        // Add frame to GOP cache
        let frame = GopFrame {
            data,
            timestamp,
            is_keyframe,
            frame_type: if is_video {
                FrameType::Video
            } else {
                FrameType::Audio
            },
        };

        // Use composite key "room_id:media_id" for GOP cache
        let stream_key = format!("{}:{}", self.room_id, self.media_id);
        self.gop_cache.add_frame(&stream_key, frame);

        // TODO: Broadcast to local viewers
        // TODO: Send to gRPC stream relay service for Pullers

        Ok(())
    }

    /// Stop publishing and cleanup
    pub async fn stop(&mut self) -> Result<()> {
        info!(
            room_id = self.room_id,
            media_id = self.media_id,
            node_id = self.node_id,
            "Publisher stopping"
        );

        // Unregister from Redis
        self.registry.unregister_publisher(&self.room_id, &self.media_id).await?;

        // Clear GOP cache
        let stream_key = format!("{}:{}", self.room_id, self.media_id);
        self.gop_cache.clear_stream(&stream_key);

        Ok(())
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        warn!(
            room_id = self.room_id,
            "Publisher dropped - may need manual cleanup"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::GopCacheConfig;

    #[tokio::test]
    #[ignore] // Requires Redis instance
    async fn test_publisher_creation() {
        let gop_cache = Arc::new(GopCache::new(GopCacheConfig::default()));
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = redis::aio::ConnectionManager::new(redis_client)
            .await
            .unwrap();
        let registry = StreamRegistry::new(redis);

        let publisher = Publisher::new(
            "room123".to_string(),
            "node1".to_string(),
            gop_cache,
            registry,
        );

        assert_eq!(publisher.room_id, "room123");
        assert_eq!(publisher.node_id, "node1");
    }
}
