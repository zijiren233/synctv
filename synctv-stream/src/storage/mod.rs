// HLS Storage abstraction layer
//
// Supports multiple storage backends:
// - FileStorage: Local filesystem (default)
// - MemoryStorage: In-memory (for testing/caching)
// - OssStorage: Object storage (S3/Aliyun OSS/etc)
//
// Based on xiu's HLS implementation but with pluggable storage

pub mod file;
pub mod memory;
pub mod oss;

use async_trait::async_trait;
use bytes::Bytes;
use std::io::Result;

/// HLS storage trait for pluggable backends
///
/// Pure key-value storage interface. The storage layer should NOT know about:
/// - Segment metadata/lifecycle (handled by SegmentManager)
/// - M3U8 generation (handled by HLS layer)
///
/// Storage layer does: write, read, delete, exists, cleanup, and optionally provides public URLs.
#[async_trait]
pub trait HlsStorage: Send + Sync {
    /// Write data to storage
    ///
    /// # Arguments
    /// * `key` - Storage key (e.g., "live/room_123/segment_0.ts")
    /// * `data` - Binary data to store
    async fn write(&self, key: &str, data: Bytes) -> Result<()>;

    /// Read data from storage
    ///
    /// # Arguments
    /// * `key` - Storage key
    ///
    /// # Returns
    /// Binary data or NotFound error
    async fn read(&self, key: &str) -> Result<Bytes>;

    /// Delete single key from storage
    ///
    /// # Arguments
    /// * `key` - Storage key
    async fn delete(&self, key: &str) -> Result<()>;

    /// Check if key exists
    ///
    /// # Arguments
    /// * `key` - Storage key
    async fn exists(&self, key: &str) -> Result<bool>;

    /// Cleanup expired data
    ///
    /// Storage backend scans and deletes all data older than the specified duration.
    /// Upper layer (SegmentManager) calls this periodically to cleanup old segments.
    ///
    /// # Use Cases
    /// - Normal cleanup: Delete segments older than retention period
    /// - Leak prevention: Cleanup orphaned data after crash/restart
    /// - Storage management: Keep disk/OSS usage under control
    ///
    /// # Arguments
    /// * `older_than` - Delete data older than this duration
    ///
    /// # Returns
    /// Number of keys successfully deleted
    ///
    /// # Implementation Notes
    /// - **FileStorage**: Scan directory, check file mtime, delete old files
    /// - **MemoryStorage**: Iterate DashMap, check write timestamp, delete old entries
    /// - **OssStorage**: List objects, check LastModified, delete old objects
    ///
    /// Note: With hash-based keys, prefix filtering is not possible.
    /// All expired data will be cleaned up regardless of original key prefix.
    /// If you need per-room cleanup, use separate storage instances per room.
    ///
    /// # Default Implementation
    /// No-op by default (returns 0). Storage backends should implement this if possible.
    async fn cleanup(&self, _older_than: std::time::Duration) -> Result<usize> {
        Ok(0)
    }

    /// Get public URL for direct access (async)
    ///
    /// Use cases:
    /// - **OSS Storage with CDN**: Return CDN URL (e.g., "https://cdn.example.com/hls/segment.ts")
    /// - **OSS Storage without CDN**: Generate temporary presigned URL with expiration
    /// - **File/Memory Storage**: Return None, let HTTP layer generate local URLs
    ///
    /// # Arguments
    /// * `key` - Storage key
    ///
    /// # Returns
    /// - `Ok(Some(url))` - Public URL (CDN or presigned) for direct access
    /// - `Ok(None)` - No public URL available (File/Memory storage)
    /// - `Err(e)` - Failed to generate presigned URL
    ///
    /// # Default Implementation
    /// Returns None by default (File/Memory storage don't need public URLs)
    async fn get_public_url(&self, _key: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

/// Storage backend type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageBackend {
    /// Local filesystem storage
    File,
    /// In-memory storage (for testing/caching)
    Memory,
    /// Object storage (S3/OSS/etc)
    Oss,
}

pub use file::FileStorage;
pub use memory::MemoryStorage;
pub use oss::{OssStorage, OssConfig};

/// Example: HLS Segment Management Pattern
///
/// ```rust,ignore
/// use synctv_stream::storage::HlsStorage;
/// use std::time::Duration;
/// use std::sync::Arc;
///
/// struct SegmentInfo {
///     key: String,        // Flat key format: "app-stream-tsname"
///     duration: f64,
/// }
///
/// // SegmentManager - manages segment lifecycle
/// struct SegmentManager {
///     storage: Arc<dyn HlsStorage>,
///     retention: Duration, // e.g., 60 seconds
/// }
///
/// impl SegmentManager {
///     /// Periodic cleanup (called every 10 seconds)
///     async fn cleanup_expired_segments(&self) {
///         // Note: With hash-based storage, cleanup deletes ALL expired segments
///         // regardless of room/app. Cannot filter by prefix.
///         let deleted = self.storage
///             .cleanup(self.retention)
///             .await
///             .unwrap_or(0);
///
///         if deleted > 0 {
///             log::info!("Cleaned up {} expired segments", deleted);
///         }
///     }
///
///     /// Generate M3U8 playlist from current segments
///     async fn generate_m3u8(
///         &self,
///         segments: &[SegmentInfo],
///         base_url: &str,
///     ) -> String {
///         let mut m3u8 = String::from("#EXTM3U\n#EXT-X-VERSION:3\n");
///
///         for seg in segments {
///             m3u8.push_str(&format!("#EXTINF:{:.3},\n", seg.duration));
///
///             // Check if storage provides public URL (OSS/CDN or presigned)
///             let url = self.storage.get_public_url(&seg.key)
///                 .await
///                 .ok()
///                 .flatten()
///                 .unwrap_or_else(|| format!("{}/{}.ts", base_url, seg.key));
///
///             m3u8.push_str(&format!("{}\n", url));
///         }
///
///         m3u8
///     }
/// }
///
/// // Example M3U8 output:
/// //
/// // OSS Storage with CDN (hashed keys):
/// // #EXTINF:10.0,
/// // https://cdn.example.com/hls/hls/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
/// //
/// // OSS Storage without CDN (presigned URLs):
/// // #EXTINF:10.0,
/// // https://bucket.s3.amazonaws.com/hls/e3b0c44...?X-Amz-Signature=...
/// //
/// // File Storage (Local URLs):
/// // #EXTINF:10.0,
/// // /hls/live-room_123-a1b2c3d4e5f6.ts
/// ```
#[cfg(doc)]
pub fn _example_usage() {}
