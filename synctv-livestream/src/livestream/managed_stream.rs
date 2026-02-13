// Shared lifecycle state and pool utilities for managed streams.
//
// Both PullStreamManager and ExternalPublishManager follow the same pattern:
// - Streams tracked in a DashMap with double-checked locking for creation
// - Subscriber counting, health checks, last-active tracking
// - Background cleanup task that stops idle streams
//
// This module extracts the common parts.

use anyhow::Result;
use dashmap::DashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{self as log, Instrument};

/// Common lifecycle state shared by all managed streams.
///
/// Handles subscriber counting, health tracking, last-active timestamps,
/// and task handle management. Embed this in your stream struct and delegate.
pub struct StreamLifecycle {
    subscriber_count: AtomicUsize,
    last_active: Mutex<Instant>,
    is_running: AtomicBool,
    task_handle: Mutex<Option<tokio::task::JoinHandle<Result<()>>>>,
}

impl StreamLifecycle {
    #[must_use]
    pub fn new() -> Self {
        Self {
            subscriber_count: AtomicUsize::new(0),
            last_active: Mutex::new(Instant::now()),
            is_running: AtomicBool::new(false),
            task_handle: Mutex::new(None),
        }
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscriber_count.load(Ordering::SeqCst)
    }

    pub fn increment_subscriber_count(&self) {
        self.subscriber_count.fetch_add(1, Ordering::SeqCst);
    }

    pub fn decrement_subscriber_count(&self) {
        let result = self
            .subscriber_count
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                if v > 0 { Some(v - 1) } else { None }
            });
        if result.is_err() {
            tracing::warn!("Attempted to decrement subscriber count below zero");
        }
    }

    pub async fn is_healthy(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    pub fn set_running(&self) {
        self.is_running.store(true, Ordering::SeqCst);
    }

    /// Mark as stopping — new `is_healthy()` calls return false.
    pub fn mark_stopping(&self) {
        self.is_running.store(false, Ordering::SeqCst);
    }

    /// Restore running state (used when cleanup detects a late subscriber).
    pub fn restore_running(&self) {
        self.is_running.store(true, Ordering::SeqCst);
    }

    pub async fn last_active_time(&self) -> Instant {
        *self.last_active.lock().await
    }

    pub async fn update_last_active_time(&self) {
        *self.last_active.lock().await = Instant::now();
    }

    pub async fn set_task_handle(&self, handle: tokio::task::JoinHandle<Result<()>>) {
        *self.task_handle.lock().await = Some(handle);
    }

    pub async fn abort_task(&self) {
        if let Some(handle) = self.task_handle.lock().await.take() {
            handle.abort();
        }
    }
}

impl Default for StreamLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for streams managed by [`StreamPool`].
pub trait ManagedStream: Send + Sync + 'static {
    fn lifecycle(&self) -> &StreamLifecycle;
    fn stream_key(&self) -> String;
}

/// Creation lock entry with last access time for cleanup
struct CreationLockEntry {
    lock: Arc<tokio::sync::Mutex<()>>,
    last_accessed: AtomicUsize, // stores seconds since Unix epoch as usize
}

impl CreationLockEntry {
    fn new() -> Self {
        Self {
            lock: Arc::new(tokio::sync::Mutex::new(())),
            last_accessed: AtomicUsize::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as usize)
                    .unwrap_or(0),
            ),
        }
    }

    fn touch(&self) {
        if let Ok(d) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            self.last_accessed.store(d.as_secs() as usize, Ordering::Relaxed);
        }
    }

    fn age_seconds(&self) -> u64 {
        if let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            let last = self.last_accessed.load(Ordering::Relaxed) as u64;
            now.as_secs().saturating_sub(last)
        } else {
            0
        }
    }
}

/// Generic stream pool with double-checked locking and idle cleanup.
///
/// Provides the common infrastructure for both `PullStreamManager` and
/// `ExternalPublishManager`: creation locks, fast-path reuse of healthy
/// streams, and background idle cleanup.
pub struct StreamPool<S: ManagedStream> {
    pub streams: Arc<DashMap<String, Arc<S>>>,
    creation_locks: Arc<DashMap<String, Arc<CreationLockEntry>>>,
    pub cleanup_check_interval: Duration,
    pub idle_timeout: Duration,
    /// Maximum age of unused creation locks before cleanup (prevents memory leak)
    creation_lock_max_age: Duration,
}

