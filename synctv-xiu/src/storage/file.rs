// File system storage backend for HLS
//
// Default storage backend using local filesystem
// With hash-based path security
//
// Storage key format: "app-stream-ts" (flat, no directories)
// Keys are hashed with SHA256 before using as filenames

use super::HlsStorage;
use async_trait::async_trait;
use bytes::Bytes;
use sha2::{Sha256, Digest};
use std::io::Result;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tokio::fs;

/// Check if a filename matches the SHA256 hex hash pattern (exactly 64 hex chars).
/// This prevents accidentally deleting non-HLS files if `base_path` is misconfigured.
fn is_sha256_filename(name: &str) -> bool {
    name.len() == 64 && name.chars().all(|c| c.is_ascii_hexdigit())
}

/// Hash storage key to prevent path traversal attacks
///
/// Uses SHA256 to convert arbitrary keys into safe filenames
/// Example: "live-room123-a1b2c3d4e5f6" -> "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
fn hash_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// File system storage backend
pub struct FileStorage {
    base_path: PathBuf,
}

impl FileStorage {
    /// Create new file storage with base path
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into(),
        }
    }

    /// Get full file path from hashed key (keys are hashed for security)
    fn get_path(&self, key: &str) -> PathBuf {
        let hashed = hash_key(key);
        self.base_path.join(hashed)
    }
}

#[async_trait]
impl HlsStorage for FileStorage {
    async fn write(&self, key: &str, data: Bytes) -> Result<()> {
        let file_path = self.get_path(key);
        let size = data.len();
        fs::write(&file_path, data).await?;

        tracing::trace!("Wrote: {:?} ({} bytes) for key: {}", file_path, size, key);

        Ok(())
    }

    async fn read(&self, key: &str) -> Result<Bytes> {
        let file_path = self.get_path(key);
        let data = fs::read(&file_path).await?;

        tracing::trace!("Read: {:?} ({} bytes) for key: {}", file_path, data.len(), key);

        Ok(Bytes::from(data))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let file_path = self.get_path(key);

        // Use tokio async exists check
        if fs::try_exists(&file_path).await.unwrap_or(false) {
            fs::remove_file(&file_path).await?;
            tracing::trace!("Deleted: {:?} for key: {}", file_path, key);
        }

        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let file_path = self.get_path(key);
        fs::try_exists(&file_path).await
    }

    async fn cleanup(&self, older_than: Duration) -> Result<usize> {
        // Use tokio async exists check
        if !fs::try_exists(&self.base_path).await.unwrap_or(false) {
            tracing::debug!("Cleanup base path does not exist: {:?}", self.base_path);
            return Ok(0);
        }

        let cutoff_time = SystemTime::now() - older_than;
        let mut deleted = 0;
        let mut entries = fs::read_dir(&self.base_path).await?;

        // Scan all files in base_path (flat structure with hashed filenames)
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            // Only process files (skip directories) - use async metadata check
            let file_type = match entry.file_type().await {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if !file_type.is_file() {
                continue;
            }

            // Only delete files matching the SHA256 hex filename pattern.
            // This prevents deleting unrelated files if base_path is misconfigured.
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();
            if !is_sha256_filename(&file_name_str) {
                continue;
            }

            // Check file modified time
            if let Ok(metadata) = fs::metadata(&path).await {
                if let Ok(modified) = metadata.modified() {
                    if modified < cutoff_time {
                        // File is older than cutoff, delete it
                        if fs::remove_file(&path).await.is_ok() {
                            deleted += 1;
                            tracing::trace!("Deleted expired file: {:?}", path);
                        }
                    }
                }
            }
        }

        tracing::info!(
            "Cleanup completed: scanned {:?}, deleted {} files older than {:?}",
            self.base_path,
            deleted,
            older_than
        );

        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_file_storage_write_read() {
        let temp_dir = tempdir().unwrap();
        let storage = FileStorage::new(temp_dir.path());

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

        // Delete
        let result = storage.delete("live-room_123-segment_0").await;
        assert!(result.is_ok());

        // Check not exists
        let exists = storage
            .exists("live-room_123-segment_0")
            .await
            .unwrap();
        assert!(!exists);
    }


    #[tokio::test]
    async fn test_file_storage_public_url() {
        let temp_dir = tempdir().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        // File storage should return None (no public URL)
        let url = storage.get_public_url("live-room_123-segment_0").await.unwrap();
        assert_eq!(url, None);
    }

    #[tokio::test]
    async fn test_file_storage_cleanup() {
        let temp_dir = tempdir().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        // Write files (flat key format)
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
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Write another file (should not be deleted)
        storage
            .write("live-room_123-segment_2", Bytes::from_static(b"data3"))
            .await
            .unwrap();

        // Cleanup files older than 50ms
        let deleted = storage
            .cleanup(Duration::from_millis(50))
            .await
            .unwrap();

        // Should delete segment_0, segment_1, and segment from room_456 (all are old)
        // Note: cleanup now deletes ALL expired files, not just specific prefix
        assert_eq!(deleted, 3);
        assert!(!storage.exists("live-room_123-segment_0").await.unwrap());
        assert!(!storage.exists("live-room_123-segment_1").await.unwrap());
        assert!(storage.exists("live-room_123-segment_2").await.unwrap());

        // room_456 segment will also be deleted since it's old
        assert!(!storage.exists("live-room_456-segment_0").await.unwrap());
    }
}
