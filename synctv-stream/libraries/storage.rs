// Storage library
//
// Provides storage abstraction for HLS segments.
// Supports multiple backends: file, memory, OSS.

pub mod file;
pub mod memory;
pub mod oss;
pub mod storage;

pub use storage::{HlsStorage, StorageBackend};