impl<S: ManagedStream> StreamPool<S> {
    pub fn new(cleanup_check_interval: Duration, idle_timeout: Duration) -> Self {
        Self {
            streams: Arc::new(DashMap::new()),
            creation_locks: Arc::new(DashMap::new()),
            cleanup_check_interval,
            idle_timeout,
            // Clean up creation locks that haven't been used for 10 minutes
            creation_lock_max_age: Duration::from_secs(600),
        }
    }

    /// Try to reuse an existing healthy stream (fast path, no lock).
    ///
    /// Increments subscriber count and updates last-active if found.
    /// Returns `None` and removes the unhealthy entry if the stream is stale.
    pub async fn get_existing(&self, stream_key: &str) -> Option<Arc<S>> {
        if let Some(stream) = self.streams.get(stream_key) {
            if stream.lifecycle().is_healthy().await {
                stream.lifecycle().increment_subscriber_count();
                stream.lifecycle().update_last_active_time().await;
                return Some(stream.clone());
            }
            drop(stream);
            self.streams.remove(stream_key);
        }
        None
    }

    /// Acquire the per-key creation lock. Hold the returned guard while
    /// creating the stream to prevent duplicate creation.
    pub async fn acquire_creation_lock(
        &self,
        stream_key: &str,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        let entry = self
            .creation_locks
            .entry(stream_key.to_string())
            .or_insert_with(|| Arc::new(CreationLockEntry::new()));
        entry.touch();
        let lock = Arc::clone(&entry.lock);
        lock.lock_owned().await
    }

    /// Remove the creation lock for a stream key (called when stream is destroyed)
    pub fn remove_creation_lock(&self, stream_key: &str) {
        self.creation_locks.remove(stream_key);
    }

    /// Periodically clean up old unused creation locks to prevent memory leak
    pub fn cleanup_old_creation_locks(&self) {
        let max_age = self.creation_lock_max_age;
        self.creation_locks.retain(|_key, entry| {
            entry.age_seconds() < max_age.as_secs()
        });
    }

