//! User information cache (L1: Moka in-memory, L2: Redis)
//!
//! Provides fast access to user profile data with a two-tier caching strategy:
//! - L1: In-memory Moka cache (very fast, local to the node)
//! - L2: Redis cache (fast, shared across nodes)

use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{models::UserId, Error, Result};

/// User cache with L1 (Moka) + L2 (Redis) strategy
#[derive(Clone)]
pub struct UserCache {
    redis_client: Option<Client>,
    l1_cache: Arc<moka::future::Cache<UserId, CachedUser>>,
    l2_ttl_seconds: u64,
    key_prefix: String,
}

/// Cached user data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CachedUser {
    id: String,
    username: String,
    permissions: i64,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl UserCache {
    /// Create a new UserCache
    ///
    /// # Arguments
    /// * `redis_client` - Optional Redis client. If None, only L1 caching is used.
    /// * `l1_max_capacity` - Maximum number of entries in L1 cache
    /// * `l1_ttl_minutes` - TTL for L1 cache entries in minutes
    /// * `l2_ttl_seconds` - TTL for L2 (Redis) cache entries in seconds
    /// * `key_prefix` - Redis key prefix (e.g., "synctv:user:")
    pub fn new(
        redis_client: Option<Client>,
        l1_max_capacity: u64,
        l1_ttl_minutes: u64,
        l2_ttl_seconds: u64,
        key_prefix: String,
    ) -> Result<Self> {
        let l1_cache = moka::future::CacheBuilder::new(l1_max_capacity)
            .time_to_live(std::time::Duration::from_secs(l1_ttl_minutes as u64 * 60))
            .build();

        Ok(Self {
            redis_client,
            l1_cache: Arc::new(l1_cache),
            l2_ttl_seconds,
            key_prefix,
        })
    }

    /// Get user data from cache
    ///
    /// Checks L1 first, then L2. Returns None if not found in either cache.
    pub async fn get(&self, user_id: &UserId) -> Result<Option<CachedUser>> {
        // Check L1 (in-memory) cache first
        if let Some(user) = self.l1_cache.get(user_id).await {
            tracing::debug!(
                user_id = %user_id.as_str(),
                "User cache hit (L1)"
            );
            return Ok(Some(user));
        }

        // Check L2 (Redis) cache
        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| Error::Internal(format!("Redis connection failed: {}", e)))?;

            let key = format!("{}{}", self.key_prefix, user_id.as_str());
            let user_json: Option<String> = conn
                .get(&key)
                .await
                .map_err(|e| Error::Internal(format!("Failed to get user from cache: {}", e)))?;

            if let Some(json) = user_json {
                tracing::debug!(
                    user_id = %user_id.as_str(),
                    "User cache hit (L2)"
                );

                let user: CachedUser = serde_json::from_str(&json).map_err(|e| {
                    Error::Internal(format!("Failed to deserialize cached user: {}", e))
                })?;

                // Populate L1 cache
                self.l1_cache.insert(user_id.clone(), user.clone()).await;

                return Ok(Some(user));
            }
        }

        tracing::debug!(user_id = %user_id.as_str(), "User cache miss");
        Ok(None)
    }

    /// Set user data in cache
    ///
    /// Updates both L1 and L2 caches.
    pub async fn set(&self, user_id: &UserId, user: CachedUser) -> Result<()> {
        // Update L1 cache
        self.l1_cache.insert(user_id.clone(), user.clone()).await;

        // Update L2 cache
        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| Error::Internal(format!("Redis connection failed: {}", e)))?;

            let key = format!("{}{}", self.key_prefix, user_id.as_str());
            let json = serde_json::to_string(&user).map_err(|e| {
                Error::Internal(format!("Failed to serialize user for caching: {}", e))
            })?;

            if self.l2_ttl_seconds > 0 {
                let _: () = conn
                    .set_ex(&key, json, self.l2_ttl_seconds)
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to set user in cache: {}", e)))?;
            } else {
                let _: () = conn
                    .set(&key, json)
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to set user in cache: {}", e)))?;
            }

            tracing::debug!(
                user_id = %user_id.as_str(),
                ttl_seconds = self.l2_ttl_seconds,
                "User cached"
            );
        }

        Ok(())
    }

    /// Invalidate user data from cache
    ///
    /// Removes from both L1 and L2 caches.
    pub async fn invalidate(&self, user_id: &UserId) -> Result<()> {
        // Remove from L1 cache
        self.l1_cache.invalidate(user_id).await;

        // Remove from L2 cache
        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| Error::Internal(format!("Redis connection failed: {}", e)))?;

            let key = format!("{}{}", self.key_prefix, user_id.as_str());
            let _: () = conn
                .del(&key)
                .await
                .map_err(|e| Error::Internal(format!("Failed to invalidate user cache: {}", e)))?;

            tracing::debug!(user_id = %user_id.as_str(), "User cache invalidated");
        }

        Ok(())
    }

    /// Get multiple users at once
    ///
    /// More efficient than calling get() multiple times.
    /// Returns a map of user_id -> CachedUser.
    pub async fn get_batch(&self, user_ids: &[UserId]) -> Result<std::collections::HashMap<UserId, CachedUser>> {
        let mut result = std::collections::HashMap::new();
        let mut missing_ids = Vec::new();

        // Check L1 cache first
        for user_id in user_ids {
            if let Some(user) = self.l1_cache.get(user_id).await {
                result.insert(user_id.clone(), user);
            } else {
                missing_ids.push(user_id.clone());
            }
        }

        // Check L2 cache for missing IDs
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

                let user_jsons: Vec<Option<String>> = pipe
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to batch get users: {}", e)))?;

                // Update L1 cache and result
                for (user_id, user_json_opt) in missing_ids.iter().zip(user_jsons) {
                    if let Some(json) = user_json_opt {
                        if let Ok(user) = serde_json::from_str::<CachedUser>(&json) {
                            result.insert(user_id.clone(), user.clone());
                            self.l1_cache.insert(user_id.clone(), user).await;
                        }
                    }
                }
            }
        }

        tracing::debug!(
            total = user_ids.len(),
            found = result.len(),
            "Batch user lookup"
        );

        Ok(result)
    }

    /// Clear L1 cache (memory only)
    ///
    /// Useful for testing or manual cache clearing.
    /// Note: L2 cache is not cleared.
    pub async fn clear_l1(&self) {
        self.l1_cache.invalidate_all();
        tracing::debug!("L1 user cache cleared");
    }

    /// Get cache statistics
    pub async fn stats(&self) -> UserCacheStats {
        UserCacheStats {
            l1_size: self.l1_cache.entry_count(),
            l1_hit_count: 0, // Not tracked by moka cache
            l1_miss_count: 0, // Not tracked by moka cache
            l1_hit_rate: 0.0, // Not tracked by moka cache
        }
    }
}

