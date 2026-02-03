//! Type-safe room settings with automatic lazy_static registration
//!
//! # Architecture
//!
//! Each room setting is an **independent type** that implements `RoomSetting` trait.
//! The `room_setting!` macro generates the type with **lazy_static! auto-registration**.
//!
//! # Examples
//!
//! ```rust,ignore
//! // Define a setting - auto-registers on first use!
//! room_setting!(ChatEnabled, bool, "chat_enabled", true);
//!
//! // Use by type (compile-time safe)
//! let setting = ChatEnabled(true);
//! // Registration happens automatically on first Default::default() call!
//!
//! // Or use via registry
//! RoomSettingsRegistry::has_key("chat_enabled");  // auto-registers
//! ```
//!
//! # Auto-Registration with lazy_static!
//!
//! Each type has a **lazy_static!** block in the macro that:
//! - Runs once on first access
//! - Registers the type in the global registry
//! - No manual registration needed!

use crate::models::permission::PermissionBits;
use crate::{Error, Result};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Trait for room setting operations (type-erased)
///
/// This trait provides a unified interface for working with room settings dynamically
pub trait RoomSettingProvider: Send + Sync {
    /// Get the setting key
    fn key(&self) -> &'static str;

    /// Get the setting type name
    fn type_name(&self) -> &'static str;

    /// Validate a raw string value (for dynamic API validation)
    fn is_valid_raw(&self, value: &str) -> Result<()>;

    /// Parse raw string to the setting's value type
    fn parse_raw(&self, value: &str) -> Result<Box<dyn std::any::Any + Send + Sync>>;

    /// Get default value as string
    fn default_as_string(&self) -> String;
}

/// Global registry for all room setting types
///
/// Auto-populated by ctor in each setting type.
static REGISTRY: once_cell::sync::Lazy<RwLock<HashMap<String, Arc<dyn RoomSettingProvider>>>> =
    once_cell::sync::Lazy::new(|| RwLock::new(HashMap::new()));

/// Global registry for all room setting types
pub struct RoomSettingsRegistry;

impl RoomSettingsRegistry {
    /// Register a setting type (called automatically by ctor)
    pub fn register(key: &'static str, provider: Arc<dyn RoomSettingProvider>) {
        let mut registry = REGISTRY.write().unwrap();
        registry.insert(key.to_string(), provider);
    }

    /// Get provider for a setting by key
    pub fn get_provider(key: &str) -> Option<Arc<dyn RoomSettingProvider>> {
        let registry = REGISTRY.read().ok()?;
        registry.get(key).cloned()
    }

    /// Get all registered setting keys
    pub fn all_keys() -> Vec<String> {
        let registry = REGISTRY.read().unwrap();
        registry.keys().cloned().collect()
    }

    /// Check if a setting exists
    pub fn has_key(key: &str) -> bool {
        let registry = REGISTRY.read().unwrap();
        registry.contains_key(key)
    }

    /// Validate a setting value by key (dynamic validation)
    pub fn validate_setting(key: &str, value: &str) -> Result<()> {
        let provider = Self::get_provider(key)
            .ok_or_else(|| Error::NotFound(format!("Setting '{}' not found", key)))?;
        provider.is_valid_raw(value)
    }
}

/// Core trait for room settings
///
/// Each setting type implements this trait.
pub trait RoomSetting: Sized + Send + Sync + 'static {
    /// Storage key in database
    const KEY: &'static str;

    /// The underlying value type
    type Value: Clone + Send + Sync + 'static;

    /// Get the underlying value
    fn value(&self) -> &Self::Value;

    /// Get mutable reference to the value
    fn value_mut(&mut self) -> &mut Self::Value;

    /// Validate the setting value (override for custom validation)
    fn validate(&self) -> Result<()> {
        Ok(())
    }

    /// Parse from string (for dynamic API validation)
    fn parse_from_str(value: &str) -> Result<Self::Value>;

    /// Format to string (for serialization)
    fn format_value(value: &Self::Value) -> String;

    /// Type name (for debugging/registry)
    const TYPE_NAME: &'static str;

    /// Get default value
    fn default_value() -> Self::Value;
}

