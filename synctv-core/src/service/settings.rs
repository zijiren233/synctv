//! System settings service for runtime configuration management
//!
//! Provides methods for managing settings groups with change notifications
//! Uses `PostgreSQL` LISTEN/NOTIFY for hot reload across multiple replicas
//!
//! Design reference: /Volumes/workspace/rust/synctv-rs-design/19-配置管理系统.md §6.3

use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn, error};
use sqlx::PgPool;

use crate::models::settings::{get_default_settings, SettingsGroup};
use crate::repository::SettingsRepository;
use crate::Error;

/// Change listener callback type
pub type SettingsChangeListener = Arc<dyn Fn(&str, &serde_json::Value) + Send + Sync>;

/// System settings service
#[derive(Clone)]
pub struct SettingsService {
    repository: SettingsRepository,
    pool: PgPool,
    // M-02: Lock-free cache using DashMap for concurrent reads
    cache: Arc<DashMap<String, SettingsGroup>>,
    // Change listeners
    listeners: Arc<RwLock<Vec<SettingsChangeListener>>>,
}

impl std::fmt::Debug for SettingsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Note: We can't show cache_size here without blocking on async
        f.debug_struct("SettingsService")
            .field("repository", &std::any::type_name::<SettingsRepository>())
            .finish_non_exhaustive()
    }
}