/// User cache statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserCacheStats {
    pub l1_size: u64,
    pub l1_hit_count: u64,
    pub l1_miss_count: u64,
    pub l1_hit_rate: f64,
}

impl std::fmt::Debug for UserCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserCache")
            .field("redis_enabled", &self.redis_client.is_some())
            .field("l2_ttl_seconds", &self.l2_ttl_seconds)
            .field("key_prefix", &self.key_prefix)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_user_id(id: &str) -> UserId {
        UserId::from_string(id.to_string())
    }

    fn create_test_user(id: &str, username: &str) -> CachedUser {
        CachedUser {
            id: id.to_string(),
            username: username.to_string(),
            permissions: 0,
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_l1_cache_only() {
        let cache = UserCache::new(None, 100, 5, 0, "test:".to_string()).unwrap();

        let user_id = create_test_user_id("user1");
        let user = create_test_user("user1", "alice");

        // Cache miss
        assert!(cache.get(&user_id).await.unwrap().is_none());

        // Set and get
        cache.set(&user_id, user.clone()).await.unwrap();
        let retrieved = cache.get(&user_id).await.unwrap().unwrap();
        assert_eq!(retrieved.username, "alice");

        // Invalidate
        cache.invalidate(&user_id).await.unwrap();
        assert!(cache.get(&user_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_batch_lookup() {
        let cache = UserCache::new(None, 100, 5, 0, "test:".to_string()).unwrap();

        let user1 = create_test_user_id("user1");
        let user2 = create_test_user_id("user2");
        let user3 = create_test_user_id("user3");

        // Set some entries
        cache.set(&user1, create_test_user("user1", "alice")).await.unwrap();
        cache.set(&user3, create_test_user("user3", "charlie")).await.unwrap();

        // Batch lookup
        let result = cache
            .get_batch(&[user1.clone(), user2.clone(), user3.clone()])
            .await
            .unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result.get(&user1).map(|u| &u.username), Some(&"alice".to_string()));
        assert_eq!(result.get(&user2), None);
        assert_eq!(result.get(&user3).map(|u| &u.username), Some(&"charlie".to_string()));
    }

    #[tokio::test]
    async fn test_stats() {
        let cache = UserCache::new(None, 100, 5, 0, "test:".to_string()).unwrap();

        let user_id = create_test_user_id("user1");
        let user = create_test_user("user1", "alice");

        // Miss
        cache.get(&user_id).await.unwrap();

        // Set and hit
        cache.set(&user_id, user).await.unwrap();
        cache.get(&user_id).await.unwrap();

        let stats = cache.stats().await;
        assert_eq!(stats.l1_miss_count, 1);
        assert_eq!(stats.l1_hit_count, 1);
        assert_eq!(stats.l1_size, 1);
    }
}
