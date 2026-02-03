use anyhow::{Result, anyhow};
use tracing::{info, warn, error};
use crate::relay::{StreamRegistry, PublisherInfo};
use crate::grpc::GrpcStreamPuller;
use streamhub::define::StreamHubEventSender;

/// Puller node - pulls stream from Publisher and serves to local viewers
pub struct Puller {
    room_id: String,
    media_id: String,
    node_id: String,
    registry: StreamRegistry,
    publisher_info: Option<PublisherInfo>,
    stream_hub_event_sender: StreamHubEventSender,
    grpc_puller_handle: Option<tokio::task::JoinHandle<Result<()>>>,
}

impl Puller {
    /// Create a new Puller
    pub fn new(
        room_id: String,
        media_id: String,
        node_id: String,
        registry: StreamRegistry,
        stream_hub_event_sender: StreamHubEventSender,
    ) -> Self {
        Self {
            room_id,
            media_id,
            node_id,
            registry,
            publisher_info: None,
            stream_hub_event_sender,
            grpc_puller_handle: None,
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

        // Create gRPC stream puller
        let grpc_puller = GrpcStreamPuller::new(
            self.room_id.clone(),
            self.media_id.clone(),
            publisher_info.node_id.clone(),
            self.node_id.clone(),
            self.stream_hub_event_sender.clone(),
            std::sync::Arc::new(self.registry.clone()),
        );

        // Spawn puller task
        let handle = tokio::spawn(async move {
            info!("Starting gRPC stream puller task");
            grpc_puller.run().await
        });

        self.grpc_puller_handle = Some(handle);

        info!(
            "gRPC puller task started for room {} / media {}",
            self.room_id, self.media_id
        );

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

        // Abort the gRPC puller task if running
        if let Some(handle) = self.grpc_puller_handle.take() {
            handle.abort();
            info!("gRPC puller task aborted");
        }

        Ok(())
    }

    /// Get Publisher node ID
    pub fn publisher_node_id(&self) -> Option<&str> {
        self.publisher_info.as_ref().map(|info| info.node_id.as_str())
    }

    /// Check if still pulling from Publisher
    pub async fn is_active(&mut self) -> Result<bool> {
        // Check if the task is still running
        if let Some(handle) = &self.grpc_puller_handle {
            Ok(!handle.is_finished())
        } else {
            Ok(false)
        }
    }

    /// Check if the puller task has finished and get the result
    pub async fn check_result(&mut self) -> Option<Result<()>> {
        if let Some(handle) = self.grpc_puller_handle.take() {
            if handle.is_finished() {
                Some(handle.await.unwrap_or_else(|e| Err(anyhow!("Puller task panicked: {}", e))))
            } else {
                // Task is still running, put the handle back
                self.grpc_puller_handle = Some(handle);
                None
            }
        } else {
            None
        }
    }
}

impl Drop for Puller {
    fn drop(&mut self) {
        if self.grpc_puller_handle.is_some() {
            warn!(
                room_id = self.room_id,
                media_id = self.media_id,
                "Puller dropped - gRPC task may still be running"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_puller_creation() {
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let Some(redis_conn) = try_redis_connection().await else {
            eprintln!("Redis not available, skipping test");
            return;
        };

        let puller = Puller::new(
            "room123".to_string(),
            "media123".to_string(),
            "puller-node".to_string(),
            StreamRegistry::new(redis_conn),
            stream_hub_event_sender,
        );

        assert_eq!(puller.room_id, "room123");
        assert_eq!(puller.media_id, "media123");
        assert_eq!(puller.node_id, "puller-node");
        assert!(puller.grpc_puller_handle.is_none());
        assert!(puller.publisher_info.is_none());
    }

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
            .register_publisher("room123", "media123", "publisher-node", "live")
            .await
            .unwrap();

        // Create a dummy stream hub event sender
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        // Create puller
        let mut puller = Puller::new(
            "room123".to_string(),
            "media123".to_string(),
            "puller-node".to_string(),
            StreamRegistry::new(redis),
            stream_hub_event_sender,
        );

        // Start pulling
        // Note: This will fail to connect to publisher-node since it's not actually running
        let result = puller.start().await;
        assert!(result.is_err(), "Should fail to connect to non-existent publisher");

        // Cleanup
        registry.unregister_publisher("room123", "media123").await.unwrap();
    }

    #[tokio::test]
    async fn test_puller_is_active() {
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let Some(redis_conn) = try_redis_connection().await else {
            eprintln!("Redis not available, skipping test");
            return;
        };

        let mut puller = Puller::new(
            "room123".to_string(),
            "media123".to_string(),
            "puller-node".to_string(),
            StreamRegistry::new(redis_conn),
            stream_hub_event_sender,
        );

        // No task running, should not be active
        let result = puller.is_active().await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    #[ignore] // Requires Redis and active Publisher
    async fn test_puller_stop() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = redis::aio::ConnectionManager::new(redis_client)
            .await
            .unwrap();
        let mut registry = StreamRegistry::new(redis.clone());

        registry
            .register_publisher("room123", "media123", "publisher-node", "live")
            .await
            .unwrap();

        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let mut puller = Puller::new(
            "room123".to_string(),
            "media123".to_string(),
            "puller-node".to_string(),
            StreamRegistry::new(redis),
            stream_hub_event_sender,
        );

        // Start puller (will fail to connect, but handle should be created)
        let _ = puller.start().await;

        // Stop should succeed
        let result = puller.stop().await;
        assert!(result.is_ok());

        // Cleanup
        registry.unregister_publisher("room123", "media123").await.unwrap();
    }

    #[tokio::test]
    async fn test_puller_publisher_node_id() {
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let Some(redis_conn) = try_redis_connection().await else {
            eprintln!("Redis not available, skipping test");
            return;
        };

        let puller = Puller::new(
            "room123".to_string(),
            "media123".to_string(),
            "puller-node".to_string(),
            StreamRegistry::new(redis_conn),
            stream_hub_event_sender,
        );

        // No publisher set, should return None
        assert!(puller.publisher_node_id().is_none());
    }

    // Helper function for tests that need Redis
    // Returns None if Redis is not available
    async fn try_redis_connection() -> Option<redis::aio::ConnectionManager> {
        let redis_client = redis::Client::open("redis://127.0.0.1:6379").unwrap();
        match redis::aio::ConnectionManager::new(redis_client).await {
            Ok(conn) => Some(conn),
            Err(_) => None,
        }
    }
}
