use anyhow::{Context, Result};
use futures::stream::StreamExt;
use redis::{AsyncCommands, Client as RedisClient};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::events::ClusterEvent;
use super::room_hub::RoomMessageHub;
use synctv_core::models::id::RoomId;

/// Redis Pub/Sub service for cross-node event synchronization
///
/// This service enables multi-replica deployments by:
/// 1. Publishing local room events to Redis channels
/// 2. Subscribing to Redis channels for events from other nodes
/// 3. Forwarding received events to the local `RoomMessageHub`
///
/// Channel naming: `room:{room_id`} for room-specific events
pub struct RedisPubSub {
    redis_client: RedisClient,
    message_hub: Arc<RoomMessageHub>,
    node_id: String,
}

impl RedisPubSub {
    /// Create a new `RedisPubSub` service
    pub fn new(redis_url: &str, message_hub: Arc<RoomMessageHub>, node_id: String) -> Result<Self> {
        let redis_client = RedisClient::open(redis_url).context("Failed to create Redis client")?;

        Ok(Self {
            redis_client,
            message_hub,
            node_id,
        })
    }

    /// Start the Pub/Sub service
    /// This spawns a background task that subscribes to all room channels
    pub async fn start(self: Arc<Self>) -> Result<mpsc::UnboundedSender<PublishRequest>> {
        // Create channel for publishing events
        let (publish_tx, mut publish_rx) = mpsc::unbounded_channel::<PublishRequest>();

        // Clone for the publish task
        let publish_client = self.redis_client.clone();
        let node_id = self.node_id.clone();

        // Spawn task to handle publishing
        tokio::spawn(async move {
            let mut conn = match publish_client.get_multiplexed_async_connection().await {
                Ok(conn) => conn,
                Err(e) => {
                    error!(error = %e, "Failed to get Redis connection for publishing");
                    return;
                }
            };

            info!("Redis publisher task started");

            while let Some(req) = publish_rx.recv().await {
                match Self::publish_event(&mut conn, &node_id, req.event).await {
                    Ok(subscribers) => {
                        debug!(
                            room_id = %req.room_id.as_str(),
                            subscribers = subscribers,
                            "Event published to Redis"
                        );
                    }
                    Err(e) => {
                        error!(
                            error = %e,
                            room_id = %req.room_id.as_str(),
                            "Failed to publish event to Redis"
                        );
                    }
                }
            }

            warn!("Redis publisher task exiting");
        });

        // Clone for the subscriber task
        let self_clone = self;

        // Spawn task to handle subscribing
        tokio::spawn(async move {
            loop {
                match self_clone.run_subscriber().await {
                    Ok(()) => {
                        info!("Redis subscriber task completed normally");
                        break;
                    }
                    Err(e) => {
                        error!(error = %e, "Redis subscriber task failed, retrying in 5s");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    }
                }
            }
        });

        Ok(publish_tx)
    }

    /// Run the subscriber task
    /// This subscribes to room:* pattern and forwards events to `RoomMessageHub`
    async fn run_subscriber(&self) -> Result<()> {
        let mut pubsub = self
            .redis_client
            .get_async_pubsub()
            .await
            .context("Failed to get Redis Pub/Sub connection")?;

        // Subscribe to all room channels using pattern
        pubsub
            .psubscribe("room:*")
            .await
            .context("Failed to subscribe to room:* pattern")?;

        info!("Redis subscriber connected, listening to room:* channels");

        // Process incoming messages
        let mut stream = pubsub.on_message();

        while let Some(msg) = stream.next().await {
            let channel = msg.get_channel_name().to_string();

            let payload: String = match msg.get_payload() {
                Ok(p) => p,
                Err(e) => {
                    warn!(error = %e, channel = %channel, "Invalid payload");
                    continue;
                }
            };

            // Parse the envelope
            match serde_json::from_str::<EventEnvelope>(&payload) {
                Ok(envelope) => {
                    // Ignore events from this node (already broadcasted locally)
                    if envelope.node_id == self.node_id {
                        debug!(
                            channel = %channel,
                            "Ignoring event from self (node_id: {})",
                            self.node_id
                        );
                        continue;
                    }

                    // Extract room_id from channel name (room:{room_id})
                    if let Some(room_id_str) = channel.strip_prefix("room:") {
                        let room_id = RoomId::from_string(room_id_str.to_string());

                        debug!(
                            channel = %channel,
                            from_node = %envelope.node_id,
                            event_type = %envelope.event.event_type(),
                            "Received event from Redis"
                        );

                        // Broadcast to local subscribers
                        let sent_count = self.message_hub.broadcast(&room_id, envelope.event);

                        debug!(
                            room_id = %room_id.as_str(),
                            local_subscribers = sent_count,
                            "Forwarded Redis event to local subscribers"
                        );
                    } else {
                        warn!(channel = %channel, "Invalid channel format");
                    }
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        channel = %channel,
                        payload = %payload,
                        "Failed to deserialize event envelope"
                    );
                }
            }
        }

