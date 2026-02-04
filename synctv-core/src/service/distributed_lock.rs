//! Distributed lock service using Redis
//!
//! Design reference: /Volumes/workspace/rust/synctv-rs-design/21-关键实现.md §12.2.3
//!
//! Provides distributed locking mechanism for multi-replica deployments.
//! Uses Redis SET NX EX for atomic lock acquisition.

use redis::aio::ConnectionManager as RedisConnectionManager;
use redis::Script;
use std::future::Future;
use crate::{Error, Result};

/// Distributed lock service
///
/// Provides Redis-based distributed locking for cross-replica critical sections
#[derive(Clone)]
pub struct DistributedLock {
    redis: RedisConnectionManager,
}

impl DistributedLock {
    /// Create a new distributed lock service
    #[must_use] 
    pub const fn new(redis: RedisConnectionManager) -> Self {
        Self { redis }
    }

    /// Acquire a lock (using SET NX EX atomic operation)
    ///
    /// Returns the lock value if acquired successfully, None if lock is already held
    ///
    /// # Arguments
    /// * `key` - Lock key (without "lock:" prefix)
    /// * `ttl_seconds` - Lock expiration time in seconds
    ///
    /// # Example
    /// ```ignore
    /// let lock_value = lock.acquire("create_room:user123", 10).await?;
    /// if let Some(value) = lock_value {
    ///     // Lock acquired, perform operation
    ///     // ...
    ///     lock.release("create_room:user123", &value).await?;
    /// } else {
    ///     // Lock already held by another process
    /// }
    /// ```
    pub async fn acquire(&self, key: &str, ttl_seconds: u64) -> Result<Option<String>> {
        let lock_key = format!("lock:{key}");
        let lock_value = crate::models::generate_id(); // nanoid(12)

        let mut conn = self.redis.clone();

        // SET key value NX EX ttl
        // NX: Only set if not exists
        // EX: Set expiration time
        let result: Option<String> = redis::cmd("SET")
            .arg(&lock_key)
            .arg(&lock_value)
            .arg("NX")
            .arg("EX")
            .arg(ttl_seconds)
            .query_async(&mut conn)
            .await
            .map_err(|e| Error::Internal(format!("Failed to acquire lock: {e}")))?;

        if result.is_some() {
            tracing::debug!(
                lock_key = %lock_key,
                lock_value = %lock_value,
                ttl_seconds = %ttl_seconds,
                "Lock acquired"
            );
            Ok(Some(lock_value))
        } else {
            tracing::debug!(
                lock_key = %lock_key,
                "Lock already held by another process"
            );
            Ok(None)
        }
    }

    /// Release a lock (using Lua script for atomicity)
    ///
    /// Only the lock holder (matching `lock_value`) can release the lock
    ///
    /// # Arguments
    /// * `key` - Lock key (without "lock:" prefix)
    /// * `lock_value` - The value returned by `acquire()`
    ///
    /// # Returns
    /// * `true` if lock was released successfully
    /// * `false` if lock was not held or already expired
    pub async fn release(&self, key: &str, lock_value: &str) -> Result<bool> {
        let lock_key = format!("lock:{key}");

        // Lua script: Only delete if the value matches
        // This prevents releasing a lock that was already expired and reacquired
        let script = Script::new(
            r#"
            if redis.call("GET", KEYS[1]) == ARGV[1] then
                return redis.call("DEL", KEYS[1])
            else
                return 0
            end
            "#,
        );

        let mut conn = self.redis.clone();

        let result: i32 = script
            .key(&lock_key)
            .arg(lock_value)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| Error::Internal(format!("Failed to release lock: {e}")))?;

        let released = result == 1;
        if released {
            tracing::debug!(
                lock_key = %lock_key,
                "Lock released"
            );
        } else {
            tracing::warn!(
                lock_key = %lock_key,
                "Lock release failed: value mismatch or already expired"
            );
        }

