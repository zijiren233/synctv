// synctv-livestream - Live streaming infrastructure for SyncTV
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
pub mod xiu_integration;
pub mod error;

// Libraries (defined in sibling directory)
#[path = "../libraries/mod.rs"]
pub mod libraries;

// Protocols (defined in sibling directory)
#[path = "../protocols/mod.rs"]
pub mod protocols;

// Relay (defined in sibling directory)
#[path = "../relay/mod.rs"]
pub mod relay;

// API (defined in sibling directory)
#[path = "../api/mod.rs"]
pub mod api;

// Server orchestration (in src/)
pub mod livestream;

// Re-exports for convenience
pub use libraries::gop_cache::GopCache;
pub use libraries::storage::HlsStorage;
pub use xiu_integration::RtmpServer;
pub use protocols::httpflv::HttpFlvSession;
pub use protocols::hls::{HlsServer, CustomHlsRemuxer, StreamRegistry};
pub use api::{LiveStreamingInfrastructure, FlvStreamingApi, HlsStreamingApi};
pub use livestream::{LivestreamServer, PullStreamManager, SegmentManager};
