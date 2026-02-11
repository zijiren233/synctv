// In-memory storage backend for HLS
//
// Useful for:
// - Testing without filesystem I/O
// - Temporary caching before OSS upload
// - Short-lived streams that don't need persistence
//
// Note: Data is lost on server restart

use super::HlsStorage;
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use std::io::{Result, Error, ErrorKind};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing as log;

/// In-memory storage backend
#[derive(Clone)]
pub struct MemoryStorage {
    /// Store data in memory with concurrent access
    /// Key: storage key, Value: (data, `write_time`)
    data: Arc<DashMap<String, (Bytes, Instant)>>,
}

impl MemoryStorage {
    /// Create new memory storage
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Arc::new(DashMap::new()),
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
        let cutoff_time = Instant::now().checked_sub(older_than).unwrap();
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
