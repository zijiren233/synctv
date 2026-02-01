//! System settings service for runtime configuration management
//!
//! Provides methods for managing settings groups with change notifications

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::models::settings::{get_default_settings, SettingsGroup};
use crate::repository::SettingsRepository;
use crate::Error;

/// Change listener callback type
pub type SettingsChangeListener = Arc<dyn Fn(&str, &serde_json::Value) + Send + Sync>;

/// System settings service
#[derive(Clone)]
pub struct SettingsService {
    repository: SettingsRepository,
    // In-memory cache for fast reads
    cache: Arc<RwLock<std::collections::HashMap<String, SettingsGroup>>>,
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
    pub fn new(repository: SettingsRepository) -> Self {
        Self {
            repository,
            cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
            listeners: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Initialize the service by loading all settings into cache
    pub async fn initialize(&self) -> Result<(), Error> {
        info!("Initializing settings service");

        let groups = self.repository.get_all().await.map_err(|e| {
            Error::Internal(format!("Failed to load settings: {}", e))
        })?;

        let mut cache = self.cache.write().await;
        cache.clear();

        for group in groups {
            debug!(
                "Loaded settings group '{}' with {} keys",
                group.group_name,
                group.settings_json.as_object().map(|o| o.len()).unwrap_or(0)
            );
            cache.insert(group.group_name.clone(), group);
        }

        info!(
            "Settings service initialized with {} groups",
            cache.len()
        );
        Ok(())
    }

    /// Get all settings groups
    pub async fn get_all(&self) -> Result<Vec<SettingsGroup>, Error> {
        let cache = self.cache.read().await;
        let mut groups: Vec<_> = cache.values().cloned().collect();
        groups.sort_by(|a, b| a.group_name.cmp(&b.group_name));
        Ok(groups)
    }

    /// Get a specific settings group by name
    pub async fn get(&self, group_name: &str) -> Result<SettingsGroup, Error> {
        // Try cache first
        {
            let cache = self.cache.read().await;
            if let Some(group) = cache.get(group_name) {
                return Ok(group.clone());
            }
        }

        // Not in cache, load from database
        debug!(
            "Settings group '{}' not in cache, loading from database",
            group_name
        );

        let group = self
            .repository
            .get_or_create(group_name)
            .await
            .map_err(|e| Error::Internal(format!("Failed to get settings: {}", e)))?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(group_name.to_string(), group.clone());
        }

        Ok(group)
    }

    /// Update a settings group
    pub async fn update(
        &self,
        group_name: &str,
        settings_json: serde_json::Value,
    ) -> Result<SettingsGroup, Error> {
        debug!("Updating settings group '{}'", group_name);

        // Validate JSON is an object
        if !settings_json.is_object() {
            return Err(Error::InvalidInput(
                "Settings must be a JSON object".to_string(),
            ));
        }

        // Update in database
        let group = self
            .repository
            .update(group_name, &settings_json)
            .await
            .map_err(|e| Error::Internal(format!("Failed to update settings: {}", e)))?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(group_name.to_string(), group.clone());
        }

        // Notify listeners
        self.notify_listeners(group_name, &settings_json).await;

        info!("Updated settings group '{}'", group_name);
        Ok(group)
    }

    /// Reset a settings group to defaults
    pub async fn reset_to_defaults(&self, group_name: &str) -> Result<SettingsGroup, Error> {
        info!("Resetting settings group '{}' to defaults", group_name);

        let group = self
            .repository
            .reset_to_defaults(group_name)
            .await
            .map_err(|e| {
                Error::Internal(format!("Failed to reset settings: {}", e))
            })?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(group_name.to_string(), group.clone());
        }

        // Notify listeners
        self.notify_listeners(group_name, &group.settings_json).await;

        info!("Reset settings group '{}' to defaults", group_name);
        Ok(group)
    }

    /// Get a specific setting value by key path (e.g., "server.allow_registration")
    pub async fn get_value(&self, path: &str) -> Option<serde_json::Value> {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.is_empty() {
            return None;
        }

        let group_name = parts[0];
        let group = self.get(group_name).await.ok()?;

        let setting_path = parts[1..].join(".");
        group.get(&setting_path).cloned()
    }

    /// Get a boolean setting value
    pub async fn get_bool(&self, path: &str, default: bool) -> bool {
        self.get_value(path)
            .await
            .and_then(|v| v.as_bool())
            .unwrap_or(default)
    }

    /// Get a string setting value
    pub async fn get_str(&self, path: &str, default: &str) -> String {
        match self.get_value(path).await {
            Some(v) if v.is_string() => v.as_str().unwrap_or(default).to_string(),
            _ => default.to_string(),
        }
    }

    /// Get an integer setting value
    pub async fn get_i64(&self, path: &str, default: i64) -> i64 {
        self.get_value(path)
            .await
            .and_then(|v| v.as_i64())
            .unwrap_or(default)
    }

    /// Register a change listener
    pub async fn register_listener(&self, listener: SettingsChangeListener) {
        let mut listeners = self.listeners.write().await;
        listeners.push(listener);
        debug!("Registered settings change listener, total: {}", listeners.len());
    }

    /// Notify all listeners of a settings change
    async fn notify_listeners(&self, group_name: &str, settings_json: &serde_json::Value) {
        let listeners = self.listeners.read().await;
        if listeners.is_empty() {
            return;
        }

        debug!(
            "Notifying {} listeners of settings change in group '{}'",
            listeners.len(),
            group_name
        );

        for listener in listeners.iter() {
            listener(group_name, settings_json);
        }
    }

    /// Check if registration is allowed
    pub async fn allow_registration(&self) -> bool {
        self.get_bool("server.allow_registration", true).await
    }

    /// Check if room creation is allowed
    pub async fn allow_room_creation(&self) -> bool {
        self.get_bool("server.allow_room_creation", true).await
    }

    /// Get max rooms per user
    pub async fn max_rooms_per_user(&self) -> i64 {
        self.get_i64("server.max_rooms_per_user", 10).await
    }

    /// Get max members per room
    pub async fn max_members_per_room(&self) -> i64 {
        self.get_i64("server.max_members_per_room", 100).await
    }

    /// Check if email is enabled
    pub async fn email_enabled(&self) -> bool {
        self.get_bool("email.enabled", false).await
    }

    /// Check if rate limiting is enabled
    pub async fn rate_limit_enabled(&self) -> bool {
        self.get_bool("rate_limit.enabled", true).await
    }

    /// Get API rate limit
    pub async fn api_rate_limit(&self) -> i64 {
        self.get_i64("rate_limit.api_rate_limit", 100).await
    }

    /// Get API rate window (in seconds)
    pub async fn api_rate_window(&self) -> i64 {
        self.get_i64("rate_limit.api_rate_window", 60).await
    }
}

/// Helper to get default settings for a group
pub fn get_default_settings_json(group_name: &str) -> Option<serde_json::Value> {
    get_default_settings(group_name)
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
