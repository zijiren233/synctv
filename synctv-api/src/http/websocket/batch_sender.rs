//! Batch WebSocket message sender
//!
//! Aggregates multiple messages into batches to reduce network overhead and
//! improve throughput. Messages are aggregated over a time window and sent
//! as a single batch.

use prost::Message as ProstMessage;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, trace, warn};

use synctv_proto::server::ServerMessage;

/// Configuration for batch message sender
#[derive(Debug, Clone)]
pub struct BatchSenderConfig {
    /// Maximum time to wait before flushing a batch (in milliseconds)
    pub max_batch_interval_ms: u64,
    /// Maximum number of messages per batch
    pub max_batch_size: usize,
    /// Channel buffer size
    pub channel_buffer_size: usize,
}

impl Default for BatchSenderConfig {
    fn default() -> Self {
        Self {
            max_batch_interval_ms: 50, // 50ms
            max_batch_size: 100,
            channel_buffer_size: 1000,
        }
    }
}

/// Message with timestamp for batching
#[derive(Debug, Clone)]
struct TimestampedMessage {
    message: ServerMessage,
    enqueued_at: Instant,
}

/// Batch statistics
#[derive(Debug, Clone, Default)]
pub struct BatchStats {
    pub total_messages_sent: u64,
    pub total_batches_sent: u64,
    pub total_messages_aggregated: u64,
    pub average_batch_size: f64,
}

/// Batch sender for WebSocket messages
///
/// Aggregates messages over a time window and sends them as batches.
pub struct BatchSender {
    config: BatchSenderConfig,
    tx: mpsc::UnboundedSender<ServerMessage>,
    stats: Arc<RwLock<BatchStats>>,
}

impl BatchSender {
    /// Create a new batch sender
    ///
    /// # Arguments
    /// * `sender` - The actual WebSocket sender (must implement BatchSend)
    /// * `config` - Batch configuration
    pub fn new<S>(sender: S, config: BatchSenderConfig) -> Self
    where
        S: BatchSend + Send + 'static,
    {
        let (tx, rx) = mpsc::unbounded_channel();
        let stats = Arc::new(RwLock::new(BatchStats::default()));

        let stats_clone = stats.clone();
        tokio::spawn(async move {
            batch_sender_task(sender, rx, config, stats_clone).await;
        });

        Self { config, tx, stats }
    }

    /// Create a batch sender with default configuration
    pub fn with_defaults<S>(sender: S) -> Self
    where
        S: BatchSend + Send + 'static,
    {
        Self::new(sender, BatchSenderConfig::default())
    }

    /// Send a message (will be batched)
    ///
    /// Returns an error if the sender channel is closed.
    pub fn send(&self, message: ServerMessage) -> Result<(), mpsc::error::SendError<ServerMessage>> {
        self.tx.send(message)?;
        Ok(())
    }

    /// Get batch statistics
    pub async fn stats(&self) -> BatchStats {
        self.stats.read().await.clone()
    }
}

