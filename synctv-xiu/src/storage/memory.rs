// In-memory storage backend for HLS
//
// Useful for:
// - Testing without filesystem I/O
// - Temporary caching before OSS upload
// - Short-lived streams that don't need persistence
//
// Note: Data is lost on server restart
//
// Memory Safety:
// - Configurable max memory and max key limits prevent OOM
// - When limits are reached, oldest entries are evicted (LRU-like by write time)

use super::HlsStorage;
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use std::io::{Result, Error, ErrorKind};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing as log;

/// Default max memory: 512 MB
const DEFAULT_MAX_MEMORY_BYTES: usize = 512 * 1024 * 1024;
/// Default max keys: 10,000
const DEFAULT_MAX_KEYS: usize = 10_000;

/// In-memory storage backend with configurable memory limits
#[derive(Clone)]
pub struct MemoryStorage {
    /// Store data in memory with concurrent access
    /// Key: storage key, Value: (data, `write_time`)
    data: Arc<DashMap<String, (Bytes, Instant)>>,
    /// Maximum memory usage in bytes (0 = unlimited)
    max_memory_bytes: usize,
    /// Maximum number of keys (0 = unlimited)
    max_keys: usize,
}

impl MemoryStorage {
    /// Create new memory storage with default limits (512 MB, 10,000 keys)
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Arc::new(DashMap::new()),
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            max_keys: DEFAULT_MAX_KEYS,
        }
    }

    /// Create new memory storage with custom limits
    ///
    /// # Arguments
    /// * `max_memory_bytes` - Maximum memory in bytes (0 = unlimited)
    /// * `max_keys` - Maximum number of keys (0 = unlimited)
    #[must_use]
    pub fn with_limits(max_memory_bytes: usize, max_keys: usize) -> Self {
        Self {
            data: Arc::new(DashMap::new()),
            max_memory_bytes,
            max_keys,
        }
    }

    /// Create new memory storage with no limits (use with caution)
    #[must_use]
    pub fn unlimited() -> Self {
        Self {
            data: Arc::new(DashMap::new()),
            max_memory_bytes: 0,
            max_keys: 0,
        }
    }

    /// Get current memory usage in bytes
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        self.data.iter().map(|entry| entry.value().0.len()).sum()
    }

    /// Get number of stored keys
    #[must_use]
    pub fn key_count(&self) -> usize {
        self.data.len()
    }

    /// Clear all data (for testing/cleanup)
    pub fn clear(&self) {
        self.data.clear();
        log::info!("Cleared memory storage");
    }

    /// Evict oldest entries until we're under the memory and key limits.
    /// Returns the number of entries evicted.
    fn evict_if_needed(&self, incoming_bytes: usize) -> usize {
        let mut evicted = 0;

        // Check key limit
        if self.max_keys > 0 {
            while self.data.len() >= self.max_keys {
                if self.evict_oldest() {
                    evicted += 1;
                } else {
                    break;
                }
            }
        }

        // Check memory limit
        if self.max_memory_bytes > 0 {
            while self.memory_usage() + incoming_bytes > self.max_memory_bytes {
                if self.evict_oldest() {
                    evicted += 1;
                } else {
                    break;
                }
            }
        }

        if evicted > 0 {
            log::debug!(
                evicted = evicted,
                keys = self.data.len(),
                memory_bytes = self.memory_usage(),
                "Evicted old entries from memory storage"
            );
        }

        evicted
    }

    /// Evict the oldest entry by write time. Returns true if an entry was evicted.
    fn evict_oldest(&self) -> bool {
        let oldest_key = self
            .data
            .iter()
            .min_by_key(|entry| entry.value().1)
            .map(|entry| entry.key().clone());

        if let Some(key) = oldest_key {
            self.data.remove(&key);
            true
        } else {
            false
        }
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HlsStorage for MemoryStorage {
    async fn write(&self, key: &str, data: Bytes) -> Result<()> {
        let size = data.len();

        // Check if single entry exceeds the memory limit
        if self.max_memory_bytes > 0 && size > self.max_memory_bytes {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "Data size ({size} bytes) exceeds max memory limit ({} bytes)",
                    self.max_memory_bytes
                ),
            ));
        }

        // Evict old entries if needed to make room
        self.evict_if_needed(size);

        let write_time = Instant::now();
        self.data.insert(key.to_string(), (data, write_time));

        log::trace!("Wrote to memory: {} ({} bytes)", key, size);

        Ok(())
    }

    async fn read(&self, key: &str) -> Result<Bytes> {
        if let Some(entry) = self.data.get(key) {
            let (data, _) = entry.value();
            log::trace!("Read from memory: {} ({} bytes)", key, data.len());
            Ok(data.clone())
        } else {
            log::warn!("Key not found in memory: {}", key);
            Err(Error::new(
                ErrorKind::NotFound,
                format!("Key not found: {key}"),
            ))
        }
    }

    async fn delete(&self, key: &str) -> Result<()> {
        if self.data.remove(key).is_some() {
            log::trace!("Deleted from memory: {}", key);
        }

        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        Ok(self.data.contains_key(key))
    }

    async fn cleanup(&self, older_than: Duration) -> Result<usize> {
        let cutoff_time = Instant::now()
            .checked_sub(older_than)
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "older_than duration is too large"))?;
        let mut deleted = 0;

        // Collect expired keys
        let expired_keys: Vec<String> = self
            .data
            .iter()
            .filter(|entry| {
                let (_, write_time) = entry.value();
                *write_time < cutoff_time
            })
            .map(|entry| entry.key().clone())
            .collect();

        // Delete expired keys
        for key in expired_keys {
            if self.data.remove(&key).is_some() {
                deleted += 1;
                log::trace!("Deleted expired key from memory: {}", key);
            }
        }

        log::info!(
            "Cleanup expired: deleted {} keys older than {:?}",
            deleted,
            older_than
        );

        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_storage_write_read() {
        let storage = MemoryStorage::new();

        // Write (flat key format: app-stream-ts)
        let data = Bytes::from_static(b"test segment data");
        let result = storage
            .write("live-room_123-segment_0", data.clone())
            .await;
        assert!(result.is_ok());

        // Read
        let read_data = storage
            .read("live-room_123-segment_0")
            .await
            .unwrap();
        assert_eq!(data, read_data);

        // Check exists
        let exists = storage
            .exists("live-room_123-segment_0")
            .await
            .unwrap();
        assert!(exists);

        // Check memory usage
        assert_eq!(storage.memory_usage(), data.len());
        assert_eq!(storage.key_count(), 1);

        // Delete
        let result = storage.delete("live-room_123-segment_0").await;
        assert!(result.is_ok());

        // Check not exists
        let exists = storage
            .exists("live-room_123-segment_0")
            .await
            .unwrap();
        assert!(!exists);

        assert_eq!(storage.memory_usage(), 0);
        assert_eq!(storage.key_count(), 0);
    }


    #[tokio::test]
    async fn test_memory_storage_clear() {
        let storage = MemoryStorage::new();

        // Write some data
        storage
            .write("live-room_123-segment_0", Bytes::from_static(b"data1"))
            .await
            .unwrap();
        storage
            .write("live-room_456-segment_0", Bytes::from_static(b"data2"))
            .await
            .unwrap();

        assert_eq!(storage.key_count(), 2);

        // Clear
        storage.clear();

        assert_eq!(storage.key_count(), 0);
        assert_eq!(storage.memory_usage(), 0);
    }

    #[tokio::test]
    async fn test_memory_storage_not_found() {
        let storage = MemoryStorage::new();

        // Try to read non-existent key
        let result = storage.read("live-room_123-segment_0").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn test_memory_storage_public_url() {
        let storage = MemoryStorage::new();

        // Memory storage should return None (no public URL)
        let url = storage.get_public_url("live-room_123-segment_0").await.unwrap();
        assert_eq!(url, None);
    }

    #[tokio::test]
    async fn test_memory_storage_key_limit_eviction() {
        // Allow only 3 keys max
        let storage = MemoryStorage::with_limits(0, 3);

        storage.write("key1", Bytes::from_static(b"data1")).await.unwrap();
        storage.write("key2", Bytes::from_static(b"data2")).await.unwrap();
        storage.write("key3", Bytes::from_static(b"data3")).await.unwrap();
        assert_eq!(storage.key_count(), 3);

        // Writing a 4th key should evict the oldest (key1)
        storage.write("key4", Bytes::from_static(b"data4")).await.unwrap();
        assert_eq!(storage.key_count(), 3);
        assert!(!storage.exists("key1").await.unwrap());
        assert!(storage.exists("key4").await.unwrap());
    }

    #[tokio::test]
    async fn test_memory_storage_memory_limit_eviction() {
        // Allow only 15 bytes max
        let storage = MemoryStorage::with_limits(15, 0);

        storage.write("key1", Bytes::from_static(b"12345")).await.unwrap(); // 5 bytes
        storage.write("key2", Bytes::from_static(b"12345")).await.unwrap(); // 5 bytes, total 10
        assert_eq!(storage.key_count(), 2);
        assert_eq!(storage.memory_usage(), 10);

        // Writing 10 more bytes would exceed 15 byte limit, oldest (key1) should be evicted
        storage.write("key3", Bytes::from_static(b"1234567890")).await.unwrap(); // 10 bytes
        assert!(storage.memory_usage() <= 15);
        assert!(!storage.exists("key1").await.unwrap());
        assert!(storage.exists("key3").await.unwrap());
    }

    #[tokio::test]
    async fn test_memory_storage_reject_oversized() {
        // Allow only 10 bytes max
        let storage = MemoryStorage::with_limits(10, 0);

        // Data larger than max_memory_bytes should be rejected
        let result = storage.write("big", Bytes::from(vec![0u8; 20])).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn test_memory_storage_unlimited() {
        let storage = MemoryStorage::unlimited();

        // Should accept any amount of data
        for i in 0..100 {
            storage
                .write(&format!("key{i}"), Bytes::from(vec![0u8; 1024]))
                .await
                .unwrap();
        }
        assert_eq!(storage.key_count(), 100);
    }

    #[tokio::test]
    async fn test_memory_storage_cleanup() {
        let storage = MemoryStorage::new();

        // Write some keys (flat format)
        storage
            .write("live-room_123-segment_0", Bytes::from_static(b"data0"))
            .await
            .unwrap();
        storage
            .write("live-room_123-segment_1", Bytes::from_static(b"data1"))
            .await
            .unwrap();
        storage
            .write("live-room_456-segment_0", Bytes::from_static(b"data2"))
            .await
            .unwrap();

        // Sleep a bit
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Write another key (should not be deleted)
        storage
            .write("live-room_123-segment_2", Bytes::from_static(b"data3"))
            .await
            .unwrap();

        assert_eq!(storage.key_count(), 4);

        // Cleanup keys older than 50ms
        let deleted = storage
            .cleanup(Duration::from_millis(50))
            .await
            .unwrap();

        // Should delete segment_0, segment_1, and room_456 segment (all are old)
        // Note: cleanup now deletes ALL expired keys, not just specific prefix
        assert_eq!(deleted, 3);
        assert!(!storage.exists("live-room_123-segment_0").await.unwrap());
        assert!(!storage.exists("live-room_123-segment_1").await.unwrap());
        assert!(storage.exists("live-room_123-segment_2").await.unwrap());

        // room_456 segment is also deleted since it's old
        assert!(!storage.exists("live-room_456-segment_0").await.unwrap());

        assert_eq!(storage.key_count(), 1);
    }
}