/// Macro to generate room setting types with automatic ctor registration
///
/// # Examples
///
/// ```rust,ignore
/// room_setting!(ChatEnabled, bool, "chat_enabled", true);
/// room_setting!(MaxMembers, u64, "max_members", 0);
/// ```
///
/// **Auto-registration**: Each type has a `#[ctor]` function that registers default instance!
#[macro_export]
macro_rules! room_setting {
    ($name:ident, $ty:ty, $key:expr, $default:expr) => {
        #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub $ty);

        impl $crate::models::room_settings::RoomSetting for $name {
            const KEY: &'static str = $key;
            const TYPE_NAME: &'static str = stringify!($name);
            type Value = $ty;

            fn value(&self) -> &Self::Value {
                &self.0
            }

            fn value_mut(&mut self) -> &mut Self::Value {
                &mut self.0
            }

            fn parse_from_str(value: &str) -> Result<$ty> {
                value.parse::<$ty>().map_err(|_| {
                    $crate::Error::InvalidInput(format!("Invalid value for {}: {}", $key, value))
                })
            }

            fn format_value(value: &$ty) -> String {
                value.to_string()
            }

            fn default_value() -> $ty {
                $default
            }
        }

        // Implement RoomSettingProvider for dynamic operations
        impl $crate::models::room_settings::RoomSettingProvider for $name {
            fn key(&self) -> &'static str {
                <$name as $crate::models::room_settings::RoomSetting>::KEY
            }

            fn type_name(&self) -> &'static str {
                <$name as $crate::models::room_settings::RoomSetting>::TYPE_NAME
            }

            fn is_valid_raw(&self, value: &str) -> Result<()> {
                Self::parse_from_str(value)?;
                Ok(())
            }

            fn parse_raw(&self, value: &str) -> Result<Box<dyn std::any::Any + Send + Sync>> {
                let parsed = Self::parse_from_str(value)?;
                Ok(Box::new(parsed))
            }

            fn default_as_string(&self) -> String {
                Self::format_value(
                    &<$name as $crate::models::room_settings::RoomSetting>::default_value(),
                )
            }
        }

        // Auto-registration at program startup using ctor
        paste::paste! {
            #[ctor::ctor]
            fn [<_register_ $name:snake>]() {
                // Register the default instance as provider
                let default_instance: $name = std::default::Default::default();
                $crate::models::room_settings::RoomSettingsRegistry::register(
                    $key,
                    std::sync::Arc::new(default_instance),
                );
            }
        }

        impl std::default::Default for $name {
            fn default() -> Self {
                Self($default)
            }
        }
    };
}

// ==================== Generate Setting Types ====================
// Each type has its own lazy_static! that auto-registers!

room_setting!(ChatEnabled, bool, "chat_enabled", true);
room_setting!(DanmakuEnabled, bool, "danmaku_enabled", true);
room_setting!(AllowGuestJoin, bool, "allow_guest_join", false);
room_setting!(RequirePassword, bool, "require_password", false);
room_setting!(RequireApproval, bool, "require_approval", false);
room_setting!(AllowAutoJoin, bool, "allow_auto_join", true);
room_setting!(AutoPlayNext, bool, "auto_play_next", false);
room_setting!(LoopPlaylist, bool, "loop_playlist", false);
room_setting!(ShufflePlaylist, bool, "shuffle_playlist", false);

room_setting!(MaxMembers, u64, "max_members", 0);

room_setting!(AdminAddedPermissions, u64, "admin_added_permissions", 0);
room_setting!(AdminRemovedPermissions, u64, "admin_removed_permissions", 0);
room_setting!(MemberAddedPermissions, u64, "member_added_permissions", 0);
room_setting!(
    MemberRemovedPermissions,
    u64,
    "member_removed_permissions",
    0
);
room_setting!(GuestAddedPermissions, u64, "guest_added_permissions", 0);
room_setting!(GuestRemovedPermissions, u64, "guest_removed_permissions", 0);

use crate::models::room::AutoPlaySettings;

/// Auto play settings (complex type)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
#[derive(Default)]
pub struct AutoPlay {
    pub value: AutoPlaySettings,
}

impl AutoPlay {
    pub fn new(value: AutoPlaySettings) -> Self {
        Self { value }
    }
}

impl RoomSetting for AutoPlay {
    const KEY: &'static str = "auto_play";
    const TYPE_NAME: &'static str = "AutoPlay";
    type Value = AutoPlaySettings;

    fn value(&self) -> &AutoPlaySettings {
        &self.value
    }

    fn value_mut(&mut self) -> &mut AutoPlaySettings {
        &mut self.value
    }

    fn parse_from_str(value: &str) -> Result<AutoPlaySettings> {
        serde_json::from_str(value).map_err(|_| {
            crate::Error::InvalidInput(format!("Invalid JSON for auto_play: {}", value))
        })
    }

    fn format_value(value: &AutoPlaySettings) -> String {
        serde_json::to_string(value).unwrap_or_default()
    }

    fn default_value() -> AutoPlaySettings {
        AutoPlaySettings::default()
    }
}

// Implement RoomSettingProvider for AutoPlay
impl RoomSettingProvider for AutoPlay {
    fn key(&self) -> &'static str {
        <AutoPlay as RoomSetting>::KEY
    }

    fn type_name(&self) -> &'static str {
        <AutoPlay as RoomSetting>::TYPE_NAME
    }

    fn is_valid_raw(&self, value: &str) -> Result<()> {
        Self::parse_from_str(value)?;
        Ok(())
    }

    fn parse_raw(&self, value: &str) -> Result<Box<dyn std::any::Any + Send + Sync>> {
        let parsed = Self::parse_from_str(value)?;
        Ok(Box::new(parsed))
    }

    fn default_as_string(&self) -> String {
        Self::format_value(&AutoPlaySettings::default())
    }
}

