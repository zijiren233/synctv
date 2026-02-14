use anyhow::{Context, Result};
use futures::stream::StreamExt;
use redis::{AsyncCommands, Client as RedisClient};
use redis::streams::StreamReadReply;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{timeout, Duration};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// Timeout for Redis operations in seconds
const REDIS_TIMEOUT_SECS: u64 = 5;

/// Initial backoff delay for subscriber reconnection
const INITIAL_BACKOFF_SECS: u64 = 1;

/// Maximum backoff delay for subscriber reconnection
const MAX_BACKOFF_SECS: u64 = 30;

/// Redis Stream key for reliable event delivery
const EVENT_STREAM_KEY: &str = "synctv:events:stream";
/// Max length of the event stream (approximate)
const MAX_STREAM_LENGTH: usize = 10000;

use super::dedup::{DedupKey, MessageDeduplicator};
use super::events::ClusterEvent;
use super::room_hub::RoomMessageHub;
use synctv_core::models::id::RoomId;
use synctv_core::service::PermissionService;

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
    permission_service: Option<PermissionService>,
    deduplicator: Arc<MessageDeduplicator>,
    cancel_token: CancellationToken,
}

impl RedisPubSub {
    /// Create a new `RedisPubSub` service
    pub fn new(
        redis_url: &str,
        message_hub: Arc<RoomMessageHub>,
        node_id: String,
        admin_event_tx: broadcast::Sender<ClusterEvent>,
        permission_service: Option<PermissionService>,
        deduplicator: Arc<MessageDeduplicator>,
    ) -> Result<Self> {
        let redis_client = RedisClient::open(redis_url).context("Failed to create Redis client")?;

        Ok(Self {
            redis_client,
            message_hub,
            node_id,
            admin_event_tx,
            permission_service,
            deduplicator,
            cancel_token: CancellationToken::new(),
        })
    }

