// Publisher Manager - Handles RTMP publisher registration to Redis
//
// Listens to StreamHub events and manages publisher lifecycle:
// 1. On Publish event: Register publisher to Redis (atomic HSETNX)
// 2. Maintain heartbeat to keep registration alive
// 3. On UnPublish event: Remove publisher from Redis
//
// Based on design doc 17-数据流设计.md § 11.1

use super::registry::HEARTBEAT_INTERVAL_SECS;
use super::registry_trait::StreamRegistryTrait;
use anyhow::anyhow;
use synctv_xiu::streamhub::{
    define::BroadcastEventReceiver,
    stream::StreamIdentifier,
};
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tracing as log;
use dashmap::DashMap;

/// Publisher manager that listens to `StreamHub` events
pub struct PublisherManager {
    registry: Arc<dyn StreamRegistryTrait>,
    local_node_id: String,
    /// Active publishers (`stream_key` -> `media_id`)
    /// Live streaming is media-level, not room-level
    active_publishers: Arc<DashMap<String, String>>,
}

impl PublisherManager {
    pub fn new(registry: Arc<dyn StreamRegistryTrait>, local_node_id: String) -> Self {
        Self {
            registry,
            local_node_id,
            active_publishers: Arc::new(DashMap::new()),
        }
    }

    /// Start listening to `StreamHub` broadcast events
    pub async fn start(self: Arc<Self>, mut event_receiver: BroadcastEventReceiver) {
        log::info!("Publisher manager started");

        // Start heartbeat maintenance task
        let heartbeat_manager = Arc::clone(&self);
        tokio::spawn(async move {
            heartbeat_manager.maintain_heartbeats().await;
        });

        // Listen to broadcast events
        loop {
            match event_receiver.recv().await {
                Ok(event) => {
                    if let Err(e) = self.handle_broadcast_event(event).await {
                        log::error!("Failed to handle broadcast event: {}", e);
                    }
                }
                Err(e) => {
                    log::error!("Error receiving broadcast event: {}", e);
                    break;
                }
            }
        }

        log::warn!("Publisher manager stopped");
    }

    /// Handle `StreamHub` broadcast events
    async fn handle_broadcast_event(&self, event: synctv_xiu::streamhub::define::BroadcastEvent) -> anyhow::Result<()> {
        match event {
            synctv_xiu::streamhub::define::BroadcastEvent::Publish { identifier } => {
                self.handle_publish(identifier).await?;
            }
            synctv_xiu::streamhub::define::BroadcastEvent::UnPublish { identifier } => {
                self.handle_unpublish(identifier).await?;
            }
        }
        Ok(())
    }

    /// Handle Publish event - Register publisher to Redis
    async fn handle_publish(&self, identifier: StreamIdentifier) -> anyhow::Result<()> {
        // Extract app_name and stream_name from RTMP identifier
        let (app_name, stream_name) = if let StreamIdentifier::Rtmp { app_name, stream_name } = identifier { (app_name, stream_name) } else {
            log::warn!("Ignoring non-RTMP publish event: {:?}", identifier);
            return Ok(());
        };

        log::info!(
            "RTMP Publish event: app_name={}, stream_name={}",
            app_name,
            stream_name
        );

        // Parse room_id and media_id from stream_name
        // Expected format: "{room_id}/{media_id}" (e.g., "room123/media456")
        // Live streaming granularity is media-level within a room context
        let (room_id, media_id) = if let Some((r, m)) = stream_name.split_once('/') {
            (r.to_string(), m.to_string())
        } else {
            log::error!("Invalid stream_name format, expected 'room_id/media_id': {}", stream_name);
            return Err(anyhow!("Invalid stream_name format, expected 'room_id/media_id'"));
        };

        // Try to register as publisher (atomic HSETNX)
        match self.registry.try_register_publisher(&room_id, &media_id, &self.local_node_id, "").await {
            Ok(true) => {
                log::info!(
                    "Successfully registered as publisher for room {} / media {} (stream: {})",
                    room_id,
                    media_id,
                    stream_name
                );
                // Track active publisher with composite key
                let publisher_key = format!("{room_id}:{media_id}");
                self.active_publishers.insert(stream_name.clone(), publisher_key);
            }
            Ok(false) => {
                log::warn!(
                    "Publisher already exists for room {} / media {} (stream: {})",
                    room_id,
                    media_id,
                    stream_name
                );
                // Another node is already publisher, reject this push
                return Err(anyhow!("Publisher already exists for room {room_id} / media {media_id}"));
            }
            Err(e) => {
                log::error!("Failed to register publisher: {}", e);
                return Err(e);
            }
        }

        Ok(())
    }

