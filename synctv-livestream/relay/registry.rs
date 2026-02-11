use anyhow::{Result, anyhow};
use redis::aio::ConnectionManager as RedisConnectionManager;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Publisher information stored in Redis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublisherInfo {
    /// Node ID of the publisher
    pub node_id: String,
    /// RTMP app name
    pub app_name: String,
    /// User ID of the publisher (for reverse-index lookups)
    #[serde(default)]
    pub user_id: String,
    /// When the stream started
    pub started_at: DateTime<Utc>,
}

/// Stream registry for tracking active publishers
#[derive(Clone)]
pub struct StreamRegistry {
    redis: RedisConnectionManager,
}

impl StreamRegistry {
    /// Create a new stream registry
    #[must_use] 
    pub const fn new(redis: RedisConnectionManager) -> Self {
        Self { redis }
    }

    /// Register a publisher for a media in a room (atomic operation)
    /// Returns true if registered successfully, false if already exists
    pub async fn register_publisher(
        &mut self,
        room_id: &str,
        media_id: &str,
        node_id: &str,
        app_name: &str,
    ) -> anyhow::Result<bool> {
        self.register_publisher_with_user(room_id, media_id, node_id, app_name, "").await
    }

    /// Register a publisher with `user_id` for a media in a room (atomic operation)
    /// Returns true if registered successfully, false if already exists
    pub async fn register_publisher_with_user(
        &mut self,
        room_id: &str,
        media_id: &str,
        node_id: &str,
        app_name: &str,
        user_id: &str,
    ) -> anyhow::Result<bool> {
        let key = format!("stream:publisher:{room_id}:{media_id}");
        let info = PublisherInfo {
            node_id: node_id.to_string(),
            app_name: app_name.to_string(),
            user_id: user_id.to_string(),
            started_at: Utc::now(),
        };

        let info_json = serde_json::to_string(&info)?;

        // Use HSETNX for atomic set-if-not-exists
        let registered: bool = self.redis
            .hset_nx(&key, "publisher", info_json)
            .await?;

        if registered {
            // Set TTL of 300 seconds (5 minutes)
            let _: () = self.redis.expire(&key, 300).await?;

            // Add to user reverse index if user_id is provided
            if !user_id.is_empty() {
                let user_key = format!("stream:user_publishers:{user_id}");
                let member = format!("{room_id}:{media_id}");
                let _: () = self.redis.sadd(&user_key, &member).await?;
                let _: () = self.redis.expire(&user_key, 300).await?;
            }
        }

        Ok(registered)
    }

    /// Try to register as publisher (simplified version for `PublisherManager`)
    /// Returns true if registered successfully, false if already exists
    pub async fn try_register_publisher(
        &self,
        room_id: &str,
        media_id: &str,
        node_id: &str,
    ) -> anyhow::Result<bool> {
        self.try_register_publisher_with_user(room_id, media_id, node_id, "").await
    }

