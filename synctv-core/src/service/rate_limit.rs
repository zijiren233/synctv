use anyhow::Result;
use redis::AsyncCommands;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Rate limiting error
#[derive(Error, Debug)]
pub enum RateLimitError {
    #[error("Rate limit exceeded. Try again in {retry_after_seconds}s")]
    RateLimitExceeded { retry_after_seconds: u64 },

    #[error("Redis error: {0}")]
    RedisError(#[from] redis::RedisError),
}

/// In-memory sliding window rate limiter (per-instance fallback when Redis is unavailable)
///
/// Uses a `DashMap` of `VecDeque<u64>` (timestamps) per key.
/// Each entry stores recent request timestamps; expired ones are pruned on access.
#[derive(Clone)]
struct InMemoryRateLimiter {
    /// key -> timestamps (sorted, oldest first)
    windows: Arc<dashmap::DashMap<String, VecDeque<u64>>>,
}

impl InMemoryRateLimiter {
    fn new() -> Self {
        Self {
            windows: Arc::new(dashmap::DashMap::new()),
        }
    }

    /// Check rate limit. Returns Ok(()) if allowed, or the retry_after seconds.
    fn check(&self, key: &str, max_requests: u32, window_seconds: u64) -> std::result::Result<(), u64> {
        let now_ms = Self::now_ms();
        let window_start_ms = now_ms.saturating_sub(window_seconds * 1000);

        let mut entry = self.windows.entry(key.to_string()).or_insert_with(VecDeque::new);
        let timestamps = entry.value_mut();

        // Remove expired timestamps from front
        while timestamps.front().map_or(false, |&ts| ts < window_start_ms) {
            timestamps.pop_front();
        }

        if timestamps.len() >= max_requests as usize {
            // Rate limited - compute retry_after from the oldest entry
            let oldest = timestamps.front().copied().unwrap_or(now_ms);
            let time_since_oldest = now_ms.saturating_sub(oldest);
            let remaining_ms = (window_seconds * 1000).saturating_sub(time_since_oldest);
            return Err((remaining_ms / 1000).max(1));
        }

        timestamps.push_back(now_ms);
        Ok(())
    }

    /// Get remaining quota
    fn quota(&self, key: &str, max_requests: u32, window_seconds: u64) -> (u32, u64) {
        let now_ms = Self::now_ms();
        let window_start_ms = now_ms.saturating_sub(window_seconds * 1000);

        let mut entry = self.windows.entry(key.to_string()).or_insert_with(VecDeque::new);
        let timestamps = entry.value_mut();

        while timestamps.front().map_or(false, |&ts| ts < window_start_ms) {
            timestamps.pop_front();
        }

        let current = timestamps.len() as u32;
        let remaining = max_requests.saturating_sub(current);
        let reset = timestamps.front().map_or(0, |&oldest| {
            let time_since = now_ms.saturating_sub(oldest);
            (window_seconds * 1000).saturating_sub(time_since) / 1000
        });
        (remaining, reset)
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }
}

/// Rate limiter using Redis sliding window algorithm
///
/// Uses Redis sorted sets to implement accurate sliding window rate limiting
/// that works across multiple replicas. Falls back to per-instance in-memory
/// rate limiting when Redis is not configured.
#[derive(Clone)]
pub struct RateLimiter {
    redis_conn: Option<redis::aio::ConnectionManager>,
    key_prefix: String,
    /// In-memory fallback (always present, used when redis_conn is None)
    in_memory: InMemoryRateLimiter,
}

impl RateLimiter {
    /// Create a new `RateLimiter`
    ///
    /// If `redis_conn` is None, falls back to per-instance in-memory rate limiting.
    pub fn new(redis_conn: Option<redis::aio::ConnectionManager>, key_prefix: String) -> Self {
        if redis_conn.is_none() {
            tracing::warn!(
                "Rate limiting using in-memory fallback: Redis not configured. \
                 Limits are per-instance only (not shared across replicas)."
            );
        }
        Self {
            redis_conn,
            key_prefix,
            in_memory: InMemoryRateLimiter::new(),
        }
    }

    /// Create a `RateLimiter` with in-memory fallback only (no Redis)
    #[must_use]
    pub fn in_memory_only(key_prefix: String) -> Self {
        Self {
            redis_conn: None,
            key_prefix,
            in_memory: InMemoryRateLimiter::new(),
        }
    }

    /// Check if Redis is connected and responding
    ///
    /// Returns Ok(()) if Redis is healthy, or an error if not configured or unreachable.
    pub async fn health_check(&self) -> Result<(), String> {
        let Some(ref conn) = self.redis_conn else {
            return Err("Redis not configured".to_string());
        };
        let mut conn = conn.clone();
        redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .map_err(|e| format!("Redis ping failed: {e}"))?;
        Ok(())
    }

