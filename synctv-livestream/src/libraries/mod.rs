// Libraries - Shared foundational components
//
// Provides common components used across different streaming protocols:
// - Storage abstraction for HLS segments

pub mod storage;

pub use storage::{HlsStorage, StorageBackend, FileStorage, MemoryStorage, OssStorage, OssConfig};
