//! Global setting variables
//!
//! This module defines all setting variables used throughout the application.
//! Each variable is type-safe, thread-safe, and automatically persists to the database.
//!
//! # Usage
//!
//! ```rust,no_run
//! use synctv_core::service::global_settings::*;
//!
//! // Initialize during app startup
//! let registry = SettingsRegistry::new(settings_service);
//! registry.init().await.unwrap();
//!
//! // Read - type-safe, returns cached value
//! if registry.signup_enabled.get().unwrap() {
//!     // Signup is enabled
//! }
//!
//! // Write - auto-converts to string and persists
//! registry.signup_enabled.set(false).await?;
//!
//! // Validate user input via storage
//! if registry.storage.validate("server.signup_enabled", "true") {
//!     // Value is valid
//! }
//! ```

use std::sync::Arc;
use crate::service::{SettingsService, settings_vars::{Setting, SettingsStorage}};
use crate::setting;

/// Settings registry for runtime initialization
///
/// Use this to initialize and manage all settings during app startup
#[derive(Clone)]
pub struct SettingsRegistry {
    /// Storage for managing all settings
    pub storage: Arc<SettingsStorage>,

    // Server settings
    pub signup_enabled: Setting<bool>,
    pub allow_room_creation: Setting<bool>,
    pub max_rooms_per_user: Setting<i64>,
    pub max_members_per_room: Setting<i64>,

    // Permission settings - global defaults for each role
    pub admin_default_permissions: Setting<i64>,
    pub member_default_permissions: Setting<i64>,
    pub guest_default_permissions: Setting<i64>,
}

impl std::fmt::Debug for SettingsRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsRegistry")
            .finish()
    }
}

impl SettingsRegistry {
    /// Create a new settings registry with all setting instances
    pub fn new(settings_service: Arc<SettingsService>) -> Self {
        let storage = Arc::new(SettingsStorage::new(settings_service));

        Self {
            storage: storage.clone(),

            // Server settings using the setting! macro
            // Each setting auto-registers its provider to storage
            signup_enabled: setting!(bool, "server.signup_enabled", storage.clone(), true),
            allow_room_creation: setting!(bool, "server.allow_room_creation", storage.clone(), true),
            max_rooms_per_user: setting!(i64, "server.max_rooms_per_user", storage.clone(), 10,
                |v: &i64| -> anyhow::Result<()> {
                    if *v > 0 && *v <= 1000 {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("max_rooms_per_user must be between 1 and 1000"))
                    }
                }
            ),
            max_members_per_room: setting!(i64, "server.max_members_per_room", storage.clone(), 100,
                |v: &i64| -> anyhow::Result<()> {
                    if *v > 0 && *v <= 10000 {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("max_members_per_room must be between 1 and 10000"))
                    }
                }
            ),

            // Permission settings - global defaults for each role
            // These are base permissions that rooms can override with added/removed permissions
            // Admin default: All permissions except System::ADMIN (1073741823 = 0x3FFFFFFF)
            admin_default_permissions: setting!(i64, "permissions.admin_default", storage.clone(), 1073741823),
            // Member default: Basic member permissions (262143 = 0x3FFFF)
            member_default_permissions: setting!(i64, "permissions.member_default", storage.clone(), 262143),
            // Guest default: Read-only permissions (511 = 0x1FF)
            guest_default_permissions: setting!(i64, "permissions.guest_default", storage, 511),
        }
    }

    /// Initialize storage from database
    pub async fn init(&self) -> anyhow::Result<()> {
        // Load raw values from database into shared storage
        // Individual settings will lazy-load on first get()
        self.storage.init().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_compile() {
        // Just verify the types compile
        assert!(true);
    }
}
