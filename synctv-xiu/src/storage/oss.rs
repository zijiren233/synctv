// Object Storage Service (OSS) backend for HLS
//
// Supports:
// - AWS S3
// - Aliyun OSS
// - Minio
// - Any S3-compatible storage
//
// Uses OpenDAL for unified storage access

#[cfg(feature = "oss")]
mod inner {
    use crate::storage::HlsStorage;
    use async_trait::async_trait;
    use bytes::Bytes;
    use chrono::{DateTime, Utc, Duration as ChronoDuration};
    use opendal::{Operator, services::S3};
    use sha2::{Sha256, Digest};
    use std::io::{Result, Error, ErrorKind};
    use std::sync::Arc;
    use std::time::Duration;

    /// Hash storage key to prevent path traversal attacks
    ///
    /// Uses SHA256 to convert arbitrary keys into safe object keys
    fn hash_key(key: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// OSS storage configuration
    #[derive(Debug, Clone)]
    pub struct OssConfig {
        /// OSS endpoint (e.g., "oss-cn-hangzhou.aliyuncs.com" or "s3.amazonaws.com")
        pub endpoint: String,
        /// Access key ID
        pub access_key_id: String,
        /// Secret access key
        pub secret_access_key: String,
        /// Bucket name
        pub bucket: String,
        /// Region (for S3)
        pub region: Option<String>,
        /// Base path prefix in bucket (e.g., "hls/")
        pub base_path: String,
        /// Public URL prefix for serving (e.g., "<https://cdn.example.com/hls>/")
        /// If empty, will generate presigned temporary URLs
        pub public_url_prefix: String,
        /// Presigned URL expiration time in seconds (default: 3600 = 1 hour)
        /// Only used when `public_url_prefix` is empty
        pub presign_expires_in: u64,
    }

    /// OSS storage backend
    pub struct OssStorage {
        config: OssConfig,
        operator: Arc<Operator>,
    }

    impl OssStorage {
        /// Create new OSS storage with configuration
        pub fn new(config: OssConfig) -> std::result::Result<Self, Box<dyn std::error::Error>> {
            tracing::info!(
                "Initializing OSS storage: bucket={}, endpoint={}",
                config.bucket,
                config.endpoint
            );

            // Configure S3 service
            let mut builder = S3::default()
                .endpoint(&config.endpoint)
                .access_key_id(&config.access_key_id)
                .secret_access_key(&config.secret_access_key)
                .bucket(&config.bucket);

            if let Some(region) = &config.region {
                builder = builder.region(region);
            }

            // Build operator
            let operator = Operator::new(builder)?.finish();

            Ok(Self {
                config,
                operator: Arc::new(operator),
            })
        }

        /// Get full object key with base path prefix and hashing
        fn get_object_key(&self, key: &str) -> String {
            let hashed = hash_key(key);
            if self.config.base_path.is_empty() {
                hashed
            } else {
                format!("{}{}", self.config.base_path, hashed)
            }
        }
    }

    #[async_trait]
    impl HlsStorage for OssStorage {
        async fn write(&self, key: &str, data: Bytes) -> Result<()> {
            let object_key = self.get_object_key(key);
            let size = data.len();

            // Write to OSS using OpenDAL
            self.operator
                .write(&object_key, data)
                .await
                .map_err(|e| Error::other(format!("OSS write failed: {e}")))?;

            tracing::trace!("Wrote to OSS: {} ({} bytes) for key: {}", object_key, size, key);

            Ok(())
        }

        async fn read(&self, key: &str) -> Result<Bytes> {
            let object_key = self.get_object_key(key);

            // Read from OSS using OpenDAL
            let buffer = self.operator
                .read(&object_key)
                .await
                .map_err(|e| Error::new(ErrorKind::NotFound, format!("OSS read failed: {e}")))?;

            // Convert OpenDAL Buffer to Bytes
            let data = Bytes::from(buffer.to_vec());

            tracing::trace!("Read from OSS: {} ({} bytes) for key: {}", object_key, data.len(), key);

            Ok(data)
        }

        async fn delete(&self, key: &str) -> Result<()> {
            let object_key = self.get_object_key(key);

            // Delete from OSS using OpenDAL
            self.operator
                .delete(&object_key)
                .await
                .map_err(|e| Error::other(format!("OSS delete failed: {e}")))?;

            tracing::trace!("Deleted from OSS: {} for key: {}", object_key, key);

            Ok(())
        }

        async fn exists(&self, key: &str) -> Result<bool> {
            let object_key = self.get_object_key(key);

            // Check if object exists using OpenDAL
            match self.operator.is_exist(&object_key).await {
                Ok(exists) => Ok(exists),
                Err(e) => {
                    tracing::warn!("OSS exists check failed for {}: {}", object_key, e);
                    Ok(false)
                }
            }
        }

        async fn cleanup(&self, older_than: Duration) -> Result<usize> {
            // Convert Duration to chrono Duration
            let chrono_duration = ChronoDuration::from_std(older_than)
                .map_err(|e| Error::new(ErrorKind::InvalidInput, format!("Invalid duration: {e}")))?;
            let cutoff_time: DateTime<Utc> = Utc::now() - chrono_duration;
            let mut deleted = 0;

            let base_path = if self.config.base_path.is_empty() {
                String::new()
            } else {
                self.config.base_path.clone()
            };

            // List all objects in base_path
            let lister = self.operator
                .lister(&base_path)
                .await
                .map_err(|e| Error::other(format!("OSS list failed: {e}")))?;

            // Iterate through objects and delete old ones
            use futures::TryStreamExt;
            let mut entries = lister;
            while let Some(entry) = entries.try_next().await
                .map_err(|e| Error::other(format!("OSS list iteration failed: {e}")))? {

                let path = entry.path();

                // Get object metadata to check last modified time
                match self.operator.stat(path).await {
                    Ok(metadata) => {
                        if let Some(last_modified) = metadata.last_modified() {
                            if last_modified < cutoff_time {
                                // Object is older than cutoff, delete it
                                if self.operator.delete(path).await.is_ok() {
                                    deleted += 1;
                                    tracing::trace!("Deleted expired OSS object: {}", path);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to stat OSS object {}: {}", path, e);
                    }
                }
            }

            tracing::info!(
                "OSS cleanup completed: bucket={}, deleted {} objects older than {:?}",
                self.config.bucket,
                deleted,
                older_than
            );

            Ok(deleted)
        }

        async fn get_public_url(&self, key: &str) -> Result<Option<String>> {
            // Get hashed object key (used for both CDN and presigned URLs)
            let object_key = self.get_object_key(key);

            // If CDN is configured, return CDN URL with hashed key
            if !self.config.public_url_prefix.is_empty() {
                let cdn_url = format!("{}{}", self.config.public_url_prefix, object_key);
                tracing::trace!("Generated CDN URL for key '{}': {}", key, cdn_url);
                return Ok(Some(cdn_url));
            }

            // No CDN, generate presigned URL with expiration
            // Convert u64 seconds to Duration
            let expires_in = Duration::from_secs(self.config.presign_expires_in);

            // Generate presigned read URL with expiration
            let presigned_req = self.operator
                .presign_read(&object_key, expires_in)
                .await
                .map_err(|e| Error::other(format!("Failed to presign URL: {e}")))?;

            // Get the presigned URL
            let url = presigned_req.uri().to_string();

            tracing::trace!(
                "Generated presigned URL for key '{}': expires in {}s",
                key,
                self.config.presign_expires_in
            );

            Ok(Some(url))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[tokio::test]
        async fn test_oss_storage_public_url_with_cdn() {
            let config = OssConfig {
                endpoint: "s3.amazonaws.com".to_string(),
                access_key_id: "test".to_string(),
                secret_access_key: "test".to_string(),
                bucket: "my-bucket".to_string(),
                region: Some("us-east-1".to_string()),
                base_path: "hls/".to_string(),
                public_url_prefix: "https://cdn.example.com/hls/".to_string(),
                presign_expires_in: 3600,
            };

            let storage = OssStorage::new(config).unwrap();

            // With CDN configured, should return CDN URL with hashed key
            let url = storage.get_public_url("live-room_123-segment_0").await.unwrap();
            assert!(url.is_some());
            let url_str = url.unwrap();
            assert!(url_str.starts_with("https://cdn.example.com/hls/hls/"));
            // URL should contain hash of the key, not the original key
            assert!(!url_str.contains("live-room_123-segment_0"));
        }

        #[tokio::test]
        async fn test_oss_storage_public_url_no_base_path() {
            let config = OssConfig {
                endpoint: "https://minio.example.com:9000".to_string(),
                access_key_id: "test".to_string(),
                secret_access_key: "test".to_string(),
                bucket: "hls".to_string(),
                region: Some("us-east-1".to_string()),
                base_path: "".to_string(),
                public_url_prefix: "https://minio.example.com:9000/hls/".to_string(),
                presign_expires_in: 3600,
            };

            let storage = OssStorage::new(config).unwrap();

            let url = storage.get_public_url("room_123-segment_0").await.unwrap();
            assert!(url.is_some());
            let url_str = url.unwrap();
            assert!(url_str.starts_with("https://minio.example.com:9000/hls/"));
            assert!(!url_str.contains("room_123-segment_0"));
        }
    }
}

#[cfg(feature = "oss")]
pub use inner::*;

// When oss feature is disabled, provide stub types so downstream code compiles
#[cfg(not(feature = "oss"))]
mod stub {
    /// OSS storage configuration (requires `oss` feature)
    #[derive(Debug, Clone)]
    pub struct OssConfig {
        pub endpoint: String,
        pub access_key_id: String,
        pub secret_access_key: String,
        pub bucket: String,
        pub region: Option<String>,
        pub base_path: String,
        pub public_url_prefix: String,
        pub presign_expires_in: u64,
    }

    /// OSS storage backend (requires `oss` feature)
    pub struct OssStorage;

    impl OssStorage {
        pub fn new(_config: OssConfig) -> std::result::Result<Self, Box<dyn std::error::Error>> {
            Err("OSS storage requires the `oss` feature to be enabled".into())
        }
    }
}

#[cfg(not(feature = "oss"))]
pub use stub::*;
