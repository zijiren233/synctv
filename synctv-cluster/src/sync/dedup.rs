//! Message deduplication for cross-cluster synchronization
//!
//! Prevents duplicate processing of events when:
//! - Multiple Redis subscribers exist
//! - Network issues cause retries
//! - Events are published multiple times
//!
//! Uses `moka::sync::Cache` with TTL-based expiration, eliminating the need
//! for a manual cleanup task.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Deduplication key for events
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct DedupKey {
    pub event_type: String,
    pub room_id: String,
    pub user_id: String,
    /// Extra discriminator for events without `room_id/user_id` (e.g. `SystemNotification` message)
    pub extra: String,
    pub timestamp_ms: i64,
    /// Content hash to prevent false positives on same-millisecond events
    /// with different payloads (e.g. two chat messages in the same ms)
    pub content_hash: u64,
}

impl DedupKey {
    /// Create a deduplication key from a cluster event.
    ///
    /// Uses the event's unique `event_id` (nanoid) as the primary dedup key
    /// when available, falling back to content hashing for legacy events
    /// without an `event_id`.
    #[must_use]
    pub fn from_event(event: &crate::sync::events::ClusterEvent) -> Self {
        let eid = event.event_id();
        // If event_id is present and non-empty, use it as the sole differentiator
        // to avoid hash collisions entirely.
        let content_hash = if eid.is_empty() {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            if let Ok(json) = serde_json::to_string(event) {
                json.hash(&mut hasher);
            }
            hasher.finish()
        } else {
            0
        };
        Self {
            event_type: event.event_type().to_string(),
            room_id: event.room_id().map_or_else(|| "global".to_string(), |id| id.as_str().to_string()),
            user_id: if eid.is_empty() {
                event.user_id().map_or_else(|| "system".to_string(), |id| id.as_str().to_string())
            } else {
                // When event_id is present, embed it in the user_id field
                // so each event gets a distinct key
                String::new()
            },
            extra: if eid.is_empty() {
                event.dedup_extra()
            } else {
                eid.to_string()
            },
            timestamp_ms: event.timestamp().timestamp_millis(),
            content_hash,
        }
    }
}

/// Message deduplicator using moka TTL cache.
///
/// Entries are automatically evicted after `dedup_window` via moka's built-in
/// TTL support, eliminating the need for a manual cleanup task.
#[derive(Clone)]
pub struct MessageDeduplicator {
    /// Cache of dedup keys with TTL-based expiration
    cache: moka::sync::Cache<DedupKey, ()>,
}

impl MessageDeduplicator {
    /// Create a new deduplicator
    ///
    /// # Arguments
    /// * `dedup_window` - How long to remember events (default 5 seconds)
    /// * `_cleanup_interval` - Ignored (moka handles cleanup internally), kept for API compatibility
    #[must_use]
    pub fn new(dedup_window: Duration, _cleanup_interval: Duration) -> Self {
        let cache = moka::sync::Cache::builder()
            .time_to_live(dedup_window)
            .build();
        Self { cache }
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
    /// Returns `true` if this is a new event, `false` if it's a duplicate
    /// within the dedup window.
    #[must_use]
    pub fn should_process(&self, key: &DedupKey) -> bool {
        // Try to get: if present, it's a duplicate
        if self.cache.get(key).is_some() {
            return false;
        }
        // Insert the key; moka handles TTL expiration automatically
        self.cache.insert(key.clone(), ());
        true
    }

    /// Mark an event as processed
    pub fn mark_processed(&self, key: DedupKey) {
        self.cache.insert(key, ());
    }

    /// Shutdown the deduplicator (no-op with moka, kept for API compatibility)
    pub fn shutdown(&self) {
        self.cache.invalidate_all();
    }

    /// Get the number of tracked events
    #[must_use]
    pub fn len(&self) -> usize {
        // Run pending tasks to get accurate count
        self.cache.run_pending_tasks();
        self.cache.entry_count() as usize
    }

    /// Check if there are any tracked events
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all tracked events (for testing)
    pub fn clear(&self) {
        self.cache.invalidate_all();
        self.cache.run_pending_tasks();
    }
}

impl Default for MessageDeduplicator {
    fn default() -> Self {
        Self::with_defaults()
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
            content_hash: 0,
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
            event_id: nanoid::nanoid!(16),
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
