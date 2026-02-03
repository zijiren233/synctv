// Libraries - Shared foundational components
//
// Provides common components used across different streaming protocols:
// - GOP cache for instant playback
// - Storage abstraction for HLS segments

pub mod gop_cache;
pub mod storage;

pub use gop_cache::{GopCache, GopCacheConfig, GopFrame, FrameType};
pub use storage::{HlsStorage, StorageBackend, FileStorage, MemoryStorage, OssStorage, OssConfig};