/// Trait for types that can send batched messages
pub trait BatchSend: Send {
    /// Send a batch of messages
    fn send_batch(&self, messages: Vec<ServerMessage>) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Batch sender task
///
/// Receives messages from the channel, aggregates them into batches,
/// and sends them using the underlying sender.
async fn batch_sender_task<S>(
    sender: S,
    mut rx: mpsc::UnboundedReceiver<ServerMessage>,
    config: BatchSenderConfig,
    stats: Arc<RwLock<BatchStats>>,
) where
    S: BatchSend,
{
    let mut buffer = Vec::with_capacity(config.max_batch_size);
    let mut interval = tokio::time::interval(Duration::from_millis(config.max_batch_interval_ms));
    let mut ticker = interval.tick();

    loop {
        tokio::select! {
            // Receive message
            result = rx.recv() => {
                match result {
                    Some(message) => {
                        trace!("BatchSender: Received message");
                        buffer.push(TimestampedMessage {
                            message,
                            enqueued_at: Instant::now(),
                        });

                        // Flush if buffer is full
                        if buffer.len() >= config.max_batch_size {
                            flush_buffer(&sender, &mut buffer, &stats).await;
                            ticker = interval.tick(); // Reset interval
                        }
                    }
                    None => {
                        // Channel closed, flush remaining messages and exit
                        debug!("BatchSender: Channel closed, flushing remaining messages");
                        if !buffer.is_empty() {
                            flush_buffer(&sender, &mut buffer, &stats).await;
                        }
                        break;
                    }
                }
            }
            // Interval elapsed
            _ = &mut ticker => {
                if !buffer.is_empty() {
                    trace!("BatchSender: Interval elapsed, flushing buffer ({} messages)", buffer.len());
                    flush_buffer(&sender, &mut buffer, &stats).await;
                }
            }
        }
    }
}

/// Flush the buffer to the underlying sender
async fn flush_buffer<S>(
    sender: &S,
    buffer: &mut Vec<TimestampedMessage>,
    stats: &Arc<RwLock<BatchStats>>,
) where
    S: BatchSend,
{
    if buffer.is_empty() {
        return;
    }

    let batch_size = buffer.len();
    let messages: Vec<ServerMessage> = buffer.drain(..).map(|m| m.message).collect();

    // Calculate latency statistics
    let now = Instant::now();
    let total_latency_ms: u64 = messages
        .iter()
        .map(|_| {
            // We can't calculate actual latency here since we drained the buffer,
            // but we could track it if needed
            0
        })
        .sum();

    // Send batch
    match sender.send_batch(messages) {
        Ok(()) => {
            trace!("BatchSender: Sent batch of {} messages", batch_size);

            // Update statistics
            let mut stats_guard = stats.write().await;
            stats_guard.total_batches_sent += 1;
            stats_guard.total_messages_sent += batch_size as u64;
            stats_guard.total_messages_aggregated += batch_size as u64;
            stats_guard.average_batch_size = stats_guard.total_messages_sent as f64
                / stats_guard.total_batches_sent as f64;
        }
        Err(e) => {
            warn!("BatchSender: Failed to send batch: {}", e);
        }
    }
}

/// Wrapper for Axum WebSocket sender to implement BatchSend
pub struct AxumBatchSender {
    sender: futures::stream::SplitSink<axum::extract::ws::WebSocket, axum::extract::ws::Message>,
}

impl AxumBatchSender {
    pub fn new(
        sender: futures::stream::SplitSink<axum::extract::ws::WebSocket, axum::extract::ws::Message>,
    ) -> Self {
        Self { sender }
    }
}

impl BatchSend for AxumBatchSender {
    fn send_batch(&self, messages: Vec<ServerMessage>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for message in messages {
            let mut buf = Vec::new();
            message.encode(&mut buf)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

            use futures::SinkExt;
            let mut sender = self.sender.clone();
            tokio::spawn(async move {
                if let Err(e) = sender.send(axum::extract::ws::Message::Binary(buf)).await {
                    warn!("Failed to send WebSocket message: {}", e);
                }
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockSender {
        batches: Arc<RwLock<Vec<Vec<ServerMessage>>>>,
    }

    impl MockSender {
        fn new() -> Self {
            Self {
                batches: Arc::new(RwLock::new(Vec::new())),
            }
        }

        async fn get_batches(&self) -> Vec<Vec<ServerMessage>> {
            self.batches.read().await.clone()
        }
    }

    impl BatchSend for MockSender {
        fn send_batch(&self, messages: Vec<ServerMessage>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            tokio::spawn({
                let batches = self.batches.clone();
                async move {
                    let mut guard = batches.write().await;
                    guard.push(messages);
                }
            });
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_batch_aggregation() {
        let sender = MockSender::new();
        let config = BatchSenderConfig {
            max_batch_interval_ms: 100,
            max_batch_size: 5,
            channel_buffer_size: 100,
        };

        let batch_sender = BatchSender::new(sender, config);

        // Send messages
        for _ in 0..10 {
            let msg = ServerMessage {
                r#type: Some(synctv_proto::server::server_message::Type::ChatMessage(
                    synctv_proto::server::ChatMessage {
                        room_id: "room1".to_string(),
                        user_id: "user1".to_string(),
                        username: "test".to_string(),
                        content: "hello".to_string(),
                    },
                )),
            };
            batch_sender.send(msg).unwrap();
        }

        // Wait for batches to be sent
        tokio::time::sleep(Duration::from_millis(200)).await;

        let stats = batch_sender.stats().await;
        assert!(stats.total_messages_sent >= 10);
    }
}