    /// Check if a request is allowed under the rate limit
    ///
    /// Returns Ok(()) if allowed, or `RateLimitError` if rate limit exceeded
    ///
    /// # Arguments
    /// * `key` - Unique identifier for the rate limit (e.g., "`user:{user_id}:chat`")
    /// * `max_requests` - Maximum number of requests allowed in the window
    /// * `window_seconds` - Size of the sliding window in seconds
    pub async fn check_rate_limit(
        &self,
        key: &str,
        max_requests: u32,
        window_seconds: u64,
    ) -> Result<(), RateLimitError> {
        // If Redis not configured, use in-memory fallback
        let Some(ref conn) = self.redis_conn else {
            let mem_key = format!("{}{}", self.key_prefix, key);
            return self.in_memory.check(&mem_key, max_requests, window_seconds)
                .map_err(|retry_after_seconds| RateLimitError::RateLimitExceeded { retry_after_seconds });
        };

        let mut conn = conn.clone();

        let redis_key = format!("{}{}", self.key_prefix, key);
        let now = Self::current_timestamp_millis();
        let window_start = now.saturating_sub(window_seconds * 1000);
        let expire_seconds = (window_seconds + 1) as i64;

        // Use Lua script for true atomic rate limiting (prevents TOCTOU race)
        // The script: removes expired entries, adds the new request, counts, and sets expiry
        // Returns the count AFTER adding the current request
        let script = redis::Script::new(
            r"
            redis.call('ZREMRANGEBYSCORE', KEYS[1], 0, ARGV[1])
            redis.call('ZADD', KEYS[1], ARGV[2], ARGV[2])
            local count = redis.call('ZCARD', KEYS[1])
            redis.call('EXPIRE', KEYS[1], ARGV[3])
            return count
            "
        );

        let current_count: u32 = script
            .key(&redis_key)
            .arg(window_start as i64)
            .arg(now)
            .arg(expire_seconds)
            .invoke_async(&mut conn)
            .await
            .map_err(RateLimitError::RedisError)?;

        if current_count > max_requests {
            // Rate limit exceeded
            // Calculate retry_after by finding oldest entry in window
            let oldest: Option<u64> = conn
                .zrange_withscores(&redis_key, 0, 0)
                .await
                .ok()
                .and_then(|entries: Vec<(String, u64)>| entries.first().map(|(_, ts)| *ts));

            let retry_after_seconds = if let Some(oldest_ts) = oldest {
                let time_since_oldest = now.saturating_sub(oldest_ts);
                let remaining_window = (window_seconds * 1000).saturating_sub(time_since_oldest);
                (remaining_window / 1000).max(1)
            } else {
                1
            };

            return Err(RateLimitError::RateLimitExceeded {
                retry_after_seconds,
            });
        }

        Ok(())
    }

    /// Get remaining quota for a rate limit
    ///
    /// Returns (`remaining_requests`, `reset_time_seconds`)
    pub async fn get_quota(
        &self,
        key: &str,
        max_requests: u32,
        window_seconds: u64,
    ) -> Result<(u32, u64)> {
        // If Redis not configured, use in-memory fallback
        let Some(ref conn) = self.redis_conn else {
            let mem_key = format!("{}{}", self.key_prefix, key);
            return Ok(self.in_memory.quota(&mem_key, max_requests, window_seconds));
        };

        let mut conn = conn.clone();

        let redis_key = format!("{}{}", self.key_prefix, key);
        let now = Self::current_timestamp_millis();
        let window_start = now.saturating_sub(window_seconds * 1000);

        // Remove expired entries and count
        let mut pipe = redis::pipe();
        pipe.atomic()
            .zrembyscore(&redis_key, 0, window_start as i64)
            .ignore()
            .zcard(&redis_key);

        let results: Vec<u32> = pipe.query_async(&mut conn).await?;
        let current_count = results.first().copied().unwrap_or(0);

        let remaining = max_requests.saturating_sub(current_count);

        // Calculate reset time (when oldest entry expires)
        let oldest: Option<u64> = conn
            .zrange_withscores(&redis_key, 0, 0)
            .await
            .ok()
            .and_then(|entries: Vec<(String, u64)>| entries.first().map(|(_, ts)| *ts));

        let reset_seconds = if let Some(oldest_ts) = oldest {
            let time_since_oldest = now.saturating_sub(oldest_ts);
            let remaining_window = (window_seconds * 1000).saturating_sub(time_since_oldest);
            remaining_window / 1000
        } else {
            0
        };

        Ok((remaining, reset_seconds))
    }

