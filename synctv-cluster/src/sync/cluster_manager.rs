//! Complete cluster synchronization service
//!
//! This module provides a unified interface for all cross-cluster functionality:
//! - Message broadcasting (local)
//! - Redis pub/sub (cross-node)
//! - Message deduplication
//! - Connection management
//! - Metrics and monitoring

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::dedup::{DedupKey, MessageDeduplicator};
use super::events::ClusterEvent;
use super::redis_pubsub::{PublishRequest, RedisPubSub};
use super::room_hub::{ConnectionId, RoomMessageHub};
use synctv_core::models::id::{RoomId, UserId};

/// Cluster configuration
#[derive(Debug, Clone)]
pub struct ClusterConfig {
    /// Redis connection URL
    pub redis_url: String,
    /// Unique identifier for this node
    pub node_id: String,
    /// Deduplication window duration
    pub dedup_window: Duration,
    /// How often to cleanup dedup entries
    pub cleanup_interval: Duration,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            redis_url: "redis://127.0.0.1:6379".to_string(),
            node_id: format!("node_{}", nanoid::nanoid!(8)),
            dedup_window: Duration::from_secs(5),
            cleanup_interval: Duration::from_secs(30),
        }
    }
}

/// Cluster synchronization manager
///
/// This is the main entry point for all cross-cluster functionality.
/// It manages:
/// - Local message broadcasting via `RoomMessageHub`
/// - Cross-node synchronization via Redis Pub/Sub
/// - Message deduplication
/// - Connection lifecycle
pub struct ClusterManager {
    /// Message hub for local broadcasting
    message_hub: Arc<RoomMessageHub>,
    /// Deduplicator for preventing duplicate events
    deduplicator: Arc<MessageDeduplicator>,
    /// Sender for publishing events to Redis
    redis_publish_tx: Option<mpsc::UnboundedSender<PublishRequest>>,
    /// This node's unique identifier
    node_id: String,
}

impl ClusterManager {
    /// Create a new cluster manager
    pub async fn new(config: ClusterConfig) -> Result<Self, anyhow::Error> {
        let message_hub = Arc::new(RoomMessageHub::new());
        let deduplicator = Arc::new(MessageDeduplicator::new(
            config.dedup_window,
            config.cleanup_interval,
        ));

        // Start Redis pub/sub if Redis URL is provided
        let redis_publish_tx = if config.redis_url.is_empty() {
            warn!("Redis URL not provided, running in single-node mode");
            None
        } else {
            let redis_pubsub = Arc::new(
                RedisPubSub::new(
                    &config.redis_url,
                    message_hub.clone(),
                    config.node_id.clone(),
                )?
            );

            Some(redis_pubsub.clone().start().await?)
        };

        Ok(Self {
            message_hub,
            deduplicator,
            redis_publish_tx,
            node_id: config.node_id,
        })
    }

    /// Create with default configuration
    pub async fn with_defaults() -> Result<Self, anyhow::Error> {
        Self::new(ClusterConfig::default()).await
    }

    /// Get the message hub (for subscriptions)
    #[must_use] 
    pub const fn message_hub(&self) -> &Arc<RoomMessageHub> {
        &self.message_hub
    }

    /// Get the deduplicator
    #[must_use] 
    pub const fn deduplicator(&self) -> &Arc<MessageDeduplicator> {
        &self.deduplicator
    }

    /// Get the Redis publish sender
    #[must_use] 
    pub const fn redis_publish_tx(&self) -> Option<&mpsc::UnboundedSender<PublishRequest>> {
        self.redis_publish_tx.as_ref()
    }

