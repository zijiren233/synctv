// HLS protocol implementation
//
// Provides HLS remuxer and HTTP server for HLS streaming.
// Converts RTMP to HLS segments (M3U8 + TS).

pub mod remuxer;
pub mod server;

pub use remuxer::{CustomHlsRemuxer, StreamRegistry, StreamProcessorState, SegmentInfo};
pub use server::HlsServer;
