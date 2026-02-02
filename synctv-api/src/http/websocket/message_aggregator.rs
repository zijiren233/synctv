//! Message aggregation strategies for WebSocket batching
//!
//! Provides different strategies for aggregating messages to optimize
//! network throughput and reduce message overhead.

use synctv_proto::server::ServerMessage;
use std::collections::HashMap;

/// Aggregation strategy
#[derive(Debug, Clone, Copy)]
pub enum AggregationStrategy {
    /// No aggregation, send immediately
    None,
    /// Aggregate all messages in time window
    All,
    /// Aggregate only same-type messages
    ByType,
    /// Aggregate only chat messages
    ChatOnly,
    /// Aggregate only status updates (playback, member changes)
    StatusOnly,
}

/// Message aggregator that combines multiple messages into optimized batches
pub struct MessageAggregator {
    strategy: AggregationStrategy,
    max_batch_size: usize,
}

impl MessageAggregator {
    /// Create a new message aggregator
    pub fn new(strategy: AggregationStrategy, max_batch_size: usize) -> Self {
        Self {
            strategy,
            max_batch_size,
        }
    }

    /// Aggregate messages based on the configured strategy
    ///
    /// Returns a list of message batches to send.
    pub fn aggregate(&self, messages: Vec<ServerMessage>) -> Vec<Vec<ServerMessage>> {
        if messages.is_empty() {
            return Vec::new();
        }

        match self.strategy {
            AggregationStrategy::None => messages.into_iter().map(|m| vec![m]).collect(),
            AggregationStrategy::All => {
                // Split into batches of max_batch_size
                messages
                    .chunks(self.max_batch_size)
                    .map(|chunk| chunk.to_vec())
                    .collect()
            }
            AggregationStrategy::ByType => {
                // Group by message type
                let mut groups: HashMap<String, Vec<ServerMessage>> = HashMap::new();

                for msg in messages {
                    let type_key = self.get_type_key(&msg);
                    groups.entry(type_key).or_default().push(msg);
                }

                // Flatten groups into batches
                let mut batches = Vec::new();
                for (_, mut group) in groups {
                    while !group.is_empty() {
                        let chunk_size = group.len().min(self.max_batch_size);
                        let batch: Vec<ServerMessage> = group.drain(..chunk_size).collect();
                        batches.push(batch);
                    }
                }

                batches
            }
            AggregationStrategy::ChatOnly => {
                // Separate chat messages from other messages
                let (chat, other): (Vec<_>, Vec<_>) = messages
                    .into_iter()
                    .partition(|m| matches!(m.r#type, Some(synctv_proto::server::server_message::Type::ChatMessage(_))));

                let mut batches = Vec::new();

                // Batch chat messages
                if !chat.is_empty() {
                    batches.extend(
                        chat.chunks(self.max_batch_size)
                            .map(|chunk| chunk.to_vec())
                            .collect::<Vec<_>>(),
                    );
                }

                // Send other messages individually
                for msg in other {
                    batches.push(vec![msg]);
                }

                batches
            }
            AggregationStrategy::StatusOnly => {
                // Separate status messages from other messages
                let (status, other): (Vec<_>, Vec<_>) = messages
                    .into_iter()
                    .partition(|m| {
                        matches!(
                            m.r#type,
                            Some(synctv_proto::server::server_message::Type::PlaybackState(_))
                                | Some(synctv_proto::server::server_message::Type::MemberJoined(_))
                                | Some(synctv_proto::server::server_message::Type::MemberLeft(_))
                        )
                    });

                let mut batches = Vec::new();

                // Batch status messages
                if !status.is_empty() {
                    batches.extend(
                        status.chunks(self.max_batch_size)
                            .map(|chunk| chunk.to_vec())
                            .collect::<Vec<_>>(),
                    );
                }

                // Send other messages individually
                for msg in other {
                    batches.push(vec![msg]);
                }

                batches
            }
        }
    }

    /// Get a type key for grouping messages
    fn get_type_key(&self, msg: &ServerMessage) -> String {
        match &msg.r#type {
            Some(synctv_proto::server::server_message::Type::ChatMessage(_)) => "chat".to_string(),
            Some(synctv_proto::server::server_message::Type::PlaybackState(_)) => "playback".to_string(),
            Some(synctv_proto::server::server_message::Type::MemberJoined(_)) => "member_joined".to_string(),
            Some(synctv_proto::server::server_message::Type::MemberLeft(_)) => "member_left".to_string(),
            Some(synctv_proto::server::server_message::Type::Error(_)) => "error".to_string(),
            None => "unknown".to_string(),
        }
    }

    /// Estimate the size of a batch in bytes
    pub fn estimate_batch_size(&self, batch: &[ServerMessage]) -> usize {
        batch
            .iter()
            .map(|msg| {
                // Rough estimate: each message has overhead plus encoded size
                let mut buf = Vec::new();
                if let Ok(_) = msg.encode(&mut buf) {
                    buf.len()
                } else {
                    // Fallback estimate
                    100
                }
            })
            .sum()
    }
}

/// Optimize message order within a batch
///
/// Reorders messages to prioritize important messages and improve
/// user experience.
pub fn optimize_message_order(messages: &mut Vec<ServerMessage>) {
    // Sort by priority (errors first, then playback state, then others)
    messages.sort_by(|a, b| {
        let priority_a = get_message_priority(a);
        let priority_b = get_message_priority(b);
        priority_a.cmp(&priority_b).then_with(|| {
            // Within same priority, maintain original order (stable sort)
            std::cmp::Ordering::Equal
        })
    });
}

/// Get message priority (lower number = higher priority)
fn get_message_priority(msg: &ServerMessage) -> u8 {
    match &msg.r#type {
        Some(synctv_proto::server::server_message::Type::Error(_)) => 0,
        Some(synctv_proto::server::server_message::Type::PlaybackState(_)) => 1,
        Some(synctv_proto::server::server_message::Type::MemberJoined(_)) => 2,
        Some(synctv_proto::server::server_message::Type::MemberLeft(_)) => 3,
        Some(synctv_proto::server::server_message::Type::ChatMessage(_)) => 4,
        None => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synctv_proto::server;

    fn create_chat_message(content: &str) -> ServerMessage {
        ServerMessage {
            r#type: Some(server::server_message::Type::ChatMessage(server::ChatMessage {
                room_id: "room1".to_string(),
                user_id: "user1".to_string(),
                username: "test".to_string(),
                content: content.to_string(),
            })),
        }
    }

    fn create_playback_message() -> ServerMessage {
        ServerMessage {
            r#type: Some(server::server_message::Type::PlaybackState(server::PlaybackState {
                room_id: "room1".to_string(),
                is_playing: true,
                current_time: 100.0,
                playback_rate: 1.0,
            })),
        }
    }

    #[test]
    fn test_aggregate_none() {
        let aggregator = MessageAggregator::new(AggregationStrategy::None, 10);
        let messages = vec![create_chat_message("hello"), create_chat_message("world")];

        let batches = aggregator.aggregate(messages);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 1);
        assert_eq!(batches[1].len(), 1);
    }

    #[test]
    fn test_aggregate_all() {
        let aggregator = MessageAggregator::new(AggregationStrategy::All, 10);
        let messages = vec![
            create_chat_message("hello"),
            create_chat_message("world"),
            create_playback_message(),
        ];

        let batches = aggregator.aggregate(messages);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 3);
    }

    #[test]
    fn test_aggregate_by_type() {
        let aggregator = MessageAggregator::new(AggregationStrategy::ByType, 10);
        let messages = vec![
            create_chat_message("hello"),
            create_playback_message(),
            create_chat_message("world"),
        ];

        let batches = aggregator.aggregate(messages);
        assert_eq!(batches.len(), 2); // One for chat, one for playback
    }

    #[test]
    fn test_aggregate_chat_only() {
        let aggregator = MessageAggregator::new(AggregationStrategy::ChatOnly, 10);
        let messages = vec![
            create_chat_message("hello"),
            create_playback_message(),
            create_chat_message("world"),
        ];

        let batches = aggregator.aggregate(messages);
        assert_eq!(batches.len(), 2); // One batch for chat messages, playback sent individually

        // First batch should have 2 chat messages
        assert_eq!(batches[0].len(), 2);

        // Second batch should have 1 playback message
        assert_eq!(batches[1].len(), 1);
    }

    #[test]
    fn test_optimize_message_order() {
        let mut messages = vec![
            create_chat_message("hello"),
            create_playback_message(),
            create_chat_message("world"),
        ];

        optimize_message_order(&mut messages);

        // Playback state should come first (higher priority)
        assert!(matches!(
            messages[0].r#type,
            Some(server::server_message::Type::PlaybackState(_))
        ));
    }
}
