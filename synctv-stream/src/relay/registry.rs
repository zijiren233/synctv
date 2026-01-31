use anyhow::{Result, anyhow};
use redis::aio::ConnectionManager as RedisConnectionManager;
use redis::{AsyncCommands, Commands};
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Publisher information stored in Redis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublisherInfo {
    /// Node ID of the publisher
    pub node_id: String,
    /// RTMP app name
    pub app_name: String,
    /// When the stream started
    pub started_at: DateTime<Utc>,
    /// Number of active viewers
    #[serde(default)]
    pub viewer_count: u32,
}

/// Stream registry for tracking active publishers
#[derive(Clone)]
pub struct StreamRegistry {
    redis: RedisConnectionManager,
}

impl StreamRegistry {
    /// Create a new stream registry
    pub fn new(redis: RedisConnectionManager) -> Self {
        Self { redis }
    }

    /// Register a publisher for a room (atomic operation)
    /// Returns true if registered successfully, false if already exists
    pub async fn register_publisher(
        &mut self,
        room_id: &str,
        node_id: &str,
        app_name: &str,
    ) -> anyhow::Result<bool> {
        let key = format!("stream:{}", room_id);
        let info = PublisherInfo {
            node_id: node_id.to_string(),
            app_name: app_name.to_string(),
            started_at: Utc::now(),
            viewer_count: 0,
        };

        let info_json = serde_json::to_string(&info)?;

        // Use HSETNX for atomic set-if-not-exists
        let registered: bool = self.redis
            .hset_nx(&key, "publisher", info_json)
            .await?;

        if registered {
            // Set TTL of 300 seconds (5 minutes)
            let _: () = self.redis.expire(&key, 300).await?;
        }

        Ok(registered)
    }

    /// Try to register as publisher (simplified version for PublisherManager)
    /// Returns true if registered successfully, false if already exists
    pub async fn try_register_publisher(
        &self,
        room_id: &str,
        node_id: &str,
    ) -> anyhow::Result<bool> {
        let key = format!("stream:{}", room_id);
        let mut conn = self.redis.clone();

        // Use HSETNX for atomic set-if-not-exists
        let registered: bool = redis::cmd("HSETNX")
            .arg(&key)
            .arg("publisher_node")
            .arg(node_id)
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
        }

        Ok(registered)
    }

    /// Refresh TTL for a publisher (called by heartbeat)
    pub async fn refresh_publisher_ttl(&self, room_id: &str) -> Result<()> {
        let key = format!("stream:{}", room_id);
        let mut conn = self.redis.clone();

        // Refresh TTL to 300 seconds
        let _: () = redis::cmd("EXPIRE")
            .arg(&key)
            .arg(300)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        Ok(())
    }

    /// Unregister a publisher
    pub async fn unregister_publisher(&mut self, room_id: &str) -> Result<()> {
        let key = format!("stream:{}", room_id);
        let _: () = self.redis.hdel(&key, "publisher").await?;
        Ok(())
    }

    /// Unregister a publisher (non-mut version for PublisherManager)
    pub async fn unregister_publisher_immut(&self, room_id: &str) -> Result<()> {
        let key = format!("stream:{}", room_id);
        let mut conn = self.redis.clone();

        let _: () = redis::cmd("HDEL")
            .arg(&key)
            .arg("publisher_node")
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        Ok(())
    }

    /// Get publisher info for a room
    pub async fn get_publisher(&mut self, room_id: &str) -> Result<Option<PublisherInfo>> {
        let key = format!("stream:{}", room_id);
        let info_json: Option<String> = self.redis.hget(&key, "publisher").await?;

        match info_json {
            Some(json) => {
                let info: PublisherInfo = serde_json::from_str(&json)?;
                Ok(Some(info))
            }
            None => Ok(None),
        }
    }

    /// Increment viewer count for a stream
    pub async fn increment_viewers(&mut self, room_id: &str) -> Result<u32> {
        let key = format!("stream:{}", room_id);
        let count: u32 = self.redis.hincr(&key, "viewers", 1).await?;
        Ok(count)
    }

    /// Decrement viewer count for a stream
    pub async fn decrement_viewers(&mut self, room_id: &str) -> Result<u32> {
        let key = format!("stream:{}", room_id);
        let count: i32 = self.redis.hincr(&key, "viewers", -1).await?;
        Ok(count.max(0) as u32)
    }

    /// Get viewer count for a stream
    pub async fn get_viewer_count(&mut self, room_id: &str) -> Result<u32> {
        let key = format!("stream:{}", room_id);
        let count: Option<u32> = self.redis.hget(&key, "viewers").await?;
        Ok(count.unwrap_or(0))
    }

    /// Check if a stream is active (has a publisher)
    pub async fn is_stream_active(&mut self, room_id: &str) -> anyhow::Result<bool> {
        let key = format!("stream:{}", room_id);
        let exists: bool = self.redis.hexists(&key, "publisher").await?;
        Ok(exists)
    }

    /// List all active streams
    pub async fn list_active_streams(&mut self) -> Result<Vec<String>> {
        let keys: Vec<String> = self.redis.keys("stream:*").await?;
        let room_ids: Vec<String> = keys
            .into_iter()
            .filter_map(|k| k.strip_prefix("stream:").map(|s| s.to_string()))
            .collect();
        Ok(room_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires Redis instance
    async fn test_register_publisher() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let mut registry = StreamRegistry::new(redis);

        // First registration should succeed
        let registered = registry
            .register_publisher("room123", "node1", "live")
            .await
            .unwrap();
        assert!(registered);

        // Second registration should fail (already exists)
        let registered = registry
            .register_publisher("room123", "node2", "live")
            .await
            .unwrap();
        assert!(!registered);

        // Cleanup
        registry.unregister_publisher("room123").await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Redis instance
    async fn test_viewer_count() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let mut registry = StreamRegistry::new(redis);

        registry
            .register_publisher("room456", "node1", "live")
            .await
            .unwrap();

        // Increment viewers
        let count = registry.increment_viewers("room456").await.unwrap();
        assert_eq!(count, 1);

        let count = registry.increment_viewers("room456").await.unwrap();
        assert_eq!(count, 2);

        // Decrement viewers
        let count = registry.decrement_viewers("room456").await.unwrap();
        assert_eq!(count, 1);

        // Cleanup
        registry.unregister_publisher("room456").await.unwrap();
    }
}