// Auto-registration at program startup using ctor for AutoPlay
#[ctor::ctor]
fn _auto_play_register() {
    let default_instance: AutoPlay = std::default::Default::default();
    RoomSettingsRegistry::register("auto_play", std::sync::Arc::new(default_instance));
}


use serde::{Deserialize, Serialize};

/// Room settings composed of individual type-safe settings
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RoomSettings {
    pub require_password: RequirePassword,
    pub allow_guest_join: AllowGuestJoin,
    pub max_members: MaxMembers,
    pub require_approval: RequireApproval,
    pub allow_auto_join: AllowAutoJoin,
    pub chat_enabled: ChatEnabled,
    pub danmaku_enabled: DanmakuEnabled,
    #[serde(default)]
    pub auto_play_next: AutoPlayNext,
    #[serde(default)]
    pub loop_playlist: LoopPlaylist,
    #[serde(default)]
    pub shuffle_playlist: ShufflePlaylist,
    pub auto_play: AutoPlay,
    pub admin_added_permissions: AdminAddedPermissions,
    pub admin_removed_permissions: AdminRemovedPermissions,
    pub member_added_permissions: MemberAddedPermissions,
    pub member_removed_permissions: MemberRemovedPermissions,
    pub guest_added_permissions: GuestAddedPermissions,
    pub guest_removed_permissions: GuestRemovedPermissions,
}

impl RoomSettings {
    /// Get effective permissions for Admin role
    ///
    /// Formula: (global_default | added) & ~removed
    pub fn admin_permissions(&self, global_default: PermissionBits) -> PermissionBits {
        let mut result = global_default.0;
        result |= self.admin_added_permissions.0;
        result &= !self.admin_removed_permissions.0;
        PermissionBits(result)
    }

    /// Get effective permissions for Member role
    pub fn member_permissions(&self, global_default: PermissionBits) -> PermissionBits {
        let mut result = global_default.0;
        result |= self.member_added_permissions.0;
        result &= !self.member_removed_permissions.0;
        PermissionBits(result)
    }

    /// Get effective permissions for Guest
    pub fn guest_permissions(&self, global_default: PermissionBits) -> PermissionBits {
        let mut result = global_default.0;
        result |= self.guest_added_permissions.0;
        result &= !self.guest_removed_permissions.0;
        PermissionBits(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bool_setting() {
        let setting = ChatEnabled(true);
        assert_eq!(ChatEnabled::KEY, "chat_enabled");
        assert!(*setting.value());
    }

    #[test]
    fn test_optional_setting() {
        let setting = MaxMembers(100);
        assert_eq!(MaxMembers::KEY, "max_members");
        assert_eq!(*setting.value(), 100);
        // Test that 0 means no limit
        let no_limit = MaxMembers(0);
        assert_eq!(*no_limit.value(), 0);
    }

    #[test]
    fn test_registry() {
        assert!(RoomSettingsRegistry::has_key("chat_enabled"));
        assert!(RoomSettingsRegistry::has_key("max_members"));
    }

    #[test]
    fn test_dynamic_validation() {
        // Test bool validation
        assert!(RoomSettingsRegistry::validate_setting("chat_enabled", "true").is_ok());
        assert!(RoomSettingsRegistry::validate_setting("chat_enabled", "false").is_ok());
        assert!(RoomSettingsRegistry::validate_setting("chat_enabled", "invalid").is_err());

        // Test i64 validation
        assert!(RoomSettingsRegistry::validate_setting("admin_added_permissions", "123").is_ok());
        assert!(
            RoomSettingsRegistry::validate_setting("admin_added_permissions", "invalid").is_err()
        );

        // Test u64 validation (max_members)
        assert!(RoomSettingsRegistry::validate_setting("max_members", "100").is_ok());
        assert!(RoomSettingsRegistry::validate_setting("max_members", "0").is_ok());
        assert!(RoomSettingsRegistry::validate_setting("max_members", "invalid").is_err());
    }

    #[test]
    fn test_get_provider() {
        let provider = RoomSettingsRegistry::get_provider("chat_enabled").unwrap();
        assert_eq!(provider.key(), "chat_enabled");
        assert_eq!(provider.type_name(), "ChatEnabled");
        assert_eq!(provider.default_as_string(), "true");
    }

    #[test]
    fn test_serialize_deserialize() {
        let settings = RoomSettings {
            chat_enabled: ChatEnabled(false),
            max_members: MaxMembers(100),
            ..Default::default()
        };

        let json = serde_json::to_string(&settings).unwrap();
        let deserialized: RoomSettings = serde_json::from_str(&json).unwrap();

        assert!(!deserialized.chat_enabled.0);
        assert_eq!(deserialized.max_members.0, 100);
    }
}
