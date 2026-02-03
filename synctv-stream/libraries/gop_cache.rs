// GOP Cache library
//
// Provides Group of Pictures (GOP) caching for instant viewer startup.
// Streams are identified by room_id:media_id keys.

pub mod gop_cache;

pub use gop_cache::{GopCache, GopCacheConfig, GopFrame, FrameType};