        Ok(released)
    }

    /// Execute an operation with automatic lock acquisition and release
    ///
    /// Uses RAII pattern to ensure lock is always released
    ///
    /// # Arguments
    /// * `key` - Lock key (without "lock:" prefix)
    /// * `ttl_seconds` - Lock expiration time in seconds
    /// * `operation` - Async function to execute while holding the lock
    ///
    /// # Returns
    /// * `Ok(T)` if lock was acquired and operation succeeded
    /// * `Err(Error::LockAcquisitionFailed)` if lock is already held
    /// * `Err(...)` if operation failed
    ///
    /// # Example
    /// ```ignore
    /// let result = lock.with_lock("create_room:user123", 10, || async {
    ///     // This code runs with lock held
    ///     room_service.create_room(request).await
    /// }).await?;
    /// ```
    pub async fn with_lock<F, Fut, T>(&self, key: &str, ttl_seconds: u64, operation: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        // Try to acquire lock
        let lock_value = self
            .acquire(key, ttl_seconds)
            .await?
            .ok_or_else(|| Error::Internal(format!("Failed to acquire lock: {key}")))?;

        // Execute operation
        let result = operation().await;

        // Always release lock, even if operation failed
        if let Err(e) = self.release(key, &lock_value).await {
            tracing::error!(
                key = %key,
                error = %e,
                "Failed to release lock after operation"
            );
        }

        result
    }

    /// Try to acquire a lock and execute an operation
    ///
    /// Returns None if lock is already held, Some(T) if operation succeeded
    ///
    /// # Example
    /// ```ignore
    /// match lock.try_with_lock("update_settings:room123", 10, || async {
    ///     room_service.update_settings(settings).await
    /// }).await? {
    ///     Some(result) => println!("Updated: {:?}", result),
    ///     None => println!("Lock already held, skipping update"),
    /// }
    /// ```
    pub async fn try_with_lock<F, Fut, T>(
        &self,
        key: &str,
        ttl_seconds: u64,
        operation: F,
    ) -> Result<Option<T>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        // Try to acquire lock
        let lock_value = match self.acquire(key, ttl_seconds).await? {
            Some(value) => value,
            None => return Ok(None), // Lock already held
        };

        // Execute operation
        let result = operation().await;

        // Always release lock
        if let Err(e) = self.release(key, &lock_value).await {
            tracing::error!(
                key = %key,
                error = %e,
                "Failed to release lock after operation"
            );
        }

        result.map(Some)
    }

    /// Extend lock TTL (refresh expiration)
    ///
    /// Useful for long-running operations that need to keep the lock
    ///
    /// # Returns
    /// * `true` if lock TTL was extended
    /// * `false` if lock doesn't exist or value mismatch
    pub async fn extend(&self, key: &str, lock_value: &str, ttl_seconds: u64) -> Result<bool> {
        let lock_key = format!("lock:{key}");

        // Lua script: Only extend if the value matches
        let script = Script::new(
            r#"
            if redis.call("GET", KEYS[1]) == ARGV[1] then
                return redis.call("EXPIRE", KEYS[1], ARGV[2])
            else
                return 0
            end
            "#,
        );

        let mut conn = self.redis.clone();

        let result: i32 = script
            .key(&lock_key)
            .arg(lock_value)
            .arg(ttl_seconds)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| Error::Internal(format!("Failed to extend lock: {e}")))?;

        Ok(result == 1)
    }
}

/// RAII lock guard that automatically releases on drop
///
/// # Example
/// ```ignore
/// let guard = LockGuard::new(&lock, "create_room:user123".to_string(), 10).await?;
/// // Lock is held
/// room_service.create_room(request).await?;
/// // Lock is automatically released when guard goes out of scope
/// ```
pub struct LockGuard {
    lock: DistributedLock,
    key: String,
    value: String,
}

impl LockGuard {
    /// Create a new lock guard (acquires lock)
    pub async fn new(lock: DistributedLock, key: String, ttl_seconds: u64) -> Result<Self> {
        let value = lock
            .acquire(&key, ttl_seconds)
            .await?
            .ok_or_else(|| Error::Internal(format!("Failed to acquire lock: {key}")))?;

        Ok(Self { lock, key, value })
    }

