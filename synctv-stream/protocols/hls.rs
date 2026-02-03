// HLS protocol implementation
//
// Provides HLS remuxer and HTTP server for HLS streaming.
// Converts RTMP to HLS segments (M3U8 + TS).

pub mod hls_remuxer;
pub mod hls_server;

pub use hls_remuxer::{CustomHlsRemuxer, StreamRegistry, StreamProcessorState, SegmentInfo};
pub use hls_server::HlsServer;
