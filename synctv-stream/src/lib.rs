// synctv-stream - Live streaming infrastructure for SyncTV
//
// Architecture (following xiu's modular design):
// - protocols/   - Protocol implementations (RTMP, HTTP-FLV, HLS)
// - libraries/    - Shared components (GOP cache, storage, etc.)
// - api/         - Public API for synctv-api
// - relay/       - Multi-node streaming (Publisher/Puller)
// - src/         - Server orchestration (application layer)
//
// All streams are scoped to room_id:media_id (media-level streaming).

pub mod grpc;
pub mod rtmp;
pub mod xiu_integration;
pub mod error;

// Libraries
pub mod libraries {
    pub mod gop_cache;
    pub mod storage;
}

// Protocols
pub mod protocols {
    pub mod rtmp;
    pub mod httpflv;
    pub mod hls;
}

// Relay (multi-node)
pub mod relay;

// API for synctv-api
pub mod api;

// Server orchestration
pub mod streaming;

// Re-exports for convenience
pub use libraries::gop_cache::GopCache;
pub use libraries::storage::HlsStorage;
pub use protocols::rtmp::RtmpServer;
pub use protocols::httpflv::HttpFlvSession;
pub use protocols::hls::{HlsServer, CustomHlsRemuxer, StreamRegistry};
pub use api::{LiveStreamingInfrastructure, FlvStreamingApi, HlsStreamingApi};
pub use streaming::{StreamingServer, PullStreamManager, SegmentManager};

// xiu integration
pub use xiu_integration::RtmpStreamingServer;
