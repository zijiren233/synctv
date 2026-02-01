//! Type-safe settings variables with automatic database persistence
//!
//! # Design
//!
//! - All settings share a single `Arc<RwLock<HashMap<String, String>>` for raw values
//! - Each setting has its own typed cache
//! - Type conversion via standard Rust traits (Display, FromStr)
//! - Reading returns cached value (synchronous, fast)
//! - Writing saves to storage + database (async)
//!
//! # Custom Validation
//!
//! Types can implement the `Validate` trait to provide custom validation logic:
//!
//! ```rust,no_run
//! use synctv_core::service::settings_vars::Validate;
//! use anyhow::Result;
//!
//! #[derive(Debug, Clone, PartialEq)]
//! struct MaxRooms(i64);
//!
//! impl Validate for MaxRooms {
//!     fn validate(&self) -> Result<()> {
//!         if self.0 > 0 && self.0 <= 1000 {
//!             Ok(())
//!         } else {
//!             Err(anyhow::anyhow!("max_rooms must be between 1 and 1000"))
//!         }
//!     }
//! }
//! ```

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fmt::Display;
use std::hash::BuildHasherDefault;
use std::sync::{Arc, RwLock};

use super::SettingsService;
use anyhow::Result;

/// Trait for custom validation logic
///
/// Types can implement this trait to provide custom validation logic.
/// By default, all types pass validation.
pub trait Validate: Send + Sync {
    /// Validate the value
    fn validate(&self) -> Result<()> {
        Ok(())
    }
}

// Implement Validate for all types by default (blanket implementation)
impl<T: Send + Sync> Validate for T {}

/// Trait for setting operations (type-erased)
///
/// This trait provides a unified interface for working with a single setting
pub trait SettingProvider: Send + Sync {
    /// Get raw string value
    fn get_raw(&self) -> Option<String>;

    /// Set raw string value
    fn set_raw(&self, value: String) -> Result<()>;

    /// Validate a raw string value
    fn is_valid_raw(&self, value: &str) -> Result<()>;
}

/// Macro to create a Setting with any type
///
/// # Example
///
/// ```rust,no_run
/// let signup_enabled = setting!(bool, "server.signup_enabled", storage, true);
/// let max_rooms = setting!(i64, "server.max_rooms", storage, 10);
/// ```
#[macro_export]
macro_rules! setting {
    ($type:ty, $key:expr, $storage:expr, $default:expr) => {
        $crate::service::settings_vars::Setting::new($key, $storage, $default)
    };
}

/// Raw settings storage - shared across all settings
#[derive(Clone)]
pub struct SettingsStorage {
    inner: Arc<RwLock<HashMap<String, String, BuildHasherDefault<DefaultHasher>>>>,
    settings_service: Arc<SettingsService>,
    setting_providers: Arc<RwLock<HashMap<String, Arc<dyn SettingProvider>>>>,
}

impl SettingsStorage {
    pub fn new(settings_service: Arc<SettingsService>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::default())),
            settings_service,
            setting_providers: Arc::new(RwLock::new(HashMap::default())),
        }
    }

    /// Register a setting provider for a key
    fn register_provider(&self, key: &'static str, provider: Arc<dyn SettingProvider>) {
        if let Ok(mut providers) = self.setting_providers.write() {
            providers.insert(key.to_string(), provider);
        }
    }

    /// Get a provider by key
    pub fn get_provider(&self, key: &str) -> Option<Arc<dyn SettingProvider>> {
        self.setting_providers.read().ok()?.get(key).cloned()
    }

    /// Initialize all settings from database
    pub async fn init(&self) -> Result<()> {
        // Load all settings as flat key-value pairs
        let all_values = self
            .settings_service
            .get_all_values()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load settings: {}", e))?;

        let mut storage = self
            .inner
            .write()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        *storage = all_values.into_iter().collect();

        Ok(())
    }

    /// Get raw string value for a key
    pub fn get_raw(&self, key: &str) -> Option<String> {
        let storage = self.inner.read().ok()?;
        storage.get(key).cloned()
    }

    /// Set raw string value for a key
    pub fn set_raw(&self, key: &str, value: String) -> Result<()> {
        // Update in-memory storage
        {
            let mut storage = self
                .inner
                .write()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            storage.insert(key.to_string(), value.clone());
        }

        // Spawn a task to update in database using the full key
        let settings_service = self.settings_service.clone();
        let key = key.to_string();
        tokio::spawn(async move {
            let _ = settings_service.update(&key, value).await;
        });

        Ok(())
    }

    /// Validate a setting value by key
    pub fn validate(&self, key: &str, value: &str) -> bool {
        self.get_provider(key)
            .map(|p| p.is_valid_raw(value).is_ok())
            .unwrap_or(true)
    }
}

