//! Message deduplication for cross-cluster synchronization
//!
//! Prevents duplicate processing of events when:
//! - Multiple Redis subscribers exist
//! - Network issues cause retries
//! - Events are published multiple times

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Deduplication key for events
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct DedupKey {
    pub event_type: String,
    pub room_id: String,
    pub user_id: String,
    /// Extra discriminator for events without `room_id/user_id` (e.g. `SystemNotification` message)
    pub extra: String,
    pub timestamp_ms: i64,
}

impl DedupKey {
    /// Create a deduplication key from a cluster event
    #[must_use]
    pub fn from_event(event: &crate::sync::events::ClusterEvent) -> Self {
        Self {
            event_type: event.event_type().to_string(),
            room_id: event.room_id()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            user_id: event.user_id()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            extra: event.dedup_extra(),
            timestamp_ms: event.timestamp().timestamp_millis(),
        }
    }
}

/// Deduplication entry with expiration time
#[derive(Clone)]
struct DedupEntry {
    expires_at: Instant,
}

/// Message deduplicator with automatic cleanup
#[derive(Clone)]
pub struct MessageDeduplicator {
    /// Map of dedup keys to expiration times
    entries: Arc<DashMap<DedupKey, DedupEntry>>,
    /// Dedup window duration (events older than this are accepted)
    dedup_window: Duration,
    /// Cleanup interval
    cleanup_interval: Duration,
    /// Cancellation token for graceful shutdown of the cleanup task
    cancel_token: CancellationToken,
}

impl MessageDeduplicator {
    /// Create a new deduplicator
    ///
    /// # Arguments
    /// * `dedup_window` - How long to remember events (default 5 seconds)
    /// * `cleanup_interval` - How often to clean expired entries (default 30 seconds)
    #[must_use]
    pub fn new(dedup_window: Duration, cleanup_interval: Duration) -> Self {
        let cancel_token = CancellationToken::new();
        let dedup = Self {
            entries: Arc::new(DashMap::new()),
            dedup_window,
            cleanup_interval,
            cancel_token: cancel_token,
        };

        // Start cleanup task with cancellation support
        let dedup_clone = dedup.clone();
        tokio::spawn(async move {
            dedup_clone.run_cleanup().await;
        });

        dedup
    }

    /// Create with default settings (5 second window)
    #[must_use] 
    pub fn with_defaults() -> Self {
        Self::new(
            Duration::from_secs(5),
            Duration::from_secs(30),
        )
    }

    /// Check if an event should be processed (not a duplicate)
    ///
    /// Uses `DashMap::entry()` for atomic check-and-insert to prevent TOCTOU races
    /// where two concurrent calls for the same key could both return `true`.
    #[must_use]
    pub fn should_process(&self, key: &DedupKey) -> bool {
        let now = Instant::now();
        let new_expiry = now + self.dedup_window;

        match self.entries.entry(key.clone()) {
            dashmap::mapref::entry::Entry::Occupied(mut entry) => {
                if entry.get().expires_at > now {
                    // Within dedup window, skip (duplicate)
                    false
                } else {
                    // Expired, refresh and process
                    entry.insert(DedupEntry { expires_at: new_expiry });
                    true
                }
            }
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                // Key doesn't exist, insert and process
                entry.insert(DedupEntry { expires_at: new_expiry });
                true
            }
        }
    }

    /// Mark an event as processed
    pub fn mark_processed(&self, key: DedupKey) {
        let now = Instant::now();
        self.entries.insert(key, DedupEntry {
            expires_at: now + self.dedup_window,
        });
    }

    /// Shutdown the deduplicator and its cleanup task
    pub fn shutdown(&self) {
        self.cancel_token.cancel();
    }

    /// Run periodic cleanup of expired entries (stops when cancelled)
    async fn run_cleanup(&self) {
        let mut interval = tokio::time::interval(self.cleanup_interval);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    self.cleanup_expired();
                }
                () = self.cancel_token.cancelled() => {
                    tracing::debug!("Deduplicator cleanup task shutting down");
                    return;
                }
            }
        }
    }

    /// Clean up expired entries
    fn cleanup_expired(&self) {
        let now = Instant::now();

        self.entries.retain(|_key, entry| {
            entry.expires_at > now
        });
    }

    /// Get the number of tracked events
    #[must_use] 
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if there are any tracked events
    #[must_use] 
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all tracked events (for testing)
    pub fn clear(&self) {
        self.entries.clear();
    }
}

impl Default for MessageDeduplicator {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl Drop for MessageDeduplicator {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use synctv_core::models::id::{RoomId, UserId};

    #[tokio::test]
    async fn test_dedup_basic() {
        let dedup = MessageDeduplicator::with_defaults();

        let key = DedupKey {
            event_type: "chat".to_string(),
            room_id: "room1".to_string(),
            user_id: "user1".to_string(),
            extra: String::new(),
            timestamp_ms: 1000,
        };

        // First call should return true
        assert!(dedup.should_process(&key));

        // Immediate second call should return false (duplicate)
        assert!(!dedup.should_process(&key));

        // Wait for expiration (simulated by clearing)
        dedup.clear();

        // After expiration, should process again
        assert!(dedup.should_process(&key));
    }

    #[tokio::test]
    async fn test_dedup_from_event() {
        let dedup = MessageDeduplicator::with_defaults();

        let event = crate::sync::events::ClusterEvent::ChatMessage {
            room_id: RoomId::from_string("room1".to_string()),
            user_id: UserId::from_string("user1".to_string()),
            username: "test".to_string(),
            message: "Hello".to_string(),
            timestamp: Utc::now(),
            position: None,
            color: None,
        };

        let key = DedupKey::from_event(&event);

        assert!(dedup.should_process(&key));
        assert!(!dedup.should_process(&key));
    }
}
