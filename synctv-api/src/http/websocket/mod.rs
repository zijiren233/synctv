//! WebSocket message handling with batching support

pub mod batch_sender;
pub mod message_aggregator;

pub use batch_sender::{BatchSender, BatchSenderConfig, BatchStats};
pub use message_aggregator::{MessageAggregator, AggregationStrategy};
