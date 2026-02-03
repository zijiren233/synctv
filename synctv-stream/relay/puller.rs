use anyhow::{Result, anyhow};
use tracing::{info, warn, error};
use crate::relay::{StreamRegistryTrait, PublisherInfo};
use crate::grpc::GrpcStreamPuller;
use streamhub::define::StreamHubEventSender;
use std::sync::Arc;

/// Puller node - pulls stream from Publisher and serves to local viewers
pub struct Puller {
    room_id: String,
    media_id: String,
    node_id: String,
    registry: Arc<dyn StreamRegistryTrait>,
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
        registry: Arc<dyn StreamRegistryTrait>,
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
            self.registry.clone(),
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
    use crate::relay::MockStreamRegistry;

    #[tokio::test]
    async fn test_puller_creation() {
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let puller = Puller::new(
            "room123".to_string(),
            "media123".to_string(),
            "puller-node".to_string(),
            Arc::new(MockStreamRegistry::new()),
            stream_hub_event_sender,
        );

        assert_eq!(puller.room_id, "room123");
        assert_eq!(puller.media_id, "media123");
        assert_eq!(puller.node_id, "puller-node");
        assert!(puller.grpc_puller_handle.is_none());
        assert!(puller.publisher_info.is_none());
    }

    #[tokio::test]
    async fn test_puller_start() {
        let mut publishers = std::collections::HashMap::new();
        publishers.insert(
            ("room123".to_string(), "media123".to_string()),
            PublisherInfo {
                node_id: "publisher-node".to_string(),
                app_name: "live".to_string(),
                started_at: chrono::Utc::now(),
            }
        );

        let registry = Arc::new(MockStreamRegistry::with_publishers(publishers));
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let mut puller = Puller::new(
            "room123".to_string(),
            "media123".to_string(),
            "puller-node".to_string(),
            registry,
            stream_hub_event_sender,
        );

        // Start pulling - spawns a background task for gRPC connection
        let result = puller.start().await;
        assert!(result.is_ok(), "Should spawn gRPC puller task successfully");

        // Verify that a handle was created for the background task
        assert!(puller.grpc_puller_handle.is_some(), "Should have gRPC puller handle");

        // Verify publisher info was set
        assert!(puller.publisher_info.is_some(), "Should have publisher info");
        assert_eq!(puller.publisher_info.as_ref().unwrap().node_id, "publisher-node");
    }

    #[tokio::test]
    async fn test_puller_is_active() {
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let mut puller = Puller::new(
            "room123".to_string(),
            "media123".to_string(),
            "puller-node".to_string(),
            Arc::new(MockStreamRegistry::new()),
            stream_hub_event_sender,
        );

        // No task running, should not be active
        let result = puller.is_active().await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_puller_stop() {
        let mut publishers = std::collections::HashMap::new();
        publishers.insert(
            ("room123".to_string(), "media123".to_string()),
            PublisherInfo {
                node_id: "publisher-node".to_string(),
                app_name: "live".to_string(),
                started_at: chrono::Utc::now(),
            }
        );

        let registry = Arc::new(MockStreamRegistry::with_publishers(publishers));
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let mut puller = Puller::new(
            "room123".to_string(),
            "media123".to_string(),
            "puller-node".to_string(),
            registry,
            stream_hub_event_sender,
        );

        // Start puller (will fail to connect, but handle should be created)
        let _ = puller.start().await;

        // Stop should succeed
        let result = puller.stop().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_puller_publisher_node_id() {
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let puller = Puller::new(
            "room123".to_string(),
            "media123".to_string(),
            "puller-node".to_string(),
            Arc::new(MockStreamRegistry::new()),
            stream_hub_event_sender,
        );

        // No publisher set, should return None
        assert!(puller.publisher_node_id().is_none());
    }
}
