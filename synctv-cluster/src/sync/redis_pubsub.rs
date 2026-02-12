use anyhow::{Context, Result};
use futures::stream::StreamExt;
use redis::{AsyncCommands, Client as RedisClient};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

/// Timeout for Redis operations in seconds
const REDIS_TIMEOUT_SECS: u64 = 5;

/// Initial backoff delay for subscriber reconnection
const INITIAL_BACKOFF_SECS: u64 = 1;

/// Maximum backoff delay for subscriber reconnection
const MAX_BACKOFF_SECS: u64 = 30;

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
    admin_event_tx: broadcast::Sender<ClusterEvent>,
}

impl RedisPubSub {
    /// Create a new `RedisPubSub` service
    pub fn new(
        redis_url: &str,
        message_hub: Arc<RoomMessageHub>,
        node_id: String,
        admin_event_tx: broadcast::Sender<ClusterEvent>,
    ) -> Result<Self> {
        let redis_client = RedisClient::open(redis_url).context("Failed to create Redis client")?;

        Ok(Self {
            redis_client,
            message_hub,
            node_id,
            admin_event_tx,
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
            let mut conn = match timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                publish_client.get_multiplexed_async_connection(),
            )
            .await
            {
                Ok(Ok(conn)) => conn,
                Ok(Err(e)) => {
                    error!(error = %e, "Failed to get Redis connection for publishing");
                    return;
                }
                Err(_) => {
                    error!("Timed out getting Redis connection for publishing");
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

        // Spawn task to handle subscribing with exponential backoff on reconnection
        tokio::spawn(async move {
            let mut backoff_secs = INITIAL_BACKOFF_SECS;

            loop {
                match self_clone.run_subscriber().await {
                    SubscriberExit::Disconnected => {
                        // Connection was healthy before it dropped.
                        // Reset backoff since the server was reachable.
                        error!(
                            "Redis subscriber stream ended (connection lost), reconnecting after {}s",
                            INITIAL_BACKOFF_SECS
                        );
                        backoff_secs = INITIAL_BACKOFF_SECS;
                    }
                    SubscriberExit::ConnectFailed(e) => {
                        // Could not connect -- keep increasing backoff.
                        error!(
                            error = %e,
                            backoff_secs = backoff_secs,
                            "Redis subscriber failed to connect, retrying after backoff"
                        );
                    }
                }

                tokio::time::sleep(Duration::from_secs(backoff_secs)).await;

                // Exponential backoff: double the delay, cap at MAX_BACKOFF_SECS
                backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
            }
        });

        Ok(publish_tx)
    }

    /// Run the subscriber task.
    ///
    /// Returns `SubscriberExit::Disconnected` if the connection was established but then
    /// the stream ended (Redis disconnected). Returns `SubscriberExit::ConnectFailed` if
    /// the initial connection or subscription failed.
    async fn run_subscriber(&self) -> SubscriberExit {
        let mut pubsub = match timeout(
            Duration::from_secs(REDIS_TIMEOUT_SECS),
            self.redis_client.get_async_pubsub(),
        )
        .await
        {
            Ok(Ok(ps)) => ps,
            Ok(Err(e)) => {
                return SubscriberExit::ConnectFailed(
                    anyhow::anyhow!(e).context("Failed to get Redis Pub/Sub connection"),
                );
            }
            Err(_) => {
                return SubscriberExit::ConnectFailed(anyhow::anyhow!(
                    "Timed out getting Redis Pub/Sub connection"
                ));
            }
        };

        // Subscribe to all room channels using pattern
        match timeout(
            Duration::from_secs(REDIS_TIMEOUT_SECS),
            pubsub.psubscribe("room:*"),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return SubscriberExit::ConnectFailed(
                    anyhow::anyhow!(e).context("Failed to subscribe to room:* pattern"),
                );
            }
            Err(_) => {
                return SubscriberExit::ConnectFailed(anyhow::anyhow!(
                    "Timed out subscribing to room:* pattern"
                ));
            }
        }

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

                        // Forward KickPublisher events to admin channel
                        if matches!(&envelope.event, ClusterEvent::KickPublisher { .. }) {
                            let _ = self.admin_event_tx.send(envelope.event.clone());
                        }

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

        // Stream returned None -- the Redis connection was lost
        SubscriberExit::Disconnected
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
        let subscribers: usize = timeout(
            Duration::from_secs(REDIS_TIMEOUT_SECS),
            conn.publish(&channel, payload),
        )
        .await
        .context("Timed out publishing to Redis")?
        .context("Failed to publish to Redis")?;

        Ok(subscribers)
    }
}

/// Describes how the subscriber loop exited, enabling proper backoff behavior.
enum SubscriberExit {
    /// Connection was established and messages were being processed, but the
    /// stream ended (Redis disconnected). Backoff should be reset since the
    /// connection was healthy before it dropped.
    Disconnected,
    /// Failed to connect or subscribe to Redis. Backoff should continue
    /// increasing to avoid hammering an unavailable server.
    ConnectFailed(anyhow::Error),
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
            position: None,
            color: None,
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

        let (admin_tx, _) = broadcast::channel(256);

        // Create two PubSub instances simulating different nodes
        let pubsub1 = Arc::new(
            RedisPubSub::new(redis_url, message_hub.clone(), "node1".to_string(), admin_tx.clone()).unwrap(),
        );
        let pubsub2 = Arc::new(
            RedisPubSub::new(redis_url, message_hub.clone(), "node2".to_string(), admin_tx.clone()).unwrap(),
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
            position: None,
            color: None,
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
