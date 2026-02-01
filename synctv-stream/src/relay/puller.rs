use anyhow::{Result, anyhow};
use tracing::{info, warn};
use crate::relay::{StreamRegistry, PublisherInfo};

/// Puller node - pulls stream from Publisher and serves to local viewers
pub struct Puller {
    room_id: String,
    media_id: String,
    node_id: String,
    registry: StreamRegistry,
    publisher_info: Option<PublisherInfo>,
}

impl Puller {
    /// Create a new Puller
    pub fn new(room_id: String, media_id: String, node_id: String, registry: StreamRegistry) -> Self {
        Self {
            room_id,
            media_id,
            node_id,
            registry,
            publisher_info: None,
        }
    }

    /// Start pulling stream from Publisher
    pub async fn start(&mut self) -> Result<()> {
        info!(
            room_id = self.room_id,
            media_id = self.media_id,
            node_id = self.node_id,
            "Puller starting"
        );

        // Get Publisher info from Redis
        let publisher_info = self
            .registry
            .get_publisher(&self.room_id, &self.media_id)
            .await?
            .ok_or_else(|| anyhow!("No active publisher for room {} / media {}", self.room_id, self.media_id))?;

        info!(
            room_id = self.room_id,
            media_id = self.media_id,
            publisher_node = publisher_info.node_id,
            "Found publisher, establishing gRPC connection"
        );

        self.publisher_info = Some(publisher_info.clone());

        // TODO: Establish gRPC connection to Publisher node
        // TODO: Call PullRtmpStream RPC
        // TODO: Start receiving frames via streaming RPC
        // TODO: Transcode to HLS/FLV
        // TODO: Serve to local viewers

        Ok(())
    }

    /// Stop pulling and cleanup
    pub async fn stop(&mut self) -> Result<()> {
        info!(
            room_id = self.room_id,
            media_id = self.media_id,
            node_id = self.node_id,
            "Puller stopping"
        );

        // TODO: Close gRPC connection to Publisher
        // TODO: Stop local stream distribution

        Ok(())
    }

    /// Get Publisher node ID
    pub fn publisher_node_id(&self) -> Option<&str> {
        self.publisher_info.as_ref().map(|info| info.node_id.as_str())
    }

    /// Check if still pulling from Publisher
    pub async fn is_active(&mut self) -> Result<bool> {
        self.registry.is_stream_active(&self.room_id, &self.media_id).await
    }
}

impl Drop for Puller {
    fn drop(&mut self) {
        warn!(
            room_id = self.room_id,
            media_id = self.media_id,
            "Puller dropped - may need manual cleanup"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires Redis and active Publisher
    async fn test_puller_start() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = redis::aio::ConnectionManager::new(redis_client)
            .await
            .unwrap();
        let mut registry = StreamRegistry::new(redis.clone());

        // Register a fake publisher
        registry
            .register_publisher("room123", "publisher-node", "live")
            .await
            .unwrap();

        // Create puller
        let mut puller = Puller::new(
            "room123".to_string(),
            "puller-node".to_string(),
            StreamRegistry::new(redis),
        );

        // Start pulling
        puller.start().await.unwrap();

        assert_eq!(puller.publisher_node_id(), Some("publisher-node"));

        // Cleanup
        puller.stop().await.unwrap();
        registry.unregister_publisher("room123").await.unwrap();
    }
}