/// Type-safe setting variable with lazy loading
///
/// Generic over any type that implements:
/// - `Clone` - for copying values
/// - `Display` - for formatting to string (via to_string())
/// - `std::str::FromStr` - for parsing from string
pub struct Setting<T>
where
    T: Clone + Display + std::str::FromStr + Send + Sync + 'static,
    <T as std::str::FromStr>::Err: std::error::Error + Send + Sync,
{
    key: &'static str,
    storage: Arc<SettingsStorage>,
    cache: Arc<RwLock<Option<T>>>,
    raw_cache: Arc<RwLock<Option<String>>>,
    default_value: T,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> Clone for Setting<T>
where
    T: Clone + Display + std::str::FromStr + Send + Sync + 'static,
    <T as std::str::FromStr>::Err: std::error::Error + Send + Sync,
{
    fn clone(&self) -> Self {
        Self {
            key: self.key,
            storage: self.storage.clone(),
            cache: self.cache.clone(),
            raw_cache: self.raw_cache.clone(),
            default_value: self.default_value.clone(),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T> Setting<T>
where
    T: Clone + Display + std::str::FromStr + Send + Sync + 'static,
    <T as std::str::FromStr>::Err: std::error::Error + Send + Sync,
{
    /// Create a new setting variable
    ///
    /// # Arguments
    ///
    /// * `key` - Setting key in format "group.name" (e.g., "server.signup_enabled")
    /// * `storage` - Shared settings storage
    /// * `default_value` - Default value if setting doesn't exist
    pub fn new(key: &'static str, storage: Arc<SettingsStorage>, default_value: T) -> Self {
        let setting = Self {
            key,
            storage: storage.clone(),
            cache: Arc::new(RwLock::new(None)),
            raw_cache: Arc::new(RwLock::new(None)),
            default_value,
            _phantom: std::marker::PhantomData,
        };

        // Auto-register provider
        storage.register_provider(key, Arc::new(setting.clone()));

        setting
    }

    /// Get the current value, checking for changes on every call
    pub fn get(&self) -> Result<T> {
        // Always fetch the latest raw value from storage
        let new_raw = self.storage.get_raw(self.key);

        // Check if we need to update cache
        let needs_update = {
            let raw_cache = self
                .raw_cache
                .read()
                .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
            match (&*raw_cache, &new_raw) {
                (Some(cached), Some(new)) => cached != new,
                (None, None) => false,
                _ => true, // One is None, one is Some
            }
        };

        if needs_update {
            // Raw value changed (or first load), re-parse
            let value = new_raw
                .as_ref()
                .map(|raw| raw.parse().unwrap_or_else(|_| self.default_value.clone()))
                .unwrap_or_else(|| self.default_value.clone());

            // Update both caches
            {
                let mut cache = self
                    .cache
                    .write()
                    .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
                *cache = Some(value.clone());
            }
            {
                let mut raw_cache = self
                    .raw_cache
                    .write()
                    .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
                *raw_cache = new_raw;
            }

            Ok(value)
        } else {
            // Raw value unchanged, return cached value
            let cache = self
                .cache
                .read()
                .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
            (*cache)
                .as_ref()
                .map_or_else(|| Ok(self.default_value.clone()), |value| Ok(value.clone()))
        }
    }

    /// Set a new value and persist to database
    pub fn set(&self, value: T) -> Result<()> {
        // Validate before setting
        value.validate()?;
        // Convert to string using standard Display trait
        let str_value = value.to_string();
        self.storage.set_raw(self.key, str_value)?;
        Ok(())
    }

    /// Validate a raw string value (for API input validation)
    pub fn is_valid_raw(&self, str_value: &str) -> Result<()> {
        let v = str_value
            .parse::<T>()
            .map_err(|_| anyhow::anyhow!("Invalid value for setting '{}'", self.key))?;
        v.validate()
    }
}

impl<T> SettingProvider for Setting<T>
where
    T: Clone + Display + std::str::FromStr + Send + Sync + 'static,
    <T as std::str::FromStr>::Err: std::error::Error + Send + Sync,
{
    fn get_raw(&self) -> Option<String> {
        self.storage.get_raw(self.key)
    }

    fn set_raw(&self, value: String) -> Result<()> {
        // Validate before setting
        self.is_valid_raw(&value)?;
        self.storage.set_raw(self.key, value)
    }

    fn is_valid_raw(&self, value: &str) -> Result<()> {
        let v = value.parse::<T>()?;
        v.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Example: Custom i64 wrapper that requires values > 100
    #[derive(Debug, Clone, PartialEq, Eq, Display, std::str::FromStr)]
    struct MinI100(i64);

    impl Validate for MinI100 {
        fn validate(&self) -> Result<()> {
            if self.0 > 100 {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "Value must be greater than 100, got {}",
                    self.0
                ))
            }
        }
    }

    #[test]
    fn test_custom_validation() {
        let valid_value = MinI100(150);
        assert!(valid_value.validate().is_ok());

        let invalid_value = MinI100(50);
        assert!(invalid_value.validate().is_err());
    }

    #[test]
    fn test_bool_conversion() {
        assert!("true".parse::<bool>().unwrap());
        assert!(!"false".parse::<bool>().unwrap());
        assert_eq!(true.to_string(), "true");
        assert_eq!(false.to_string(), "false");
    }

    #[test]
    fn test_i64_conversion() {
        assert_eq!("42".parse::<i64>().unwrap(), 42);
        assert_eq!(42.to_string(), "42");
    }

    #[test]
    fn test_string_conversion() {
        assert_eq!("hello".parse::<String>().unwrap(), "hello");
        assert_eq!("world".to_string(), "world");
    }

    #[test]
    fn test_invalid_bool_parse() {
        // Valid bool values
        assert!("true".parse::<bool>().is_ok());
        assert!("false".parse::<bool>().is_ok());

        // Invalid bool values
        assert!("invalid".parse::<bool>().is_err());
        assert!("1".parse::<bool>().is_err()); // FromStr is strict for bool
    }

    #[test]
    fn test_invalid_i64_parse() {
        // Valid i64 values
        assert!("42".parse::<i64>().is_ok());
        assert!("-100".parse::<i64>().is_ok());

        // Invalid i64 values
        assert!("abc".parse::<i64>().is_err());
        assert!("12.34".parse::<i64>().is_err());
    }
}
