// Livestream server orchestration
//
// Main application layer that coordinates all protocols and libraries.
// Follows xiu's application/xiu architecture.

pub mod server;
pub mod managed_stream;
pub mod pull_stream;
pub mod pull_manager;
pub mod external_publish_manager;
pub mod segment_manager;
pub mod external_puller;

pub use server::{LivestreamServer, LivestreamConfig, LivestreamHandle};
pub use pull_manager::PullStreamManager;
pub use external_publish_manager::ExternalPublishManager;
pub use segment_manager::{SegmentManager, CleanupConfig};

// Re-export from protocols
pub use crate::protocols::httpflv::HttpFlvSession;