        Ok(())
    }

    /// Publish an event to Redis
    async fn publish_event(
        conn: &mut redis::aio::MultiplexedConnection,
        node_id: &str,
        event: ClusterEvent,
    ) -> Result<usize> {
        let room_id = event
            .room_id()
            .context("Cannot publish event without room_id")?;

        let channel = format!("room:{}", room_id.as_str());

        // Wrap event in envelope with node_id
        let envelope = EventEnvelope {
            node_id: node_id.to_string(),
            event,
        };

        let payload =
            serde_json::to_string(&envelope).context("Failed to serialize event envelope")?;

        // Publish to Redis
        let subscribers: usize = conn
            .publish(&channel, payload)
            .await
            .context("Failed to publish to Redis")?;

        Ok(subscribers)
    }
}

/// Request to publish an event
pub struct PublishRequest {
    pub room_id: RoomId,
    pub event: ClusterEvent,
}

/// Envelope for events published to Redis
/// Includes `node_id` to avoid echo (each node ignores its own events)
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct EventEnvelope {
    node_id: String,
    event: ClusterEvent,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use synctv_core::models::id::UserId;

    #[test]
    fn test_event_envelope_serialization() {
        let event = ClusterEvent::ChatMessage {
            room_id: RoomId::from_string("room123".to_string()),
            user_id: UserId::from_string("user456".to_string()),
            username: "testuser".to_string(),
            message: "Hello!".to_string(),
            timestamp: Utc::now(),
        };

        let envelope = EventEnvelope {
            node_id: "node1".to_string(),
            event,
        };

        // Serialize
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(json.contains("node1"));
        assert!(json.contains("chat_message"));

        // Deserialize
        let deserialized: EventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.node_id, "node1");
        assert_eq!(deserialized.event.event_type(), "chat_message");
    }

    // Integration tests require Redis running
    #[tokio::test]
    #[ignore = "Requires Redis server"]
    async fn test_pubsub_integration() {
        let redis_url = "redis://127.0.0.1:6379";
        let message_hub = Arc::new(RoomMessageHub::new());

        // Create two PubSub instances simulating different nodes
        let pubsub1 = Arc::new(
            RedisPubSub::new(redis_url, message_hub.clone(), "node1".to_string()).unwrap(),
        );
        let pubsub2 = Arc::new(
            RedisPubSub::new(redis_url, message_hub.clone(), "node2".to_string()).unwrap(),
        );

        // Start both
        let publish_tx1 = pubsub1.start().await.unwrap();
        let _publish_tx2 = pubsub2.start().await.unwrap();

        // Wait for connections to establish
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Subscribe a client to the room
        let room_id = RoomId::from_string("test_room".to_string());
        let user_id = UserId::from_string("test_user".to_string());
        let mut rx = message_hub.subscribe(room_id.clone(), user_id.clone(), "conn1".to_string());

        // Publish event from node1
        let event = ClusterEvent::ChatMessage {
            room_id: room_id.clone(),
            user_id: user_id.clone(),
            username: "testuser".to_string(),
            message: "Hello from node1!".to_string(),
            timestamp: Utc::now(),
        };

        publish_tx1
            .send(PublishRequest {
                room_id: room_id.clone(),
                event,
            })
            .unwrap();

        // Wait for event propagation
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Client should receive the event
        let received = tokio::time::timeout(tokio::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(received.event_type(), "chat_message");
    }
}
