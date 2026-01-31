// Publisher Manager - Handles RTMP publisher registration to Redis
//
// Listens to StreamHub events and manages publisher lifecycle:
// 1. On Publish event: Register publisher to Redis (atomic HSETNX)
// 2. Maintain heartbeat to keep registration alive
// 3. On UnPublish event: Remove publisher from Redis
//
// Based on design doc 17-数据流设计.md § 11.1

use super::registry::StreamRegistry;
use anyhow::{Result, anyhow};
use streamhub::{
    define::{StreamHubEvent, BroadcastEventReceiver},
    stream::StreamIdentifier,
};
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tracing as log;
use dashmap::DashMap;

/// Publisher manager that listens to StreamHub events
pub struct PublisherManager {
    registry: StreamRegistry,
    local_node_id: String,
    /// Active publishers (stream_key -> room_id)
    active_publishers: Arc<DashMap<String, String>>,
}

impl PublisherManager {
    pub fn new(registry: StreamRegistry, local_node_id: String) -> Self {
        Self {
            registry,
            local_node_id,
            active_publishers: Arc::new(DashMap::new()),
        }
    }

    /// Start listening to StreamHub broadcast events
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

    /// Handle StreamHub broadcast events
    async fn handle_broadcast_event(&self, event: streamhub::define::BroadcastEvent) -> anyhow::Result<()> {
        match event {
            streamhub::define::BroadcastEvent::Publish { identifier, .. } => {
                self.handle_publish(identifier).await?;
            }
            streamhub::define::BroadcastEvent::UnPublish { identifier, .. } => {
                self.handle_unpublish(identifier).await?;
            }
            _ => {
                // Ignore other events (Subscribe, UnSubscribe)
            }
        }
        Ok(())
    }

    /// Handle Publish event - Register publisher to Redis
    async fn handle_publish(&self, identifier: StreamIdentifier) -> anyhow::Result<()> {
        // Extract app_name and stream_name from RTMP identifier
        let (app_name, stream_name) = match identifier {
            StreamIdentifier::Rtmp { app_name, stream_name } => (app_name, stream_name),
            _ => {
                log::warn!("Ignoring non-RTMP publish event: {:?}", identifier);
                return Ok(());
            }
        };

        log::info!(
            "RTMP Publish event: app_name={}, stream_name={}",
            app_name,
            stream_name
        );

        // Parse room_id from stream_name (expected format: "room_123")
        let room_id = if stream_name.starts_with("room_") {
            stream_name.strip_prefix("room_").unwrap().to_string()
        } else {
            stream_name.clone()
        };

        // Try to register as publisher (atomic HSETNX)
        match self.registry.try_register_publisher(&room_id, &self.local_node_id).await {
            Ok(true) => {
                log::info!(
                    "Successfully registered as publisher for room {} (stream: {})",
                    room_id,
                    stream_name
                );
                // Track active publisher
                self.active_publishers.insert(stream_name.clone(), room_id);
            }
            Ok(false) => {
                log::warn!(
                    "Publisher already exists for room {} (stream: {})",
                    room_id,
                    stream_name
                );
                // Another node is already publisher, reject this push
                return Err(anyhow!("Publisher already exists for room {}", room_id));
            }
            Err(e) => {
                log::error!("Failed to register publisher: {}", e);
                return Err(e);
            }
        }

        Ok(())
    }

    /// Handle UnPublish event - Remove publisher from Redis
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

        // Get room_id from active publishers
        if let Some((_, room_id)) = self.active_publishers.remove(&stream_name) {
            // Unregister from Redis
            if let Err(e) = self.registry.unregister_publisher_immut(&room_id).await {
                log::error!("Failed to unregister publisher for room {}: {}", room_id, e);
            } else {
                log::info!("Unregistered publisher for room {}", room_id);
            }
        }

        Ok(())
    }

    /// Maintain heartbeat for all active publishers
    async fn maintain_heartbeats(&self) {
        let mut heartbeat_interval = interval(Duration::from_secs(60));

        loop {
            heartbeat_interval.tick().await;

            for entry in self.active_publishers.iter() {
                let room_id = entry.value();

                if let Err(e) = self.registry.refresh_publisher_ttl(room_id).await {
                    log::error!("Failed to refresh TTL for room {}: {}", room_id, e);
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
