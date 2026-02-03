// RTMP protocol implementation
//
// Provides RTMP server and protocol handling for live streaming.
// Uses xiu's RTMP library with custom authentication.
//
// Stream identifier format: room_id:media_id

pub mod rtmp_handler;
pub mod rtmp_server;

pub use rtmp_server::RtmpServer;