    /// Broadcast an event to all subscribers
    ///
    /// This will:
    /// 1. Check for duplicates
    /// 2. Broadcast to local subscribers
    /// 3. Publish to Redis for cross-node sync
    pub fn broadcast(&self, event: ClusterEvent) -> BroadcastResult {
        let dedup_key = DedupKey::from_event(&event);

        // Check if this is a duplicate
        if !self.deduplicator.should_process(&dedup_key) {
            debug!(
                event_type = %event.event_type(),
                room_id = %event.room_id()
                    .map_or("n/a", synctv_core::models::RoomId::as_str),
                "Duplicate event detected, skipping"
            );
            return BroadcastResult {
                local_sent: 0,
                redis_sent: false,
            };
        }

        let mut local_sent = 0;
        let mut redis_sent = 0;

        // Get event_type for logging before moving event
        let event_type = event.event_type();

        // Get room_id for broadcasting
        if let Some(room_id) = event.room_id() {
            // Broadcast to local subscribers
            local_sent = self.message_hub.broadcast(room_id, event.clone());
        }

        // Publish to Redis for cross-node sync
        if let Some(tx) = &self.redis_publish_tx {
            if let Some(room_id) = event.room_id() {
                if let Err(e) = tx.send(PublishRequest {
                    room_id: room_id.clone(),
                    event,
                }) {
                    error!(
                        error = %e,
                        "Failed to queue event for Redis publishing"
                    );
                } else {
                    redis_sent = 1;
                }
            }
        }

        debug!(
            event_type = %event_type,
            local_subscribers = local_sent,
            redis_published = redis_sent > 0,
            "Event broadcast complete"
        );

        BroadcastResult {
            local_sent,
            redis_sent: redis_sent > 0,
        }
    }

    /// Subscribe a client to room events
    ///
    /// Returns a receiver for messages and a connection ID for cleanup
    pub fn subscribe(
        &self,
        room_id: RoomId,
        user_id: UserId,
    ) -> (tokio::sync::mpsc::UnboundedReceiver<ClusterEvent>, ConnectionId) {
        let room_id_str = room_id.as_str().to_string();
        let user_id_str = user_id.as_str().to_string();
        let connection_id = format!("{}_{}", user_id_str, nanoid::nanoid!(8));
        let rx = self.message_hub.subscribe(room_id, user_id, connection_id.clone());

        info!(
            room_id = %room_id_str,
            user_id = %user_id_str,
            connection_id = %connection_id,
            "Client subscribed to room"
        );

        (rx, connection_id)
    }

    /// Unsubscribe a client from room events
    pub fn unsubscribe(&self, connection_id: &str) {
        self.message_hub.unsubscribe(connection_id);
    }

    /// Get cluster metrics
    #[must_use] 
    pub fn metrics(&self) -> ClusterMetrics {
        ClusterMetrics {
            node_id: self.node_id.clone(),
            total_rooms: self.message_hub.room_count(),
            total_connections: self.message_hub.connection_count(),
            tracked_events: self.deduplicator.len(),
            redis_enabled: self.redis_publish_tx.is_some(),
        }
    }

    /// Get subscribers in a room
    #[must_use] 
    pub fn get_room_subscribers(&self, room_id: &RoomId) -> Vec<(UserId, ConnectionId)> {
        self.message_hub.get_room_subscribers(room_id)
    }
}

/// Result of broadcasting an event
#[derive(Debug, Clone)]
pub struct BroadcastResult {
    /// Number of local subscribers the event was sent to
    pub local_sent: usize,
    /// Whether the event was published to Redis
    pub redis_sent: bool,
}

/// Cluster metrics
#[derive(Debug, Clone)]
pub struct ClusterMetrics {
    pub node_id: String,
    pub total_rooms: usize,
    pub total_connections: usize,
    pub tracked_events: usize,
    pub redis_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn test_cluster_manager_single_node() {
        let config = ClusterConfig {
            redis_url: "".to_string(), // No Redis
            node_id: "test_node".to_string(),
            dedup_window: Duration::from_secs(1),
            cleanup_interval: Duration::from_secs(1),
        };

        let manager = ClusterManager::new(config).await.unwrap();

        // Subscribe a client
        let room_id = RoomId::from_string("room1".to_string());
        let user_id = UserId::from_string("user1".to_string());
        let (mut rx, conn_id) = manager.subscribe(room_id.clone(), user_id.clone());

        // Broadcast event
        let event = ClusterEvent::ChatMessage {
            room_id: room_id.clone(),
            user_id: user_id.clone(),
            username: "user1".to_string(),
            message: "Hello!".to_string(),
            timestamp: Utc::now(),
        };

        let result = manager.broadcast(event.clone());

        assert_eq!(result.local_sent, 1);
        assert!(!result.redis_sent);

        // Verify duplicate detection
        let result2 = manager.broadcast(event);
        assert_eq!(result2.local_sent, 0);
        assert!(matches!(result2, BroadcastResult { local_sent: 0, redis_sent: false }));

        // Verify message received
        let received = rx.recv().await.unwrap();
        assert_eq!(received.event_type(), "chat_message");

        // Cleanup
        manager.unsubscribe(&conn_id);

        let metrics = manager.metrics();
        assert_eq!(metrics.total_connections, 0);
    }
}
