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
    pub fn new(redis: RedisConnectionManager) -> Self {
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
        let key = format!("stream:publisher:{}:{}", room_id, media_id);
        let info = PublisherInfo {
            node_id: node_id.to_string(),
            app_name: app_name.to_string(),
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
        }

        Ok(registered)
    }

    /// Try to register as publisher (simplified version for PublisherManager)
    /// Returns true if registered successfully, false if already exists
    pub async fn try_register_publisher(
        &self,
        room_id: &str,
        media_id: &str,
        node_id: &str,
    ) -> anyhow::Result<bool> {
        let key = format!("stream:publisher:{}:{}", room_id, media_id);
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
    pub async fn refresh_publisher_ttl(&self, room_id: &str, media_id: &str) -> Result<()> {
        let key = format!("stream:publisher:{}:{}", room_id, media_id);
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
    pub async fn unregister_publisher(&mut self, room_id: &str, media_id: &str) -> Result<()> {
        let key = format!("stream:publisher:{}:{}", room_id, media_id);
        let _: () = self.redis.hdel(&key, "publisher").await?;
        Ok(())
    }

    /// Unregister a publisher (non-mut version for PublisherManager)
    pub async fn unregister_publisher_immut(&self, room_id: &str, media_id: &str) -> Result<()> {
        let key = format!("stream:publisher:{}:{}", room_id, media_id);
        let mut conn = self.redis.clone();

        let _: () = redis::cmd("HDEL")
            .arg(&key)
            .arg("publisher_node")
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        Ok(())
    }

    /// Get publisher info for a media in a room
    pub async fn get_publisher(&mut self, room_id: &str, media_id: &str) -> Result<Option<PublisherInfo>> {
        let key = format!("stream:publisher:{}:{}", room_id, media_id);
        let info_json: Option<String> = self.redis.hget(&key, "publisher").await?;

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
        let key = format!("stream:publisher:{}:{}", room_id, media_id);
        let exists: bool = self.redis.hexists(&key, "publisher").await?;
        Ok(exists)
    }

    /// List all active streams (returns tuples of (room_id, media_id))
    pub async fn list_active_streams(&mut self) -> Result<Vec<(String, String)>> {
        let keys: Vec<String> = self.redis.keys("stream:publisher:*").await?;
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
    #[ignore] // Requires Redis instance
    async fn test_register_publisher() {
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
    #[ignore] // Requires Redis instance
    async fn test_viewer_count() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let mut registry = StreamRegistry::new(redis);

        registry
            .register_publisher("room789", "media101", "node1", "live")
            .await
            .unwrap();

        // TODO: Add viewer tracking tests when implemented
        // let count = registry.increment_viewers("room789", "media101").await.unwrap();
        // assert_eq!(count, 1);

        // Cleanup
        registry.unregister_publisher("room789", "media101").await.unwrap();
    }
}
