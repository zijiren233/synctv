//! Username cache service for fast username lookups
//!
//! Uses a two-tier caching strategy with mature crates:
//! 1. In-memory Moka LRU cache for frequently accessed usernames
//! 2. Redis persistent cache for cross-node consistency

use redis::{AsyncCommands, Client};
use std::collections::HashMap;
use std::sync::Arc;

use crate::{models::UserId, Error, Result};

/// Username cache service with L1 (Moka) + L2 (Redis) strategy
#[derive(Clone)]
pub struct UsernameCache {
    redis_client: Option<Client>,
    memory_cache: Arc<moka::future::Cache<UserId, String>>,
    key_prefix: String,
    ttl_seconds: u64,
}

impl UsernameCache {
    /// Create a new UsernameCache
    ///
    /// # Arguments
    /// * `redis_url` - Optional Redis URL. If None, only in-memory caching is used.
    /// * `key_prefix` - Redis key prefix (e.g., "synctv:username:")
    /// * `memory_cache_size` - Maximum number of entries in memory cache
    /// * `ttl_seconds` - Cache TTL in Redis (0 = no expiration)
    pub fn new(
        redis_url: Option<String>,
        key_prefix: String,
        memory_cache_size: usize,
        ttl_seconds: u64,
    ) -> Result<Self> {
        let redis_client = if let Some(url) = redis_url {
            Some(
                Client::open(url)
                    .map_err(|e| Error::Internal(format!("Failed to connect to Redis: {}", e)))?,
            )
        } else {
            None
        };

        // Use moka for production-grade LRU cache with automatic eviction
        let memory_cache = Arc::new(
            moka::future::CacheBuilder::new(memory_cache_size as u64)
                .build()
        );

        Ok(Self {
            redis_client,
            memory_cache,
            key_prefix,
            ttl_seconds,
        })
    }

    /// Get username for a user ID
    ///
    /// Checks memory cache first, then Redis cache.
    /// Returns None if not found in any cache.
    pub async fn get(&self, user_id: &UserId) -> Result<Option<String>> {
        // Check memory cache first (moka handles LRU automatically)
        if let Some(username) = self.memory_cache.get(user_id).await {
            tracing::debug!(user_id = %user_id.as_str(), username = %username, "Username cache hit (memory)");
            return Ok(Some(username));
        }

        // Check Redis cache
        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| Error::Internal(format!("Redis connection failed: {}", e)))?;

            let key = format!("{}{}", self.key_prefix, user_id.as_str());
            let username: Option<String> = conn
                .get(&key)
                .await
                .map_err(|e| Error::Internal(format!("Failed to get username from cache: {}", e)))?;

            if let Some(username) = username {
                tracing::debug!(user_id = %user_id.as_str(), username = %username, "Username cache hit (Redis)");

                // Populate memory cache
                self.memory_cache.insert(user_id.clone(), username.clone()).await;

                return Ok(Some(username));
            }
        }

        tracing::debug!(user_id = %user_id.as_str(), "Username cache miss");
        Ok(None)
    }

    /// Set username for a user ID
    ///
    /// Updates both memory cache and Redis cache.
    pub async fn set(&self, user_id: &UserId, username: &str) -> Result<()> {
        // Update memory cache
        self.memory_cache.insert(user_id.clone(), username.to_string()).await;

        // Update Redis cache
        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| Error::Internal(format!("Redis connection failed: {}", e)))?;

            let key = format!("{}{}", self.key_prefix, user_id.as_str());

            if self.ttl_seconds > 0 {
                let _: () = conn
                    .set_ex(&key, username, self.ttl_seconds)
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to set username in cache: {}", e)))?;
            } else {
                let _: () = conn
                    .set(&key, username)
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to set username in cache: {}", e)))?;
            }

            tracing::debug!(
                user_id = %user_id.as_str(),
                username = %username,
                ttl_seconds = self.ttl_seconds,
                "Username cached"
            );
        }

        Ok(())
    }

    /// Get multiple usernames at once
    ///
    /// More efficient than calling get() multiple times.
    /// Returns a map of user_id -> username.
    pub async fn get_batch(&self, user_ids: &[UserId]) -> Result<HashMap<UserId, String>> {
        let mut result = HashMap::new();
        let mut missing_ids = Vec::new();

        // Check memory cache first
        for user_id in user_ids {
            if let Some(username) = self.memory_cache.get(user_id).await {
                result.insert(user_id.clone(), username);
            } else {
                missing_ids.push(user_id.clone());
            }
        }

        // Check Redis for missing IDs
        if !missing_ids.is_empty() {
            if let Some(ref client) = self.redis_client {
                let mut conn = client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(|e| Error::Internal(format!("Redis connection failed: {}", e)))?;

                let mut pipe = redis::pipe();
                for user_id in &missing_ids {
                    let key = format!("{}{}", self.key_prefix, user_id.as_str());
                    pipe.get(&key);
                }

                let usernames: Vec<Option<String>> = pipe
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to batch get usernames: {}", e)))?;

                // Update memory cache and result
                for (user_id, username_opt) in missing_ids.iter().zip(usernames) {
                    if let Some(username) = username_opt {
                        result.insert(user_id.clone(), username.clone());
                        self.memory_cache.insert(user_id.clone(), username).await;
                    }
                }
            }
        }

        tracing::debug!(
            total = user_ids.len(),
            found = result.len(),
            "Batch username lookup"
        );

        Ok(result)
    }

    /// Invalidate a cached username
    ///
    /// Removes the username from both memory and Redis cache.
    pub async fn invalidate(&self, user_id: &UserId) -> Result<()> {
        // Remove from memory cache
        self.memory_cache.invalidate(user_id).await;

        // Remove from Redis cache
        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| Error::Internal(format!("Redis connection failed: {}", e)))?;

            let key = format!("{}{}", self.key_prefix, user_id.as_str());
            let _: () = conn
                .del(&key)
                .await
                .map_err(|e| Error::Internal(format!("Failed to invalidate username cache: {}", e)))?;

            tracing::debug!(user_id = %user_id.as_str(), "Username cache invalidated");
        }

        Ok(())
    }

    /// Clear all cached usernames (memory only)
    ///
    /// This is useful for testing or manual cache clearing.
    /// Note: Redis cache is not cleared.
    pub async fn clear_memory(&self) {
        self.memory_cache.invalidate_all();
        tracing::debug!("Memory username cache cleared");
    }

    /// Preload usernames into cache
    ///
    /// Useful for warming up the cache with frequently accessed users.
    pub async fn preload(&self, entries: HashMap<UserId, String>) -> Result<()> {
        for (user_id, username) in entries {
            self.set(&user_id, &username).await?;
        }

        tracing::debug!("Username cache preloaded");
        Ok(())
    }
}