    /// Try to register as publisher with `user_id`
    /// Returns true if registered successfully, false if already exists
    pub async fn try_register_publisher_with_user(
        &self,
        room_id: &str,
        media_id: &str,
        node_id: &str,
        user_id: &str,
    ) -> anyhow::Result<bool> {
        let key = format!("stream:publisher:{room_id}:{media_id}");
        let mut conn = self.redis.clone();

        // Create PublisherInfo with default values for registration
        let info = PublisherInfo {
            node_id: node_id.to_string(),
            app_name: "live".to_string(),
            user_id: user_id.to_string(),
            started_at: Utc::now(),
        };
        let info_json = serde_json::to_string(&info)?;

        // Use HSETNX for atomic set-if-not-exists with "publisher" field
        let registered: bool = redis::cmd("HSETNX")
            .arg(&key)
            .arg("publisher")
            .arg(info_json)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        if registered {
            // Set TTL of 300 seconds (5 minutes)
            let _: () = redis::cmd("EXPIRE")
                .arg(&key)
                .arg(300)
                .query_async(&mut conn)
                .await
                .map_err(|e| anyhow!(e.to_string()))?;

            // Add to user reverse index if user_id is provided
            if !user_id.is_empty() {
                let user_key = format!("stream:user_publishers:{user_id}");
                let member = format!("{room_id}:{media_id}");
                let _: () = redis::cmd("SADD")
                    .arg(&user_key)
                    .arg(&member)
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| anyhow!(e.to_string()))?;
                let _: () = redis::cmd("EXPIRE")
                    .arg(&user_key)
                    .arg(300)
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| anyhow!(e.to_string()))?;
            }
        }

        Ok(registered)
    }

    /// Refresh TTL for a publisher (called by heartbeat)
    pub async fn refresh_publisher_ttl(&self, room_id: &str, media_id: &str) -> Result<()> {
        self.refresh_publisher_ttl_with_user(room_id, media_id, "").await
    }

    /// Refresh TTL for a publisher and its user reverse-index (called by heartbeat)
    pub async fn refresh_publisher_ttl_with_user(&self, room_id: &str, media_id: &str, user_id: &str) -> Result<()> {
        let key = format!("stream:publisher:{room_id}:{media_id}");
        let mut conn = self.redis.clone();

        // Refresh publisher key TTL to 300 seconds
        let _: () = redis::cmd("EXPIRE")
            .arg(&key)
            .arg(300)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        // Also refresh user reverse-index TTL if user_id is provided
        if !user_id.is_empty() {
            let user_key = format!("stream:user_publishers:{user_id}");
            let _: () = redis::cmd("EXPIRE")
                .arg(&user_key)
                .arg(300)
                .query_async(&mut conn)
                .await
                .map_err(|e| anyhow!(e.to_string()))?;
        }

        Ok(())
    }

    /// Unregister a publisher
    pub async fn unregister_publisher(&mut self, room_id: &str, media_id: &str) -> Result<()> {
        let key = format!("stream:publisher:{room_id}:{media_id}");
        let _: () = self.redis.hdel(&key, "publisher").await?;
        Ok(())
    }

    /// Unregister a publisher (non-mut version for `PublisherManager`)
    pub async fn unregister_publisher_immut(&self, room_id: &str, media_id: &str) -> Result<()> {
        let key = format!("stream:publisher:{room_id}:{media_id}");
        let mut conn = self.redis.clone();

        // Get publisher info first to clean up reverse index
        let info_json: Option<String> = redis::cmd("HGET")
            .arg(&key)
            .arg("publisher")
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        if let Some(json) = info_json {
            if let Ok(info) = serde_json::from_str::<PublisherInfo>(&json) {
                if !info.user_id.is_empty() {
                    let user_key = format!("stream:user_publishers:{}", info.user_id);
                    let member = format!("{room_id}:{media_id}");
                    let _: () = redis::cmd("SREM")
                        .arg(&user_key)
                        .arg(&member)
                        .query_async(&mut conn)
                        .await
                        .map_err(|e| anyhow!(e.to_string()))?;
                }
            }
        }

        let _: () = redis::cmd("HDEL")
            .arg(&key)
            .arg("publisher")
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        Ok(())
    }

    /// Get all active publishers for a user (via reverse index)
    /// Returns list of (`room_id`, `media_id`) pairs
    pub async fn get_user_publishers(&self, user_id: &str) -> Result<Vec<(String, String)>> {
        let user_key = format!("stream:user_publishers:{user_id}");
        let mut conn = self.redis.clone();

        let members: Vec<String> = redis::cmd("SMEMBERS")
            .arg(&user_key)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        Ok(members
            .into_iter()
            .filter_map(|m| {
                m.split_once(':')
                    .map(|(r, m)| (r.to_string(), m.to_string()))
            })
            .collect())
    }

    /// Remove all publisher entries for a user (via reverse index)
    pub async fn unregister_all_user_publishers(&self, user_id: &str) -> Result<()> {
        let publishers = self.get_user_publishers(user_id).await?;
        for (room_id, media_id) in publishers {
            self.unregister_publisher_immut(&room_id, &media_id).await?;
        }
        Ok(())
    }

    /// Get publisher info for a media in a room
    pub async fn get_publisher(&mut self, room_id: &str, media_id: &str) -> Result<Option<PublisherInfo>> {
        self.get_publisher_immut(room_id, media_id).await
    }

    /// Get publisher info for a media in a room (immutable version)
    pub async fn get_publisher_immut(&self, room_id: &str, media_id: &str) -> Result<Option<PublisherInfo>> {
        let key = format!("stream:publisher:{room_id}:{media_id}");
        let mut conn = self.redis.clone();
        let info_json: Option<String> = redis::cmd("HGET")
            .arg(&key)
            .arg("publisher")
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        match info_json {
            Some(json) => {
                let info: PublisherInfo = serde_json::from_str(&json)?;
                Ok(Some(info))
            }
            None => Ok(None),
        }
    }


    /// Check if a stream is active (has a publisher)
    pub async fn is_stream_active(&mut self, room_id: &str, media_id: &str) -> anyhow::Result<bool> {
        self.is_stream_active_immut(room_id, media_id).await
    }

    /// Check if a stream is active (immutable version)
    pub async fn is_stream_active_immut(&self, room_id: &str, media_id: &str) -> anyhow::Result<bool> {
        let key = format!("stream:publisher:{room_id}:{media_id}");
        let mut conn = self.redis.clone();
        let exists: bool = redis::cmd("HEXISTS")
            .arg(&key)
            .arg("publisher")
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;
        Ok(exists)
    }

    /// List all active streams (returns tuples of (`room_id`, `media_id`))
    pub async fn list_active_streams(&mut self) -> Result<Vec<(String, String)>> {
        self.list_active_streams_immut().await
    }

    /// List all active streams (immutable version)
    pub async fn list_active_streams_immut(&self) -> Result<Vec<(String, String)>> {
        let mut conn = self.redis.clone();
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg("stream:publisher:*")
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        let streams: Vec<(String, String)> = keys
            .into_iter()
            .filter_map(|k| {
                k.strip_prefix("stream:publisher:")
                    .and_then(|s| {
                        let parts: Vec<&str> = s.split(':').collect();
                        if parts.len() == 2 {
                            Some((parts[0].to_string(), parts[1].to_string()))
                        } else {
                            None
                        }
                    })
            })
            .collect();
        Ok(streams)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires Redis server"]
    async fn test_register_publisher_success() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let mut registry = StreamRegistry::new(redis);

        // First registration should succeed
        let registered = registry
            .register_publisher("room123", "media456", "node1", "live")
            .await
            .unwrap();
        assert!(registered);

        // Verify publisher exists
        let publisher = registry.get_publisher("room123", "media456").await.unwrap();
        assert!(publisher.is_some());

        let pub_info = publisher.unwrap();
        assert_eq!(pub_info.node_id, "node1");
        assert_eq!(pub_info.app_name, "live");

        // Cleanup
        registry.unregister_publisher("room123", "media456").await.unwrap();
    }

    #[tokio::test]
    #[ignore = "Requires Redis server"]
    async fn test_register_publisher_duplicate() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let mut registry = StreamRegistry::new(redis);

        // First registration should succeed
        let registered = registry
            .register_publisher("room123", "media456", "node1", "live")
            .await
            .unwrap();
        assert!(registered);

        // Second registration should fail (already exists)
        let registered = registry
            .register_publisher("room123", "media456", "node2", "live")
            .await
            .unwrap();
        assert!(!registered);

        // Cleanup
        registry.unregister_publisher("room123", "media456").await.unwrap();
    }

    #[tokio::test]
    #[ignore = "Requires Redis server"]
    async fn test_try_register_publisher() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let mut registry = StreamRegistry::new(redis);

        // First try_register should succeed
        let result = registry.try_register_publisher("room123", "media456", "node1").await.unwrap();
        assert!(result);

        // Second try_register should return false (already exists)
        let result = registry.try_register_publisher("room123", "media456", "node2").await.unwrap();
        assert!(!result);

        // Cleanup
        registry.unregister_publisher("room123", "media456").await.unwrap();
    }

    #[tokio::test]
    #[ignore = "Requires Redis server"]
    async fn test_unregister_publisher() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let mut registry = StreamRegistry::new(redis);

        // Register publisher
        registry
            .register_publisher("room123", "media456", "node1", "live")
            .await
            .unwrap();

        // Verify exists
        assert!(registry.is_stream_active("room123", "media456").await.unwrap());

        // Unregister
        registry.unregister_publisher("room123", "media456").await.unwrap();

        // Verify removed
        assert!(!registry.is_stream_active("room123", "media456").await.unwrap());
    }

    #[tokio::test]
    #[ignore = "Requires Redis server"]
    async fn test_get_publisher_not_found() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let mut registry = StreamRegistry::new(redis);

        // Non-existent publisher should return None
        let result = registry.get_publisher("nonexistent", "media").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    #[ignore = "Requires Redis server"]
    async fn test_list_active_streams() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let mut registry = StreamRegistry::new(redis);

        // Register multiple publishers
        registry
            .register_publisher("room1", "media1", "node1", "live")
            .await
            .unwrap();
        registry
            .register_publisher("room2", "media2", "node1", "live")
            .await
            .unwrap();

        // List active streams
        let streams = registry.list_active_streams().await.unwrap();
        assert_eq!(streams.len(), 2);
        assert!(streams.contains(&(String::from("room1"), String::from("media1"))));
        assert!(streams.contains(&(String::from("room2"), String::from("media2"))));

        // Cleanup
        registry.unregister_publisher("room1", "media1").await.unwrap();
        registry.unregister_publisher("room2", "media2").await.unwrap();
    }

    #[tokio::test]
    #[ignore = "Requires Redis server"]
    async fn test_publisher_info_serialization() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let mut registry = StreamRegistry::new(redis);

        // Register publisher
        registry
            .register_publisher("room123", "media456", "node1", "live")
            .await
            .unwrap();

        // Get publisher and verify serialization/deserialization
        let publisher = registry.get_publisher("room123", "media456").await.unwrap().unwrap();

        assert_eq!(publisher.node_id, "node1");
        assert_eq!(publisher.app_name, "live");
        assert!(publisher.started_at <= chrono::Utc::now());

        // Cleanup
        registry.unregister_publisher("room123", "media456").await.unwrap();
    }
}