    /// Reset rate limit for a key (for testing or admin purposes)
    pub async fn reset(&self, key: &str) -> Result<()> {
        let full_key = format!("{}{}", self.key_prefix, key);
        if let Some(ref conn) = self.redis_conn {
            let mut conn = conn.clone();
            let _: () = redis::cmd("DEL")
                .arg(&full_key)
                .query_async(&mut conn)
                .await?;
        }
        self.in_memory.windows.remove(&full_key);
        Ok(())
    }

    /// Get current timestamp in milliseconds
    ///
    /// Returns 0 if system time is before Unix epoch (should never happen in practice).
    fn current_timestamp_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }
}

/// Rate limit configuration
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub chat_per_second: u32,
    pub danmaku_per_second: u32,
    pub window_seconds: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            chat_per_second: 10,
            danmaku_per_second: 3,
            window_seconds: 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_without_redis() {
        let limiter = RateLimiter::new(None, "test:".to_string());
        assert!(limiter.redis_conn.is_none());
    }

    #[tokio::test]
    #[ignore = "Requires Redis server"]
    async fn test_rate_limit_basic() {
        let client = redis::Client::open("redis://127.0.0.1:6379").unwrap();
        let conn = redis::aio::ConnectionManager::new(client).await.unwrap();
        let limiter = RateLimiter::new(Some(conn), "test:".to_string());

        let key = "user:test1:chat";

        // Reset before test
        limiter.reset(key).await.unwrap();

        // First 10 requests should succeed
        for i in 0..10 {
            limiter
                .check_rate_limit(key, 10, 1)
                .await
                .unwrap_or_else(|_| panic!("Request {} should succeed", i));
        }

        // 11th request should fail
        let result = limiter.check_rate_limit(key, 10, 1).await;
        assert!(matches!(
            result,
            Err(RateLimitError::RateLimitExceeded { .. })
        ));

        // Wait for window to expire
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Should work again
        limiter.check_rate_limit(key, 10, 1).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "Requires Redis server"]
    async fn test_rate_limit_sliding_window() {
        let client = redis::Client::open("redis://127.0.0.1:6379").unwrap();
        let conn = redis::aio::ConnectionManager::new(client).await.unwrap();
        let limiter = RateLimiter::new(Some(conn), "test:".to_string());

        let key = "user:test2:chat";
        limiter.reset(key).await.unwrap();

        // Use 5 requests in 1 second window
        for _ in 0..5 {
            limiter.check_rate_limit(key, 5, 1).await.unwrap();
        }

        // Should be rate limited
        assert!(limiter.check_rate_limit(key, 5, 1).await.is_err());

        // Wait 0.6 seconds (more than half the window)
        tokio::time::sleep(tokio::time::Duration::from_millis(600)).await;

        // Still limited (sliding window)
        assert!(limiter.check_rate_limit(key, 5, 1).await.is_err());

        // Wait another 0.5 seconds (total 1.1s, oldest entries expired)
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Should work now
        limiter.check_rate_limit(key, 5, 1).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "Requires Redis server"]
    async fn test_get_quota() {
        let client = redis::Client::open("redis://127.0.0.1:6379").unwrap();
        let conn = redis::aio::ConnectionManager::new(client).await.unwrap();
        let limiter = RateLimiter::new(Some(conn), "test:".to_string());

        let key = "user:test3:chat";
        limiter.reset(key).await.unwrap();

        // Check initial quota
        let (remaining, _) = limiter.get_quota(key, 10, 1).await.unwrap();
        assert_eq!(remaining, 10);

        // Use 3 requests
        for _ in 0..3 {
            limiter.check_rate_limit(key, 10, 1).await.unwrap();
        }

        // Check remaining
        let (remaining, reset_time) = limiter.get_quota(key, 10, 1).await.unwrap();
        assert_eq!(remaining, 7);
        assert!(reset_time <= 1);
    }

    #[tokio::test]
    async fn test_without_redis_uses_in_memory_fallback() {
        let limiter = RateLimiter::new(None, "test:".to_string());

        let key = "user:test_mem:chat";

        // First 10 requests should succeed
        for i in 0..10 {
            limiter
                .check_rate_limit(key, 10, 1)
                .await
                .unwrap_or_else(|_| panic!("In-memory request {} should succeed", i));
        }

        // 11th request should be rate limited
        let result = limiter.check_rate_limit(key, 10, 1).await;
        assert!(
            matches!(result, Err(RateLimitError::RateLimitExceeded { .. })),
            "In-memory rate limiter should enforce limits"
        );

        // Check quota reflects usage
        let (remaining, _) = limiter.get_quota(key, 10, 1).await.unwrap();
        assert_eq!(remaining, 0);
    }
}