    /// Get the cancellation token for external shutdown signaling
    #[must_use]
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }

    /// Shut down the Pub/Sub service (cancels subscriber and publisher tasks)
    pub fn shutdown(&self) {
        info!("Shutting down RedisPubSub service");
        self.cancel_token.cancel();
    }

    /// Capacity for the publish channel. Events are dropped with a warning when full
    /// (e.g., during a prolonged Redis outage).
    pub const PUBLISH_CHANNEL_CAPACITY: usize = 10_000;

    /// Start the Pub/Sub service
    /// This spawns a background task that subscribes to all room channels
    pub async fn start(self: Arc<Self>) -> Result<mpsc::Sender<PublishRequest>> {
        // Create bounded channel for publishing events to prevent OOM under Redis outage
        let (publish_tx, mut publish_rx) = mpsc::channel::<PublishRequest>(Self::PUBLISH_CHANNEL_CAPACITY);

        // Clone for the publish task
        let publish_client = self.redis_client.clone();
        let node_id = self.node_id.clone();
        let cancel_publisher = self.cancel_token.clone();

        // Spawn task to handle publishing with reconnection logic
        tokio::spawn(async move {
            let mut backoff_secs = INITIAL_BACKOFF_SECS;
            // Buffer for retrying a failed publish after reconnection
            let mut retry_request: Option<PublishRequest> = None;

            loop {
                let conn = match timeout(
                    Duration::from_secs(REDIS_TIMEOUT_SECS),
                    publish_client.get_multiplexed_async_connection(),
                )
                .await
                {
                    Ok(Ok(conn)) => {
                        backoff_secs = INITIAL_BACKOFF_SECS;
                        conn
                    }
                    Ok(Err(e)) => {
                        error!(
                            error = %e,
                            backoff_secs = backoff_secs,
                            "Failed to get Redis connection for publishing, retrying"
                        );
                        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
                        continue;
                    }
                    Err(_) => {
                        error!(
                            backoff_secs = backoff_secs,
                            "Timed out getting Redis connection for publishing, retrying"
                        );
                        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
                        continue;
                    }
                };

                info!("Redis publisher task (re)connected");
                let mut conn = conn;

                // Retry the previously failed publish request if any
                if let Some(req) = retry_request.take() {
                    let event_type = req.event.event_type();
                    match Self::publish_event(&mut conn, &node_id, req.event.clone()).await {
                        Ok(subscribers) => {
                            debug!(
                                event_type = event_type,
                                subscribers = subscribers,
                                "Retried event published to Redis"
                            );
                        }
                        Err(e) => {
                            warn!(
                                error = %e,
                                event_type = event_type,
                                "Retry publish failed, will retry after next reconnect"
                            );
                            // Put request back for another attempt after reconnection
                            retry_request = Some(req);
                            tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                            backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
                            continue;
                        }
                    }
                }

                // Process events until connection breaks or cancelled
                loop {
                    let req = tokio::select! {
                        _ = cancel_publisher.cancelled() => {
                            info!("Redis publisher task cancelled");
                            return;
                        }
                        req = publish_rx.recv() => req,
                    };
                    if let Some(req) = req {
                        let event_type = req.event.event_type();
                        match Self::publish_event(&mut conn, &node_id, req.event.clone()).await {
                            Ok(subscribers) => {
                                debug!(
                                    event_type = event_type,
                                    subscribers = subscribers,
                                    "Event published to Redis"
                                );
                            }
                            Err(e) => {
                                error!(
                                    error = %e,
                                    event_type = event_type,
                                    "Failed to publish event, saving for retry after reconnect"
                                );
                                // Save failed request for retry after reconnection
                                retry_request = Some(req);
                                break;
                            }
                        }
                    } else {
                        // Channel closed, publisher shutting down
                        warn!("Redis publisher channel closed, exiting");
                        return;
                    }
                }

                // Wait before reconnecting
                tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
            }
        });

        // Clone for the subscriber task
        let self_clone = self;
        let cancel_subscriber = self_clone.cancel_token.clone();

        // Spawn task to handle subscribing with exponential backoff on reconnection
        tokio::spawn(async move {
            let mut backoff_secs = INITIAL_BACKOFF_SECS;
            // Track the last processed Redis Stream ID across reconnections.
            // "$" means "start from latest" on first connection (no catch-up for
            // events before the process existed). After processing real entries
            // this is updated to the actual stream ID so reconnections can catch up.
            let mut last_stream_id = "$".to_string();

            loop {
                // Check cancellation before each reconnect attempt
                if cancel_subscriber.is_cancelled() {
                    info!("Redis subscriber task cancelled");
                    return;
                }

                match self_clone.run_subscriber(&mut last_stream_id).await {
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

                // Wait with cancellation support
                tokio::select! {
                    _ = cancel_subscriber.cancelled() => {
                        info!("Redis subscriber task cancelled during backoff");
                        return;
                    }
                    _ = tokio::time::sleep(Duration::from_secs(backoff_secs)) => {}
                }

                // Exponential backoff: double the delay, cap at MAX_BACKOFF_SECS
                backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
            }
        });

        Ok(publish_tx)
    }

    /// Run the subscriber task.
    ///
    /// `last_stream_id` tracks the last processed Redis Stream entry ID. On
    /// first connection it should be `"$"` (start from latest). After
    /// reconnection the subscriber uses it to catch up on missed events. The
    /// value is updated in-place as stream entries are processed so it persists
    /// across reconnections.
    ///
    /// Returns `SubscriberExit::Disconnected` if the connection was established but then
    /// the stream ended (Redis disconnected). Returns `SubscriberExit::ConnectFailed` if
    /// the initial connection or subscription failed.
    async fn run_subscriber(&self, last_stream_id: &mut String) -> SubscriberExit {
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

        // Subscribe to all room channels and admin channel using patterns
        match timeout(
            Duration::from_secs(REDIS_TIMEOUT_SECS),
            pubsub.psubscribe(&["synctv:room:*", "synctv:admin:*"]),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return SubscriberExit::ConnectFailed(
                    anyhow::anyhow!(e).context("Failed to subscribe to synctv:room:*/synctv:admin:* patterns"),
                );
            }
            Err(_) => {
                return SubscriberExit::ConnectFailed(anyhow::anyhow!(
                    "Timed out subscribing to synctv:room:*/synctv:admin:* patterns"
                ));
            }
        }

        info!("Redis subscriber connected, listening to synctv:room:* and synctv:admin:* channels");

        if *last_stream_id == "$" {
            // First connection: snapshot the current stream tip so we can catch
            // up from this point if the connection drops later.
            match self.get_latest_stream_id().await {
                Ok(Some(id)) => {
                    info!(stream_id = %id, "Initialized stream cursor from current tip");
                    *last_stream_id = id;
                }
                Ok(None) => {
                    // Stream is empty or doesn't exist yet; use "0" so any
                    // future entries will be caught on reconnection.
                    *last_stream_id = "0".to_string();
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        "Failed to read latest stream ID, using '0' as fallback"
                    );
                    *last_stream_id = "0".to_string();
                }
            }
        } else {
            // Reconnection: catch up on events missed during disconnection.
            match self.read_missed_events(last_stream_id).await {
                Ok(events) => {
                    if !events.is_empty() {
                        info!(
                            count = events.len(),
                            from_id = %last_stream_id,
                            "Catching up on missed events from Redis Stream"
                        );
                        for (stream_id, channel, event) in events {
                            self.dispatch_event(&channel, event).await;
                            *last_stream_id = stream_id;
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        "Failed to read missed events from Redis Stream, continuing with live events"
                    );
                }
            }
        }

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

                    self.dispatch_event(&channel, envelope.event).await;
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

    /// Dispatch a single event received from Redis (either live or from catch-up).
    ///
    /// Handles deduplication, admin channel routing, permission cache invalidation,
    /// and local broadcast to room subscribers.
    async fn dispatch_event(&self, channel: &str, event: ClusterEvent) {
        // Deduplicate events (prevents duplicate delivery during catch-up + live overlap)
        let dedup_key = DedupKey::from_event(&event);
        if !self.deduplicator.should_process(&dedup_key) {
            debug!(
                channel = %channel,
                event_type = %event.event_type(),
                "Skipping duplicate event from Redis"
            );
            return;
        }

        debug!(
            channel = %channel,
            event_type = %event.event_type(),
            "Dispatching event from Redis"
        );

        // Handle admin channel events (no room_id)
        if channel.starts_with("synctv:admin:") {
            let _ = self.admin_event_tx.send(event);
            return;
        }

        // Extract room_id from channel name (synctv:room:{room_id})
        if let Some(room_id_str) = channel.strip_prefix("synctv:room:") {
            let room_id = RoomId::from_string(room_id_str.to_string());

            // Forward KickPublisher events to admin channel
            if matches!(&event, ClusterEvent::KickPublisher { .. }) {
                let _ = self.admin_event_tx.send(event.clone());
            }

            // Invalidate local permission cache for cross-replica consistency
            if let Some(ref perm_svc) = self.permission_service {
                match &event {
                    ClusterEvent::PermissionChanged { target_user_id, .. } => {
                        perm_svc.invalidate_cache(&room_id, target_user_id).await;
                        debug!(
                            room_id = %room_id.as_str(),
                            user_id = %target_user_id.as_str(),
                            "Invalidated permission cache (cross-replica)"
                        );
                    }
                    ClusterEvent::UserLeft { user_id, .. } => {
                        perm_svc.invalidate_cache(&room_id, user_id).await;
                        debug!(
                            room_id = %room_id.as_str(),
                            user_id = %user_id.as_str(),
                            "Invalidated permission cache on UserLeft (cross-replica)"
                        );
                    }
                    ClusterEvent::RoomSettingsChanged { .. } => {
                        perm_svc.invalidate_room_cache(&room_id).await;
                        debug!(
                            room_id = %room_id.as_str(),
                            "Invalidated room permission cache (cross-replica)"
                        );
                    }
                    _ => {}
                }
            }

            // Route WebRTC signaling to the specific target connection instead of
            // broadcasting to all subscribers. The `to` field is formatted as
            // "user_id:conn_id" -- we parse the conn_id and use targeted delivery.
            if let ClusterEvent::WebRTCSignaling { ref to, .. } = event {
                let to_owned = to.clone();
                // Parse "user_id:conn_id" format
                if let Some((_target_user, target_conn)) = to_owned.rsplit_once(':') {
                    let target_conn = target_conn.to_string();
                    let sent = self.message_hub.broadcast_to_connection(
                        &room_id,
                        &target_conn,
                        event,
                    );
                    debug!(
                        room_id = %room_id.as_str(),
                        target_connection = %target_conn,
                        sent = sent,
                        "Routed WebRTC signaling to specific connection"
                    );
                } else {
                    // Fallback: if `to` doesn't contain ':', broadcast to user
                    let target_user_id = synctv_core::models::UserId::from_string(to_owned.clone());
                    let sent = self.message_hub.broadcast_to_user(&room_id, &target_user_id, event);
                    debug!(
                        room_id = %room_id.as_str(),
                        target_user = %to_owned,
                        sent = sent,
                        "Routed WebRTC signaling to user (no conn_id)"
                    );
                }
                return;
            }

            // Broadcast to local subscribers
            let sent_count = self.message_hub.broadcast(&room_id, event);

            debug!(
                room_id = %room_id.as_str(),
                local_subscribers = sent_count,
                "Forwarded Redis event to local subscribers"
            );
        } else {
            warn!(channel = %channel, "Invalid channel format");
        }
    }

    /// Publish an event to Redis
    ///
    /// Uses both Pub/Sub (for real-time delivery) and Stream (for reliability).
    /// If a subscriber disconnects, it can catch up by reading from the stream.
    async fn publish_event(
        conn: &mut redis::aio::MultiplexedConnection,
        node_id: &str,
        event: ClusterEvent,
    ) -> Result<usize> {
        // Events with a room_id go to synctv:room:{room_id}, admin-only events go to synctv:admin:events
        let channel = if let Some(room_id) = event.room_id() {
            format!("synctv:room:{}", room_id.as_str())
        } else {
            "synctv:admin:events".to_string()
        };

        // Wrap event in envelope with node_id
        let envelope = EventEnvelope {
            node_id: node_id.to_string(),
            event,
        };

        let payload =
            serde_json::to_string(&envelope).context("Failed to serialize event envelope")?;

        // 1. Add to Redis Stream for reliable delivery (catch-up after disconnect)
        // Using XADD with MAXLEN to prevent unbounded growth
        use redis::streams::StreamMaxlen;
        let stream_result = timeout(
            Duration::from_secs(REDIS_TIMEOUT_SECS),
            conn.xadd_maxlen::<_, _, _, _, String>(
                EVENT_STREAM_KEY,
                StreamMaxlen::Approx(MAX_STREAM_LENGTH),
                "*",  // Auto-generate ID
                &[("channel", channel.as_str()), ("payload", payload.as_str())],
            ),
        ).await;

        if let Err(e) = &stream_result {
            warn!(error = ?e, "Failed to add event to Redis Stream (non-critical)");
            // Continue with Pub/Sub even if Stream fails
        }

        // 2. Publish to Pub/Sub for real-time delivery
        let subscribers: usize = timeout(
            Duration::from_secs(REDIS_TIMEOUT_SECS),
            conn.publish(&channel, &payload),
        )
        .await
        .context("Timed out publishing to Redis")?
        .context("Failed to publish to Redis")?;

        Ok(subscribers)
    }

    /// Get the ID of the latest entry in the Redis Stream, or `None` if the
    /// stream is empty / does not exist. Used on first connection to snapshot
    /// the cursor so subsequent reconnections can catch up.
    async fn get_latest_stream_id(&self) -> Result<Option<String>> {
        use redis::streams::StreamRangeReply;

        let mut conn = self.redis_client
            .get_multiplexed_async_connection()
            .await
            .context("Failed to get Redis connection for stream info")?;

        // XREVRANGE key + - COUNT 1  →  returns the single newest entry
        let reply: StreamRangeReply = timeout(
            Duration::from_secs(REDIS_TIMEOUT_SECS),
            conn.xrevrange_count(EVENT_STREAM_KEY, "+", "-", 1usize),
        )
        .await
        .context("Timed out reading latest stream ID")?
        .context("Failed to read latest stream ID")?;

        Ok(reply.ids.into_iter().next().map(|entry| entry.id))
    }

    /// Maximum number of catch-up iterations to prevent infinite loops
    const MAX_CATCHUP_ITERATIONS: usize = 10;
    /// Number of events to read per XREAD call during catch-up
    const CATCHUP_BATCH_SIZE: usize = 1000;

    /// Read missed events from Redis Stream after reconnection.
    ///
    /// Loops XREAD until no more events are returned (or up to `MAX_CATCHUP_ITERATIONS`
    /// iterations) to ensure complete catch-up even when > 1000 events were missed.
    ///
    /// Returns a list of `(stream_id, channel, event)` tuples for events that
    /// occurred after `last_id`. The caller should update its tracked stream ID
    /// to the last returned `stream_id`.
    async fn read_missed_events(
        &self,
        last_id: &str,
    ) -> Result<Vec<(String, String, ClusterEvent)>> {
        let mut conn = self.redis_client
            .get_multiplexed_async_connection()
            .await
            .context("Failed to get Redis connection for catch-up")?;

        let mut events = Vec::new();
        let mut cursor = last_id.to_string();

        for iteration in 0..Self::MAX_CATCHUP_ITERATIONS {
            // XREAD returns StreamReadReply { keys: Vec<StreamKey> }
            // Each StreamKey has `key` (the stream name) and `ids: Vec<StreamId>`.
            // Each StreamId has `id` (e.g. "1234567890-0") and `map: HashMap<String, Value>`.
            let reply: StreamReadReply = timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                conn.xread_options(
                    &[EVENT_STREAM_KEY],
                    &[&cursor],
                    &redis::streams::StreamReadOptions::default().count(Self::CATCHUP_BATCH_SIZE),
                ),
            )
            .await
            .context("Timed out reading from Redis Stream")?
            .context("Failed to read from Redis Stream")?;

            let mut batch_count = 0;
            for stream_key in reply.keys {
                for entry in stream_key.ids {
                    batch_count += 1;
                    // Update cursor to the latest processed ID
                    cursor = entry.id.clone();

                    // Extract "channel" and "payload" fields from the stream entry
                    let channel = entry.map.get("channel")
                        .and_then(|v| redis::from_redis_value::<String>(v).ok());
                    let payload = entry.map.get("payload")
                        .and_then(|v| redis::from_redis_value::<String>(v).ok());

                    if let (Some(chan), Some(payload_str)) = (channel, payload) {
                        match serde_json::from_str::<EventEnvelope>(&payload_str) {
                            Ok(envelope) => {
                                // Skip events from this node
                                if envelope.node_id != self.node_id {
                                    events.push((entry.id, chan, envelope.event));
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to parse event envelope from stream");
                            }
                        }
                    }
                }
            }

            if batch_count < Self::CATCHUP_BATCH_SIZE {
                // No more events to read
                break;
            }

            if iteration == Self::MAX_CATCHUP_ITERATIONS - 1 {
                warn!(
                    total_events = events.len(),
                    "Catch-up reached max iterations ({}), some events may be missed",
                    Self::MAX_CATCHUP_ITERATIONS
                );
            }
        }

        Ok(events)
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

/// Request to publish an event.
/// `room_id` is used for room-scoped events; admin events (e.g., `KickUser`) set it to `None`.
pub struct PublishRequest {
    pub room_id: Option<RoomId>,
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
            event_id: nanoid::nanoid!(16),
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
        let dedup1 = Arc::new(MessageDeduplicator::with_defaults());
        let dedup2 = Arc::new(MessageDeduplicator::with_defaults());
        let pubsub1 = Arc::new(
            RedisPubSub::new(redis_url, message_hub.clone(), "node1".to_string(), admin_tx.clone(), None, dedup1).unwrap(),
        );
        let pubsub2 = Arc::new(
            RedisPubSub::new(redis_url, message_hub.clone(), "node2".to_string(), admin_tx.clone(), None, dedup2).unwrap(),
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
            event_id: nanoid::nanoid!(16),
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
                room_id: Some(room_id.clone()),
                event,
            })
            .await
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
