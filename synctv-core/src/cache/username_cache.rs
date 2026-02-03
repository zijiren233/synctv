use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{models::UserId, Error, Result};

/// Username cache service for fast username lookups
///
/// Uses a two-tier caching strategy:
/// 1. In-memory LRU cache for frequently accessed usernames
/// 2. Redis persistent cache for cross-node consistency
#[derive(Clone)]
pub struct UsernameCache {
    redis_client: Option<Client>,
    memory_cache: Arc<RwLock<MemoryCache>>,
    key_prefix: String,
    memory_cache_size: usize,
    ttl_seconds: u64,
}

/// In-memory cache with LRU eviction
struct MemoryCache {
    entries: HashMap<UserId, CacheEntry>,
    access_order: Vec<UserId>,
}

#[derive(Clone)]
struct CacheEntry {
    username: String,
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

        Ok(Self {
            redis_client,
            memory_cache: Arc::new(RwLock::new(MemoryCache::new(memory_cache_size))),
            key_prefix,
            memory_cache_size,
            ttl_seconds,
        })
    }

    /// Get username for a user ID
    ///
    /// Checks memory cache first, then Redis cache.
    /// Returns None if not found in any cache.
    pub async fn get(&self, user_id: &UserId) -> Result<Option<String>> {
        // Check memory cache first
        {
            let cache = self.memory_cache.read().await;
            if let Some(username) = cache.lookup(user_id) {
                tracing::debug!(user_id = %user_id.as_str(), username = %username, "Username cache hit (memory)");
                return Ok(Some(username));
            }
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
                let mut cache = self.memory_cache.write().await;
                cache.put(user_id.clone(), username.clone());

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
        {
            let mut cache = self.memory_cache.write().await;
            cache.put(user_id.clone(), username.to_string());
        }

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
        {
            let cache = self.memory_cache.read().await;
            for user_id in user_ids {
                if let Some(username) = cache.lookup(user_id) {
                    result.insert(user_id.clone(), username);
                } else {
                    missing_ids.push(user_id.clone());
                }
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
                let mut cache = self.memory_cache.write().await;
                for (user_id, username_opt) in missing_ids.iter().zip(usernames) {
                    if let Some(username) = username_opt {
                        result.insert(user_id.clone(), username.clone());
                        cache.put(user_id.clone(), username);
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
        {
            let mut cache = self.memory_cache.write().await;
            cache.remove(user_id);
        }

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
        let mut cache = self.memory_cache.write().await;
        cache.clear();
        tracing::debug!("Memory username cache cleared");
    }

    /// Preload usernames into cache
    ///
    /// Useful for warming up the cache with frequently accessed users.
    pub async fn preload(&self, entries: HashMap<UserId, String>) -> Result<()> {
        for (user_id, username) in entries {
            self.set(&user_id, &username).await?;
        }

        tracing::debug!(count = ?self.memory_cache.read().await.len(), "Username cache preloaded");
        Ok(())
    }

    /// Get cache statistics
    pub async fn stats(&self) -> CacheStats {
        let cache = self.memory_cache.read().await;
        CacheStats {
            memory_size: cache.len(),
            memory_capacity: self.memory_cache_size,
        }
    }
}

impl MemoryCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            access_order: Vec::with_capacity(capacity),
        }
    }

    /// Get username without updating access order (for read-only access)
    fn lookup(&self, user_id: &UserId) -> Option<String> {
        self.entries.get(user_id).map(|entry| entry.username.clone())
    }

    fn put(&mut self, user_id: UserId, username: String) {
        let entry = CacheEntry {
            username,
        };

        // Update access order
        self.access_order.retain(|id| id != &user_id);
        self.access_order.push(user_id.clone());

        // Evict if over capacity
        while self.access_order.len() > self.entries.capacity().max(1) {
            if let Some(evicted) = self.access_order.first() {
                self.entries.remove(evicted);
                self.access_order.remove(0);
            } else {
                break;
            }
        }

        self.entries.insert(user_id, entry);
    }

    fn remove(&mut self, user_id: &UserId) {
        self.entries.remove(user_id);
        self.access_order.retain(|id| id != user_id);
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.access_order.clear();
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Cache statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub memory_size: usize,
    pub memory_capacity: usize,
}

impl std::fmt::Debug for UsernameCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UsernameCache")
            .field("redis_enabled", &self.redis_client.is_some())
            .field("memory_cache_size", &self.memory_cache_size)
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
        assert_eq!(cache.get(&user_id).await.unwrap(), Some("alice".to_string()));

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

        // Fill cache
        cache.set(&user1, "alice").await.unwrap();
        cache.set(&user2, "bob").await.unwrap();
        cache.set(&user3, "charlie").await.unwrap();

        // Access user1 to make it most recently used
        cache.get(&user1).await.unwrap();

        // Add user4, should evict user2 (least recently used)
        cache.set(&user4, "dave").await.unwrap();

        assert!(cache.get(&user1).await.unwrap().is_some());
        assert!(cache.get(&user2).await.unwrap().is_none()); // Evicted
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

    #[tokio::test]
    async fn test_stats() {
        let cache = UsernameCache::new(None, "test:".to_string(), 10, 0).unwrap();

        let user1 = create_test_user_id("user1");
        cache.set(&user1, "alice").await.unwrap();

        let stats = cache.stats().await;
        assert_eq!(stats.memory_size, 1);
        assert_eq!(stats.memory_capacity, 10);
    }
}