    /// Start a background task that periodically cleans up stale creation locks.
    ///
    /// This should be called once during initialization to prevent memory leaks
    /// from failed stream creation attempts that leave orphaned lock entries.
    pub fn start_creation_lock_cleanup(&self) -> tokio::task::JoinHandle<()> {
        let creation_locks = Arc::clone(&self.creation_locks);
        let max_age = self.creation_lock_max_age;
        let check_interval = self.cleanup_check_interval;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(check_interval);
            loop {
                interval.tick().await;
                let before = creation_locks.len();
                creation_locks.retain(|_key, entry| {
                    entry.age_seconds() < max_age.as_secs()
                });
                let after = creation_locks.len();
                if before != after {
                    log::debug!(
                        "Cleaned up {} stale creation lock entries",
                        before - after
                    );
                }
            }
        })
    }

    /// Insert a stream and spawn the idle cleanup task.
    ///
    /// `on_idle_cleanup` is called during cleanup, before stopping the stream.
    /// Use it for extra teardown (e.g., Redis unregistration).
    pub fn insert_and_cleanup<F>(
        &self,
        stream_key: String,
        stream: Arc<S>,
        on_idle_cleanup: F,
    ) where
        F: Fn(&str) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            + Send
            + Sync
            + 'static,
    {
        self.streams.insert(stream_key.clone(), Arc::clone(&stream));

        let streams = Arc::clone(&self.streams);
        let creation_locks = Arc::clone(&self.creation_locks);
        let check_interval = self.cleanup_check_interval;
        let idle_timeout = self.idle_timeout;

        let span = tracing::info_span!("stream_cleanup", stream_key = %stream_key);
        tokio::spawn(
            async move {
                let result = Self::cleanup_loop(
                    &stream_key,
                    &stream,
                    &streams,
                    &creation_locks,
                    check_interval,
                    idle_timeout,
                    &on_idle_cleanup,
                )
                .await;
                if let Err(e) = result {
                    log::error!("Cleanup task failed for {}: {}", stream_key, e);
                    stream.lifecycle().abort_task().await;
                    streams.remove(&stream_key);
                    creation_locks.remove(&stream_key);
                }
            }
            .instrument(span),
        );
    }

    async fn cleanup_loop<F>(
        stream_key: &str,
        stream: &Arc<S>,
        streams: &Arc<DashMap<String, Arc<S>>>,
        creation_locks: &Arc<DashMap<String, Arc<CreationLockEntry>>>,
        check_interval: Duration,
        idle_timeout: Duration,
        on_idle_cleanup: &F,
    ) -> Result<()>
    where
        F: Fn(&str) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            + Send
            + Sync
            + 'static,
    {
        let mut interval = tokio::time::interval(check_interval);

        loop {
            interval.tick().await;

            if stream.lifecycle().subscriber_count() == 0 {
                let idle_time = stream.lifecycle().last_active_time().await.elapsed();

                if idle_time > idle_timeout {
                    // Mark stopping FIRST so concurrent viewers see it as unhealthy
                    stream.lifecycle().mark_stopping();

                    // Re-check: a concurrent viewer may have incremented after our check
                    if stream.lifecycle().subscriber_count() > 0 {
                        log::debug!(
                            "Cleanup aborted for {}: late subscriber detected",
                            stream_key,
                        );
                        stream.lifecycle().restore_running();
                        continue;
                    }

                    log::info!(
                        "Auto cleanup: Stopping stream {} (idle for {:?})",
                        stream_key,
                        idle_time
                    );

                    // Run extra cleanup (e.g., Redis unregistration)
                    on_idle_cleanup(stream_key).await;

                    // Remove from map and stop
                    streams.remove(stream_key);
                    // Also remove the creation lock to prevent memory leak
                    creation_locks.remove(stream_key);
                    stream.lifecycle().abort_task().await;
                    break;
                }
            } else {
                stream.lifecycle().update_last_active_time().await;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestStream {
        lifecycle: StreamLifecycle,
        key: String,
    }

    impl ManagedStream for TestStream {
        fn lifecycle(&self) -> &StreamLifecycle {
            &self.lifecycle
        }
        fn stream_key(&self) -> String {
            self.key.clone()
        }
    }

    #[tokio::test]
    async fn test_stream_lifecycle_defaults() {
        let lc = StreamLifecycle::new();
        assert_eq!(lc.subscriber_count(), 0);
        assert!(!lc.is_healthy().await);
    }

    #[tokio::test]
    async fn test_stream_lifecycle_subscriber_count() {
        let lc = StreamLifecycle::new();
        lc.increment_subscriber_count();
        assert_eq!(lc.subscriber_count(), 1);
        lc.increment_subscriber_count();
        assert_eq!(lc.subscriber_count(), 2);
        lc.decrement_subscriber_count();
        assert_eq!(lc.subscriber_count(), 1);
        lc.decrement_subscriber_count();
        assert_eq!(lc.subscriber_count(), 0);
        // Underflow should be a no-op
        lc.decrement_subscriber_count();
        assert_eq!(lc.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn test_stream_lifecycle_health() {
        let lc = StreamLifecycle::new();
        assert!(!lc.is_healthy().await);

        lc.set_running();
        assert!(lc.is_healthy().await);

        lc.mark_stopping();
        assert!(!lc.is_healthy().await);

        lc.restore_running();
        assert!(lc.is_healthy().await);
    }

    #[tokio::test]
    async fn test_stream_pool_get_existing_empty() {
        let pool: StreamPool<TestStream> =
            StreamPool::new(Duration::from_secs(60), Duration::from_secs(300));
        assert!(pool.get_existing("key").await.is_none());
    }

    #[tokio::test]
    async fn test_stream_pool_get_existing_healthy() {
        let pool: StreamPool<TestStream> =
            StreamPool::new(Duration::from_secs(60), Duration::from_secs(300));

        let stream = Arc::new(TestStream {
            lifecycle: StreamLifecycle::new(),
            key: "room:media".to_string(),
        });
        stream.lifecycle().set_running();

        pool.streams.insert("room:media".to_string(), stream);

        let found = pool.get_existing("room:media").await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().lifecycle().subscriber_count(), 1);
    }

    #[tokio::test]
    async fn test_stream_pool_get_existing_unhealthy_removed() {
        let pool: StreamPool<TestStream> =
            StreamPool::new(Duration::from_secs(60), Duration::from_secs(300));

        let stream = Arc::new(TestStream {
            lifecycle: StreamLifecycle::new(),
            key: "room:media".to_string(),
        });
        // Not running, so unhealthy

        pool.streams.insert("room:media".to_string(), stream);

        let found = pool.get_existing("room:media").await;
        assert!(found.is_none());
        assert!(pool.streams.is_empty());
    }
}
