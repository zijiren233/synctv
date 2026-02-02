//! Cache manager for coordinating multiple cache layers
//!
//! Provides a unified interface for managing all cache layers and statistics.

use super::{user_cache::UserCache, room_cache::RoomCache, CacheConfig, CacheStats};
use redis::Client;
use std::sync::Arc;

use crate::{Error, Result};

/// Cache manager that coordinates all cache layers
#[derive(Clone)]
pub struct CacheManager {
    pub user_cache: Arc<UserCache>,
    pub room_cache: Arc<RoomCache>,
    config: CacheConfig,
}

impl CacheManager {
    /// Create a new cache manager with Redis support
    ///
    /// # Arguments
    /// * `redis_url` - Optional Redis URL. If None, only L1 (in-memory) caching is used.
    /// * `config` - Cache configuration
    pub fn new(redis_url: Option<String>, config: CacheConfig) -> Result<Self> {
        let redis_client = if let Some(url) = redis_url {
            Some(
                Client::open(url)
                    .map_err(|e| Error::Internal(format!("Failed to connect to Redis: {}", e)))?,
            )
        } else {
            None
        };

        let l1_ttl_minutes = config.l1_ttl.as_secs() / 60;

        let user_cache = Arc::new(UserCache::new(
            redis_client.clone(),
            config.l1_max_capacity as u64,
            l1_ttl_minutes as u64,
            config.l2_ttl.as_secs() as u64,
            format!("{}user:", config.redis_key_prefix),
        )?);

        let room_cache = Arc::new(RoomCache::new(
            redis_client,
            config.l1_max_capacity as u64,
            l1_ttl_minutes as u64,
            config.l2_ttl.as_secs() as u64,
            format!("{}room:", config.redis_key_prefix),
        )?);

        Ok(Self {
            user_cache,
            room_cache,
            config,
        })
    }

    /// Create a cache manager with default configuration
    pub fn with_defaults(redis_url: Option<String>) -> Result<Self> {
        Self::new(redis_url, CacheConfig::default())
    }

    /// Clear all L1 caches (memory only)
    ///
    /// Useful for testing or manual cache clearing.
    /// Note: L2 (Redis) caches are not cleared.
    pub async fn clear_all_l1(&self) {
        self.user_cache.clear_l1().await;
        self.room_cache.clear_l1().await;
        tracing::debug!("All L1 caches cleared");
    }

    /// Get aggregated cache statistics
    pub async fn aggregated_stats(&self) -> AggregatedCacheStats {
        let user_stats = self.user_cache.stats().await;
        let room_stats = self.room_cache.stats().await;

        let total_hits = user_stats.l1_hit_count + room_stats.l1_hit_count;
        let total_misses = user_stats.l1_miss_count + room_stats.l1_miss_count;
        let total_entries = user_stats.l1_size + room_stats.l1_size;
        let total_capacity = 2 * self.config.l1_max_capacity as u64;

        let hit_rate = if total_hits + total_misses > 0 {
            total_hits as f64 / (total_hits + total_misses) as f64
        } else {
            0.0
        };

        AggregatedCacheStats {
            total_entries,
            total_capacity,
            hit_rate,
            user_stats,
            room_stats,
        }
    }

    /// Get cache configuration
    pub fn config(&self) -> &CacheConfig {
        &self.config
    }
}

impl std::fmt::Debug for CacheManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheManager")
            .field("config", &self.config)
            .finish()
    }
}

/// Aggregated cache statistics across all cache types
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AggregatedCacheStats {
    /// Total number of entries across all L1 caches
    pub total_entries: u64,
    /// Total capacity across all L1 caches
    pub total_capacity: u64,
    /// Overall cache hit rate (L1 only)
    pub hit_rate: f64,
    /// User cache statistics
    pub user_stats: super::user_cache::UserCacheStats,
    /// Room cache statistics
    pub room_stats: super::room_cache::RoomCacheStats,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_manager_creation() {
        let manager = CacheManager::with_defaults(None).unwrap();
        assert_eq!(manager.config().l1_max_capacity, 10_000);
    }

    #[tokio::test]
    async fn test_clear_all_l1() {
        let manager = CacheManager::with_defaults(None).unwrap();

        // This should not panic
        manager.clear_all_l1().await;
    }

    #[tokio::test]
    async fn test_aggregated_stats() {
        let manager = CacheManager::with_defaults(None).unwrap();
        let stats = manager.aggregated_stats().await;

        // Should have zero entries initially
        assert_eq!(stats.total_entries, 0);
        assert_eq!(stats.total_capacity, 20_000);
        assert_eq!(stats.hit_rate, 0.0);
    }
}
