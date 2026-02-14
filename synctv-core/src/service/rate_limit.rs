use anyhow::Result;
use redis::AsyncCommands;
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

/// Rate limiter using Redis sliding window algorithm
///
/// Uses Redis sorted sets to implement accurate sliding window rate limiting
/// that works across multiple replicas.
#[derive(Clone)]
pub struct RateLimiter {
    redis_conn: Option<redis::aio::ConnectionManager>,
    key_prefix: String,
}

impl RateLimiter {
    /// Create a new `RateLimiter`
    ///
    /// If `redis_conn` is None, rate limiting is disabled (allows all requests)
    pub fn new(redis_conn: Option<redis::aio::ConnectionManager>, key_prefix: String) -> Self {
        Self {
            redis_conn,
            key_prefix,
        }
    }

    /// Create a disabled `RateLimiter` (no Redis, allows all requests)
    ///
    /// This is a convenience constructor that never fails, useful as a fallback
    /// when `RateLimiter::new` cannot be used with `?`.
    #[must_use]
    pub fn disabled(key_prefix: String) -> Self {
        Self {
            redis_conn: None,
            key_prefix,
        }
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
        // If Redis not configured, allow all requests
        let Some(ref conn) = self.redis_conn else {
            return Ok(());
        };

        let mut conn = conn.clone();

        let redis_key = format!("{}{}", self.key_prefix, key);
        let now = Self::current_timestamp_millis();
        let window_start = now.saturating_sub(window_seconds * 1000);

        // Use Redis pipeline for atomic operations
        let mut pipe = redis::pipe();
        pipe.atomic()
            // Remove entries older than the window
            .zrembyscore(&redis_key, 0, window_start as i64)
            .ignore()
            // Count current entries in window
            .zcard(&redis_key)
            // Add current request timestamp
            .zadd(&redis_key, now, now)
            .ignore()
            // Set expiration (window size + 1 second for cleanup)
            .expire(&redis_key, (window_seconds + 1) as i64)
            .ignore();

        let results: Vec<u32> = pipe
            .query_async(&mut conn)
            .await
            .map_err(RateLimitError::RedisError)?;

        // results[0] is the count of entries before adding the current request
        let current_count = results.first().copied().unwrap_or(0);

        if current_count >= max_requests {
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
        // If Redis not configured, return unlimited
        let Some(ref conn) = self.redis_conn else {
            return Ok((max_requests, 0));
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
        if let Some(ref conn) = self.redis_conn {
            let mut conn = conn.clone();
            let redis_key = format!("{}{}", self.key_prefix, key);
            let _: () = redis::cmd("DEL")
                .arg(&redis_key)
                .query_async(&mut conn)
                .await?;
        }
        Ok(())
    }

    /// Get current timestamp in milliseconds
    ///
    /// Returns 0 if system time is before Unix epoch (should never happen in practice).
    fn current_timestamp_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
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
    async fn test_without_redis_allows_all() {
        let limiter = RateLimiter::new(None, "test:".to_string());

        // Should allow unlimited requests
        for _ in 0..1000 {
            limiter
                .check_rate_limit("user:test:chat", 10, 1)
                .await
                .unwrap();
        }

        let (remaining, _) = limiter.get_quota("user:test:chat", 10, 1).await.unwrap();
        assert_eq!(remaining, 10); // Returns max as remaining
    }
}