impl SettingsService {
    #[must_use]
    pub fn new(repository: SettingsRepository, pool: PgPool) -> Self {
        Self {
            repository,
            pool,
            cache: Arc::new(DashMap::new()),
            listeners: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Initialize the service by loading all settings into cache
    pub async fn initialize(&self) -> Result<(), Error> {
        info!("Initializing settings service");

        let settings = self.repository.get_all().await.map_err(|e| {
            Error::Internal(format!("Failed to load settings: {e}"))
        })?;

        self.cache.clear();

        for setting in settings {
            debug!(
                "Loaded setting '{}.{}' = '{}'",
                setting.group, setting.key, setting.value
            );
            self.cache.insert(setting.key.clone(), setting);
        }

        info!(
            "Settings service initialized with {} settings",
            self.cache.len()
        );
        Ok(())
    }

    /// Get all settings groups
    pub async fn get_all(&self) -> Result<Vec<SettingsGroup>, Error> {
        let mut groups: Vec<_> = self.cache.iter().map(|entry| entry.value().clone()).collect();
        groups.sort_by(|a, b| a.group.cmp(&b.group));
        Ok(groups)
    }

    /// Get all settings as flat key-value pairs
    pub async fn get_all_values(&self) -> Result<std::collections::HashMap<String, String>, Error> {
        let settings = self.get_all().await?;
        let mut result = std::collections::HashMap::new();

        for setting in settings {
            result.insert(setting.key.clone(), setting.value.clone());
        }

        Ok(result)
    }

    /// Get a specific setting by key
    pub async fn get(&self, key: &str) -> Result<SettingsGroup, Error> {
        // Try cache first (lock-free read via DashMap)
        if let Some(setting) = self.cache.get(key) {
            return Ok(setting.value().clone());
        }

        // Not in cache, load from database
        debug!(
            "Setting '{}' not in cache, loading from database",
            key
        );

        let setting = self
            .repository
            .get(key)
            .await
            .map_err(|e| Error::Internal(format!("Failed to get setting: {e}")))?;

        // Update cache
        self.cache.insert(setting.key.clone(), setting.clone());

        Ok(setting)
    }

    /// Update a setting value by key
    pub async fn update(
        &self,
        key: &str,
        value: String,
    ) -> Result<SettingsGroup, Error> {
        debug!("Updating setting '{}'", key);

        // Update in database
        let setting = self
            .repository
            .update(key, &value)
            .await
            .map_err(|e| Error::Internal(format!("Failed to update setting: {e}")))?;

        // Update cache
        self.cache.insert(setting.key.clone(), setting.clone());

        // Notify listeners
        let json_value: serde_json::Value = value.parse().unwrap_or_else(|_| serde_json::json!(value));
        self.notify_listeners(key, &json_value).await;

        info!("Updated setting '{}'", setting.key);
        Ok(setting)
    }


    /// Get a specific setting value by key (e.g., "`server.allow_registration`")
    pub async fn get_value(&self, key: &str) -> Option<String> {
        let setting = self.get(key).await.ok()?;
        Some(setting.value)
    }


    /// Register a change listener
    pub async fn register_listener(&self, listener: SettingsChangeListener) {
        let mut listeners = self.listeners.write().await;
        listeners.push(listener);
        debug!("Registered settings change listener, total: {}", listeners.len());
    }

    /// Notify all listeners of a settings change
    async fn notify_listeners(&self, group: &str, settings_json: &serde_json::Value) {
        let listeners = self.listeners.read().await;
        if listeners.is_empty() {
            return;
        }

        debug!(
            "Notifying {} listeners of settings change in group '{}'",
            listeners.len(),
            group
        );

        for listener in listeners.iter() {
            listener(group, settings_json);
        }
    }

    /// Start `PostgreSQL` LISTEN task for hot reload
    ///
    /// Listens for '`settings_changed`' notifications and automatically reloads
    /// changed settings from database into cache.
    ///
    /// This enables hot reload across multiple replicas without restart.
    ///
    /// # Returns
    /// A `JoinHandle` for the background task
    ///
    /// # Example
    /// ```ignore
    /// let settings_service = SettingsService::new(repo, pool);
    /// settings_service.initialize().await?;
    /// let cancel = tokio_util::sync::CancellationToken::new();
    /// let _listen_task = settings_service.start_listen_task(cancel);
    /// ```
    #[must_use]
    pub fn start_listen_task(&self, cancel: CancellationToken) -> tokio::task::JoinHandle<()> {
        let service = self.clone();
        let pool = self.pool.clone();

        tokio::spawn(async move {
            info!("Starting PostgreSQL LISTEN for settings hot reload");

            loop {
                if cancel.is_cancelled() {
                    info!("Settings listen task cancelled, shutting down");
                    return;
                }

                // Create listener connection
                let mut listener = match sqlx::postgres::PgListener::connect_with(&pool).await {
                    Ok(listener) => listener,
                    Err(e) => {
                        error!("Failed to create PgListener: {}", e);
                        tokio::select! {
                            () = tokio::time::sleep(tokio::time::Duration::from_secs(5)) => {}
                            () = cancel.cancelled() => {
                                info!("Settings listen task cancelled during reconnect backoff");
                                return;
                            }
                        }
                        continue;
                    }
                };

                // Listen to 'settings_changed' channel
                if let Err(e) = listener.listen("settings_changed").await {
                    error!("Failed to LISTEN on settings_changed: {}", e);
                    tokio::select! {
                        () = tokio::time::sleep(tokio::time::Duration::from_secs(5)) => {}
                        () = cancel.cancelled() => {
                            info!("Settings listen task cancelled during listen backoff");
                            return;
                        }
                    }
                    continue;
                }

                info!("PostgreSQL LISTEN started for settings_changed channel");

                // Process notifications using blocking recv with cancellation
                loop {
                    tokio::select! {
                        result = listener.recv() => {
                            match result {
                                Ok(notification) => {
                                    let changed_key = notification.payload();
                                    info!("Received settings change notification: {}", changed_key);

                                    // Reload the changed setting from database
                                    match service.reload_setting(changed_key).await {
                                        Ok(()) => {
                                            debug!("Successfully reloaded setting: {}", changed_key);
                                        }
                                        Err(e) => {
                                            warn!(
                                                "Failed to reload setting '{}': {}",
                                                changed_key, e
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("Error receiving notification: {}", e);
                                    // Connection lost, break inner loop to reconnect
                                    break;
                                }
                            }
                        }
                        () = cancel.cancelled() => {
                            info!("Settings listen task cancelled");
                            return;
                        }
                    }
                }

                warn!("PostgreSQL LISTEN connection lost, reconnecting in 5 seconds...");
                tokio::select! {
                    () = tokio::time::sleep(tokio::time::Duration::from_secs(5)) => {}
                    () = cancel.cancelled() => {
                        info!("Settings listen task cancelled during reconnect backoff");
                        return;
                    }
                }

                // Refresh cache after reconnection to catch missed notifications
                if let Err(e) = service.initialize().await {
                    error!("Failed to refresh settings cache after reconnection: {}", e);
                }
            }
        })
    }

    /// Reload a specific setting from database into cache
    ///
    /// Called when a `PostgreSQL` NOTIFY is received
    async fn reload_setting(&self, key: &str) -> Result<(), Error> {
        debug!("Reloading setting from database: {}", key);

        // Try to fetch from database
        match self.repository.get(key).await {
            Ok(setting) => {
                // Update cache (lock-free via DashMap)
                self.cache.insert(setting.key.clone(), setting.clone());

                // Notify local listeners
                let json_value: serde_json::Value = setting.value.parse()
                    .unwrap_or_else(|_| serde_json::json!(setting.value));
                self.notify_listeners(key, &json_value).await;

                info!("Setting '{}' reloaded from database", key);
                Ok(())
            }
            Err(e) => {
                // Setting was deleted, remove from cache
                warn!(
                    "Setting '{}' not found in database (may have been deleted): {}",
                    key, e
                );
                self.cache.remove(key);

                // Notify listeners about removal
                self.notify_listeners(key, &serde_json::json!(null)).await;

                Ok(())
            }
        }
    }

}

/// Helper to get default settings for a group
#[must_use] 
pub fn get_default_settings_json(group: &str) -> Option<serde_json::Value> {
    get_default_settings(group)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_default_values() {
        // These tests verify the helper functions work
        let allow_reg = get_default_settings_json("server")
            .and_then(|v| v.get("allow_registration").cloned())
            .and_then(|v| v.as_bool());

        assert_eq!(allow_reg, Some(true));
    }
}