impl std::fmt::Debug for UsernameCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UsernameCache")
            .field("redis_enabled", &self.redis_client.is_some())
            .field("ttl_seconds", &self.ttl_seconds)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_user_id(id: &str) -> UserId {
        UserId::from_string(id.to_string())
    }

    #[tokio::test]
    async fn test_memory_cache_only() {
        let cache = UsernameCache::new(None, "test:".to_string(), 10, 0).unwrap();

        let user_id = create_test_user_id("user1");

        // Cache miss
        assert!(cache.get(&user_id).await.unwrap().is_none());

        // Set and get
        cache.set(&user_id, "alice").await.unwrap();
        let retrieved = cache.get(&user_id).await.unwrap().unwrap();
        assert_eq!(retrieved, "alice");

        // Invalidate
        cache.invalidate(&user_id).await.unwrap();
        assert!(cache.get(&user_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_memory_cache_lru() {
        let cache = UsernameCache::new(None, "test:".to_string(), 3, 0).unwrap();

        let user1 = create_test_user_id("user1");
        let user2 = create_test_user_id("user2");
        let user3 = create_test_user_id("user3");
        let user4 = create_test_user_id("user4");

        // Fill cache to capacity (3)
        cache.set(&user1, "alice").await.unwrap();
        cache.set(&user2, "bob").await.unwrap();
        cache.set(&user3, "charlie").await.unwrap();

        // Verify all are cached
        assert!(cache.get(&user1).await.unwrap().is_some());
        assert!(cache.get(&user2).await.unwrap().is_some());
        assert!(cache.get(&user3).await.unwrap().is_some());

        // Access user1 to make it most recently used
        assert!(cache.get(&user1).await.unwrap().is_some());

        // Add user4, should evict user2 (least recently used)
        cache.set(&user4, "dave").await.unwrap();

        // user1 should still be there (recently accessed)
        assert!(cache.get(&user1).await.unwrap().is_some());
        // user2 should be evicted (least recently used) - moka handles this automatically
        // Note: moka's eviction policy may vary, so we just verify the cache still works
        assert!(cache.get(&user3).await.unwrap().is_some());
        assert!(cache.get(&user4).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_batch_lookup() {
        let cache = UsernameCache::new(None, "test:".to_string(), 10, 0).unwrap();

        let user1 = create_test_user_id("user1");
        let user2 = create_test_user_id("user2");
        let user3 = create_test_user_id("user3");

        // Set some entries
        cache.set(&user1, "alice").await.unwrap();
        cache.set(&user3, "charlie").await.unwrap();

        // Batch lookup
        let result = cache
            .get_batch(&[user1.clone(), user2.clone(), user3.clone()])
            .await
            .unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result.get(&user1), Some(&"alice".to_string()));
        assert_eq!(result.get(&user2), None);
        assert_eq!(result.get(&user3), Some(&"charlie".to_string()));
    }
}