    /// Handle `UnPublish` event - Remove publisher from Redis
    async fn handle_unpublish(&self, identifier: StreamIdentifier) -> anyhow::Result<()> {
        let (app_name, stream_name) = match identifier {
            StreamIdentifier::Rtmp { app_name, stream_name } => (app_name, stream_name),
            _ => {
                return Ok(());
            }
        };

        log::info!(
            "RTMP UnPublish event: app_name={}, stream_name={}",
            app_name,
            stream_name
        );

        // Get composite key (room_id:media_id) from active publishers
        if let Some((_, publisher_key)) = self.active_publishers.remove(&stream_name) {
            // Parse room_id and media_id from the composite key
            if let Some((room_id, media_id)) = publisher_key.split_once(':') {
                // Unregister from Redis
                if let Err(e) = self.registry.unregister_publisher(room_id, media_id).await {
                    log::error!("Failed to unregister publisher for room {} / media {}: {}", room_id, media_id, e);
                } else {
                    log::info!("Unregistered publisher for room {} / media {}", room_id, media_id);
                }
            }
        }

        Ok(())
    }

    /// Maintain heartbeat for all active publishers
    async fn maintain_heartbeats(&self) {
        let mut heartbeat_interval = interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));

        loop {
            heartbeat_interval.tick().await;

            for entry in self.active_publishers.iter() {
                let publisher_key = entry.value();

                // Parse room_id and media_id from the composite key
                if let Some((room_id, media_id)) = publisher_key.split_once(':') {
                    if let Err(e) = self.registry.refresh_publisher_ttl(room_id, media_id, "").await {
                        log::error!("Failed to refresh TTL for room {} / media {}: {}", room_id, media_id, e);
                    }
                }
            }

            if !self.active_publishers.is_empty() {
                log::debug!(
                    "Heartbeat: {} active publishers",
                    self.active_publishers.len()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::MockStreamRegistry;

    #[tokio::test]
    async fn test_publisher_manager_creation() {
        let registry = Arc::new(MockStreamRegistry::new());
        let local_node_id = "test-node-1".to_string();

        let manager = PublisherManager::new(registry, local_node_id);
        assert_eq!(manager.local_node_id, "test-node-1");
        assert!(manager.active_publishers.is_empty());
    }

    #[tokio::test]
    async fn test_active_publishers_map() {
        let (_event_sender, _) = tokio::sync::mpsc::unbounded_channel::<synctv_xiu::streamhub::define::StreamHubEvent>();

        let registry = Arc::new(MockStreamRegistry::new());
        let manager = PublisherManager::new(registry, "test-node".to_string());

        // Verify active publishers map is empty
        assert!(manager.active_publishers.is_empty());
        assert_eq!(manager.active_publishers.len(), 0);
    }

    #[tokio::test]
    async fn test_handle_publish_success() {
        let registry = Arc::new(MockStreamRegistry::new());
        let manager = PublisherManager::new(registry, "test-node-1".to_string());

        let identifier = StreamIdentifier::Rtmp {
            app_name: "live".to_string(),
            stream_name: "room123/media456".to_string(),
        };

        // Handle publish event
        let result = manager.handle_publish(identifier).await;
        assert!(result.is_ok());

        // Verify publisher was tracked
        assert!(manager.active_publishers.contains_key("room123/media456"));
    }

    #[tokio::test]
    async fn test_handle_unpublish_success() {
        let registry = Arc::new(MockStreamRegistry::new());
        let manager = PublisherManager::new(registry, "test-node-1".to_string());

        // First, register a publisher
        let identifier = StreamIdentifier::Rtmp {
            app_name: "live".to_string(),
            stream_name: "room123/media456".to_string(),
        };
        let _ = manager.handle_publish(identifier.clone()).await;

        // Then unpublish
        let result = manager.handle_unpublish(identifier).await;
        assert!(result.is_ok());

        // Verify publisher was removed from tracking
        assert!(!manager.active_publishers.contains_key("room123/media456"));
    }

    #[tokio::test]
    async fn test_handle_publish_invalid_format() {
        let registry = Arc::new(MockStreamRegistry::new());
        let manager = PublisherManager::new(registry, "test-node-1".to_string());

        let identifier = StreamIdentifier::Rtmp {
            app_name: "live".to_string(),
            stream_name: "invalid_format".to_string(), // Missing room_id/media_id separator
        };

        let result = manager.handle_publish(identifier).await;
        assert!(result.is_err());
    }
}
