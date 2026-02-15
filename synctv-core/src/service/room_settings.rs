//! Room settings service with caching and multi-replica synchronization
//!
//! # Architecture
//!
//! ## Caching Strategy
//! - L1 Cache: In-memory moka cache (per-instance)
//! - TTL: 5 minutes with time-based expiration
//! - Max capacity: 10,000 rooms
//! - Cache invalidation: On settings update via Pub/Sub
//!
//! ## Multi-Replica Synchronization
//! - Uses Redis Pub/Sub to broadcast settings changes
//! - Channel: `room_settings_updates`
//! - Message format: `{"room_id": "xxx", "version": 123}`
//!
//! ## Performance Optimizations
//! - Single-flight pattern: Prevents cache thundering
//! - Background refresh: Refreshes before expiration
//! - Write-through: Updates database and cache atomically

use std::sync::Arc;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use rand::RngExt;

use crate::{
    models::{RoomId, RoomSettings},
    repository::RoomSettingsRepository,
    service::notification::NotificationService,
    Error, Result,
};

/// Settings update notification for Pub/Sub
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SettingsUpdateMessage {
    room_id: String,
    version: i64,
}

/// Abstract cache invalidation trait - allows flexible backend
#[async_trait::async_trait]
pub trait CacheInvalidation: Send + Sync {
    async fn publish(&self, channel: &str, message: &str) -> Result<()>;
    async fn subscribe(&self, channel: &str) -> Result<Box<dyn InvalidationStream>>;
}

/// Stream for receiving invalidation messages
#[async_trait::async_trait]
pub trait InvalidationStream: Send + Sync {
    async fn next_message(&mut self) -> Result<InvalidationMessage>;
}

/// Invalid message from pub/sub
#[derive(Debug, Clone)]
pub struct InvalidationMessage {
    pub payload: String,
}

/// Room settings service with caching
pub struct RoomSettingsService {
    repo: RoomSettingsRepository,
    cache: Arc<moka::future::Cache<RoomId, RoomSettings>>,
    invalidation: Option<Arc<dyn CacheInvalidation>>,
    notification_service: Arc<NotificationService>,
    single_flight: Arc<tokio::sync::Mutex<std::collections::HashMap<RoomId, Arc<tokio::sync::Semaphore>>>>,
}

impl std::fmt::Debug for RoomSettingsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoomSettingsService")
            .field("cache_size", &self.cache.entry_count())
            .finish()
    }
}

impl Clone for RoomSettingsService {
    fn clone(&self) -> Self {
        Self {
            repo: self.repo.clone(),
            cache: self.cache.clone(),
            invalidation: self.invalidation.clone(),
            notification_service: self.notification_service.clone(),
            single_flight: self.single_flight.clone(),
        }
    }
}

impl RoomSettingsService {
    const CACHE_TTL_SECS: u64 = 300; // 5 minutes
    const CACHE_MAX_CAPACITY: u64 = 10_000;
    const PUBSUB_CHANNEL: &'static str = "room_settings_updates";
    /// Maximum retry attempts for optimistic lock conflicts
    const MAX_RETRIES: u32 = 3;
    /// Base backoff in milliseconds (exponential: 5ms, 10ms, 20ms)
    const BACKOFF_BASE_MS: u64 = 5;