    /// Extend the lock TTL
    pub async fn extend(&self, ttl_seconds: u64) -> Result<bool> {
        self.lock.extend(&self.key, &self.value, ttl_seconds).await
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let lock = self.lock.clone();
        let key = self.key.clone();
        let value = self.value.clone();

        // Spawn async task to release lock
        tokio::spawn(async move {
            if let Err(e) = lock.release(&key, &value).await {
                tracing::error!(
                    key = %key,
                    error = %e,
                    "Failed to release lock in Drop"
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires Redis"]
    async fn test_acquire_and_release() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let lock = DistributedLock::new(redis);

        // Acquire lock
        let lock_value = lock.acquire("test:lock1", 10).await.unwrap();
        assert!(lock_value.is_some());

        let lock_value = lock_value.unwrap();

        // Try to acquire same lock (should fail)
        let lock_value2 = lock.acquire("test:lock1", 10).await.unwrap();
        assert!(lock_value2.is_none());

        // Release lock
        let released = lock.release("test:lock1", &lock_value).await.unwrap();
        assert!(released);

        // Acquire lock again (should succeed)
        let lock_value3 = lock.acquire("test:lock1", 10).await.unwrap();
        assert!(lock_value3.is_some());

        // Cleanup
        lock.release("test:lock1", &lock_value3.unwrap()).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "Requires Redis"]
    async fn test_with_lock() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let lock = DistributedLock::new(redis);

        let result = lock
            .with_lock("test:lock2", 10, || async {
                // Simulate operation
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                Ok::<_, Error>(42)
            })
            .await
            .unwrap();

        assert_eq!(result, 42);

        // Lock should be released, can acquire again
        let lock_value = lock.acquire("test:lock2", 10).await.unwrap();
        assert!(lock_value.is_some());

        // Cleanup
        lock.release("test:lock2", &lock_value.unwrap()).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "Requires Redis"]
    async fn test_try_with_lock() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let lock = DistributedLock::new(redis.clone());

        // Acquire lock manually
        let lock_value = lock.acquire("test:lock3", 10).await.unwrap().unwrap();

        // Try to execute with lock (should return None)
        let result = lock
            .try_with_lock("test:lock3", 10, || async { Ok::<_, Error>(42) })
            .await
            .unwrap();

        assert!(result.is_none());

        // Release lock
        lock.release("test:lock3", &lock_value).await.unwrap();

        // Try again (should succeed)
        let result = lock
            .try_with_lock("test:lock3", 10, || async { Ok::<_, Error>(42) })
            .await
            .unwrap();

        assert_eq!(result, Some(42));
    }

    #[tokio::test]
    #[ignore = "Requires Redis"]
    async fn test_lock_guard() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let lock = DistributedLock::new(redis.clone());

        {
            let _guard = LockGuard::new(lock.clone(), "test:lock4".to_string(), 10)
                .await
                .unwrap();

            // Lock is held
            let lock_value = lock.acquire("test:lock4", 10).await.unwrap();
            assert!(lock_value.is_none());

            // Guard will release lock when dropped
        }

        // Wait for async drop task to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Lock should be released
        let lock_value = lock.acquire("test:lock4", 10).await.unwrap();
        assert!(lock_value.is_some());

        // Cleanup
        lock.release("test:lock4", &lock_value.unwrap()).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "Requires Redis"]
    async fn test_extend_lock() {
        let redis_client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis = RedisConnectionManager::new(redis_client).await.unwrap();
        let lock = DistributedLock::new(redis);

        // Acquire lock with short TTL
        let lock_value = lock.acquire("test:lock5", 2).await.unwrap().unwrap();

        // Extend lock
        let extended = lock.extend("test:lock5", &lock_value, 10).await.unwrap();
        assert!(extended);

        // Lock should still be valid after original TTL
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        let lock_value2 = lock.acquire("test:lock5", 10).await.unwrap();
        assert!(lock_value2.is_none()); // Still locked

        // Cleanup
        lock.release("test:lock5", &lock_value).await.unwrap();
    }
}
