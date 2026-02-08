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

/// Deduplication key for events
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct DedupKey {
    pub event_type: String,
    pub room_id: String,
    pub user_id: String,
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
}

impl MessageDeduplicator {
    /// Create a new deduplicator
    ///
    /// # Arguments
    /// * `dedup_window` - How long to remember events (default 5 seconds)
    /// * `cleanup_interval` - How often to clean expired entries (default 30 seconds)
    #[must_use] 
    pub fn new(dedup_window: Duration, cleanup_interval: Duration) -> Self {
        let dedup = Self {
            entries: Arc::new(DashMap::new()),
            dedup_window,
            cleanup_interval,
        };

        // Start cleanup task
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
    #[must_use] 
    pub fn should_process(&self, key: &DedupKey) -> bool {
        let now = Instant::now();

        // Check if key exists and hasn't expired
        if let Some(entry) = self.entries.get(key) {
            if entry.expires_at > now {
                // Within dedup window, skip
                return false;
            }
            // Expired, remove and process
            self.entries.remove(key);
            return true;
        }

        // Key doesn't exist, add it
        self.entries.insert(key.clone(), DedupEntry {
            expires_at: now + self.dedup_window,
        });

        true
    }

    /// Mark an event as processed
    pub fn mark_processed(&self, key: DedupKey) {
        let now = Instant::now();
        self.entries.insert(key, DedupEntry {
            expires_at: now + self.dedup_window,
        });
    }

    /// Run periodic cleanup of expired entries
    async fn run_cleanup(&self) {
        let mut interval = tokio::time::interval(self.cleanup_interval);
        loop {
            interval.tick().await;
            self.cleanup_expired();
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use synctv_core::models::id::{RoomId, UserId};

    #[test]
    fn test_dedup_basic() {
        let dedup = MessageDeduplicator::with_defaults();

        let key = DedupKey {
            event_type: "chat".to_string(),
            room_id: "room1".to_string(),
            user_id: "user1".to_string(),
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

    #[test]
    fn test_dedup_from_event() {
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