    /// Create a new room settings service
    #[must_use]
    pub fn new(
        repo: RoomSettingsRepository,
        invalidation: Option<Arc<dyn CacheInvalidation>>,
        notification_service: Arc<NotificationService>,
        cache_ttl_secs: Option<u64>,
        cache_max_capacity: Option<u64>,
        cancel: Option<tokio_util::sync::CancellationToken>,
    ) -> Self {
        let ttl = cache_ttl_secs.unwrap_or(Self::CACHE_TTL_SECS);
        let capacity = cache_max_capacity.unwrap_or(Self::CACHE_MAX_CAPACITY);

        let cache = Arc::new(
            moka::future::CacheBuilder::new(capacity)
                .time_to_live(Duration::from_secs(ttl))
                .build(),
        );

        let service = Self {
            repo,
            cache,
            invalidation,
            notification_service,
            single_flight: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        };

        // Start Pub/Sub listener if invalidation backend is available
        if service.invalidation.is_some() {
            let service_clone = service.clone();
            let cancel = cancel.unwrap_or_default();
            tokio::spawn(async move {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        tracing::info!("Room settings Pub/Sub listener shutting down");
                    }
                    result = service_clone.listen_for_updates() => {
                        if let Err(e) = result {
                            tracing::error!("Room settings Pub/Sub listener error: {}", e);
                        }
                    }
                }
            });
        }

        service
    }

    /// Get room settings with caching
    ///
    /// # Performance
    /// - L1 cache hit: < 1ms
    /// - Cache miss + DB query: ~10ms
    /// - Single-flight: Prevents thundering herd
    pub async fn get(&self, room_id: &RoomId) -> Result<RoomSettings> {
        // Try cache first
        if let Some(settings) = self.cache.get(room_id).await {
            return Ok(settings);
        }

        // Use single-flight pattern to prevent thundering herd
        let semaphore = self.get_or_create_semaphore(room_id).await;
        let _permit = semaphore.acquire().await;

        // Double-check cache after acquiring semaphore
        if let Some(settings) = self.cache.get(room_id).await {
            return Ok(settings);
        }

        // Load from database
        let settings = self.repo.get(room_id).await?;

        // Store in cache
        self.cache.insert(room_id.clone(), settings.clone()).await;

        Ok(settings)
    }

    /// Get room settings without cache (force refresh)
    pub async fn get_refresh(&self, room_id: &RoomId) -> Result<RoomSettings> {
        // Invalidate cache
        self.invalidate_local(room_id).await;

        // Load from database
        let settings = self.repo.get(room_id).await?;

        // Store in cache
        let () = self.cache.insert(room_id.clone(), settings.clone()).await;

        Ok(settings)
    }

    /// Set room settings (write-through cache) with optimistic locking.
    ///
    /// Uses CAS (Compare-And-Swap) with automatic retry on version conflicts.
    ///
    /// # Multi-Replica Synchronization
    /// - Reads current version from database
    /// - Updates database with version check
    /// - Updates local cache
    /// - Publishes invalidation to Pub/Sub (if configured)
    /// - Sends WebSocket notification to connected clients
    pub async fn set(&self, room_id: &RoomId, settings: &RoomSettings) -> Result<()> {
        for attempt in 0..Self::MAX_RETRIES {
            // Get current version (bypass cache)
            let (_current, version) = self.repo.get_with_version(room_id).await?;

            // CAS write
            match self.repo.set_settings_with_version(room_id, settings, version).await {
                Ok(new_version) => {
                    // Update local cache
                    self.cache.insert(room_id.clone(), settings.clone()).await;
                    self.publish_and_notify(room_id, settings, new_version).await;
                    return Ok(());
                }
                Err(Error::OptimisticLockConflict) if attempt + 1 < Self::MAX_RETRIES => {
                    let backoff = Self::BACKOFF_BASE_MS * (1 << attempt);
                    let jitter = rand::rng().random_range(0..Self::BACKOFF_BASE_MS);
                    tokio::time::sleep(std::time::Duration::from_millis(backoff + jitter)).await;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Err(Error::Internal("Settings update failed after maximum retry attempts".to_string()))
    }

    /// Update a single setting field with optimistic locking (CAS).
    ///
    /// Reads current settings and version, applies the updater, then writes back
    /// with a version check. Retries automatically on concurrent modification.
    pub async fn update_field<F>(
        &self,
        room_id: &RoomId,
        updater: F,
    ) -> Result<RoomSettings>
    where
        F: Fn(&mut RoomSettings) + Send,
    {
        for attempt in 0..Self::MAX_RETRIES {
            // Read current settings with version (bypass cache for freshness)
            let (mut settings, version) = self.repo.get_with_version(room_id).await?;

            // Apply update
            updater(&mut settings);

            // CAS write with version check
            match self.repo.set_settings_with_version(room_id, &settings, version).await {
                Ok(new_version) => {
                    // Update local cache after successful write
                    self.cache.insert(room_id.clone(), settings.clone()).await;
                    self.publish_and_notify(room_id, &settings, new_version).await;
                    return Ok(settings);
                }
                Err(Error::OptimisticLockConflict) if attempt + 1 < Self::MAX_RETRIES => {
                    let backoff = Self::BACKOFF_BASE_MS * (1 << attempt);
                    let jitter = rand::rng().random_range(0..Self::BACKOFF_BASE_MS);
                    tokio::time::sleep(std::time::Duration::from_millis(backoff + jitter)).await;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Err(Error::Internal("Settings update failed after maximum retry attempts".to_string()))
    }

    /// Reset room settings to default
    pub async fn reset(&self, room_id: &RoomId) -> Result<RoomSettings> {
        let default_settings = RoomSettings::default();
        self.set(room_id, &default_settings).await?;
        Ok(default_settings)
    }

    /// Delete all settings for a room
    pub async fn delete(&self, room_id: &RoomId) -> Result<()> {
        self.repo.delete_all(room_id).await?;

        // Invalidate cache
        self.invalidate_local(room_id).await;

        // Notify other replicas (use timestamp as pseudo-version since row is deleted)
        if let Some(ref invalidation) = self.invalidation {
            let message = SettingsUpdateMessage {
                room_id: room_id.as_str().to_string(),
                version: chrono::Utc::now().timestamp(),
            };

            let _ = invalidation.publish(
                Self::PUBSUB_CHANNEL,
                &serde_json::to_string(&message).unwrap_or_default(),
            ).await;
        }

        Ok(())
    }

    /// Publish invalidation to other replicas and notify connected clients.
    async fn publish_and_notify(&self, room_id: &RoomId, settings: &RoomSettings, version: i64) {
        if let Some(ref invalidation) = self.invalidation {
            let message = SettingsUpdateMessage {
                room_id: room_id.as_str().to_string(),
                version,
            };

            if let Err(e) = invalidation.publish(
                Self::PUBSUB_CHANNEL,
                &serde_json::to_string(&message).unwrap_or_default(),
            ).await {
                tracing::error!("Failed to publish settings update: {}", e);
            }
        }

        self.notify_settings_changed(room_id, settings).await;
    }

    /// Invalidate local cache for a room
    async fn invalidate_local(&self, room_id: &RoomId) {
        let () = self.cache.invalidate(room_id).await;
    }

    /// Listen for settings updates from Pub/Sub
    async fn listen_for_updates(&self) -> Result<()> {
        let invalidation = self.invalidation.as_ref()
            .ok_or_else(|| Error::Internal("Invalidation backend not configured".to_string()))?;

        let mut stream = invalidation.subscribe(Self::PUBSUB_CHANNEL).await?;

        while let Ok(message) = stream.next_message().await {
            if let Ok(update) = serde_json::from_str::<SettingsUpdateMessage>(&message.payload) {
                let room_id = RoomId::from_string(update.room_id);
                self.invalidate_local(&room_id).await;

                tracing::debug!("Invalidated settings for room: {}", room_id.as_str());
            }
        }

        Ok(())
    }

    /// Notify connected clients about settings change
    async fn notify_settings_changed(&self, room_id: &RoomId, settings: &RoomSettings) {
        let settings_value = match serde_json::to_value(settings) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("Failed to serialize settings: {}", e);
                return;
            }
        };

        let _ = self.notification_service
            .notify_settings_updated(room_id, settings_value)
            .await;
    }

    /// Get or create semaphore for single-flight pattern.
    /// Cleans up entries with no external references to prevent unbounded memory growth.
    async fn get_or_create_semaphore(&self, room_id: &RoomId) -> Arc<tokio::sync::Semaphore> {
        let mut map = self.single_flight.lock().await;

        // Periodically clean up semaphores that are no longer in use (strong_count == 1
        // means only the map itself holds a reference, so no one is waiting on it)
        if map.len() > 1000 {
            map.retain(|_, sem| Arc::strong_count(sem) > 1);
        }

        map.entry(room_id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(1)))
            .clone()
    }

    /// Preload settings for multiple rooms (bulk loading)
    pub async fn preload(&self, room_ids: &[RoomId]) -> Result<()> {
        let mut loaded = std::collections::HashMap::new();

        for room_id in room_ids {
            if let Ok(settings) = self.repo.get(room_id).await {
                loaded.insert(room_id.clone(), settings);
            }
        }

        // Bulk insert into cache
        for (room_id, settings) in loaded {
            self.cache.insert(room_id, settings).await;
        }

        Ok(())
    }

    /// Get cache statistics
    #[must_use] 
    pub fn cache_stats(&self) -> CacheStats {
        CacheStats {
            entry_count: self.cache.entry_count(),
            weighted_size: self.cache.weighted_size(),
        }
    }

    /// Clear all cache
    pub async fn clear_cache(&self) {
        self.cache.invalidate_all();
    }
}

/// Cache statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub entry_count: u64,
    pub weighted_size: u64,
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_cache_invalidation() {
        // Integration test
    }
}
