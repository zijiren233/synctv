//! Global setting variables
//!
//! This module defines all setting variables used throughout the application.
//! Each variable is type-safe, thread-safe, and automatically persists to the database.
//!
//! # Usage
//!
//! ```rust,ignore
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
use serde::{Deserialize, Serialize};
use crate::service::{SettingsService, settings_vars::{Setting, SettingsStorage}};
use crate::setting;

/// A snapshot of all client-visible settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicSettings {
    pub signup_enabled: bool,
    pub allow_room_creation: bool,
    pub max_rooms_per_user: i64,
    pub max_members_per_room: i64,

    // Room settings
    pub disable_create_room: bool,
    pub create_room_need_review: bool,
    pub room_ttl: i64,
    pub room_must_need_pwd: bool,

    // User settings
    pub signup_need_review: bool,
    pub enable_password_signup: bool,
    pub enable_guest: bool,

    // Proxy settings
    pub movie_proxy: bool,
    pub live_proxy: bool,

    // RTMP settings
    pub rtmp_player: bool,
    pub ts_disguised_as_png: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub custom_publish_host: String,

    // Email settings
    pub email_whitelist_enabled: bool,

    // Server settings
    pub p2p_zone: String,
}

impl PublicSettings {
    /// Default settings when the settings registry is not configured.
    #[must_use]
    pub fn defaults() -> Self {
        Self {
            signup_enabled: true,
            allow_room_creation: true,
            max_rooms_per_user: 10,
            max_members_per_room: 100,
            disable_create_room: false,
            create_room_need_review: false,
            room_ttl: 172800,
            room_must_need_pwd: false,
            signup_need_review: false,
            enable_password_signup: true,
            enable_guest: true,
            movie_proxy: true,
            live_proxy: true,
            rtmp_player: false,
            ts_disguised_as_png: true,
            custom_publish_host: String::new(),
            email_whitelist_enabled: false,
            p2p_zone: "hk".to_string(),
        }
    }
}

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
    pub max_chat_messages: Setting<u64>,

    // Permission settings - global defaults for each role
    pub admin_default_permissions: Setting<u64>,
    pub member_default_permissions: Setting<u64>,
    pub guest_default_permissions: Setting<u64>,

    // Room settings
    pub disable_create_room: Setting<bool>,
    pub create_room_need_review: Setting<bool>,
    pub room_ttl: Setting<i64>,
    pub room_must_need_pwd: Setting<bool>,
    pub room_must_no_need_pwd: Setting<bool>,

    // User settings
    pub signup_need_review: Setting<bool>,
    pub enable_password_signup: Setting<bool>,
    pub password_signup_need_review: Setting<bool>,
    pub enable_guest: Setting<bool>,

    // Proxy settings
    pub movie_proxy: Setting<bool>,
    pub live_proxy: Setting<bool>,
    pub allow_proxy_to_local: Setting<bool>,
    pub proxy_cache_enable: Setting<bool>,

    // RTMP settings
    pub rtmp_player: Setting<bool>,
    pub custom_publish_host: Setting<String>,
    pub ts_disguised_as_png: Setting<bool>,

    // Email settings
    pub email_whitelist_enabled: Setting<bool>,
    pub email_whitelist: Setting<String>,

    // Server/network settings
    pub p2p_zone: Setting<String>,
}

impl std::fmt::Debug for SettingsRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsRegistry")
            .finish()
    }
}

impl SettingsRegistry {
    /// Create a new settings registry with all setting instances
    #[must_use] 
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
            max_chat_messages: setting!(u64, "server.max_chat_messages", storage.clone(), 500,
                |v: &u64| -> anyhow::Result<()> {
                    if *v <= 10000 {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("max_chat_messages must be at most 10000 (0 = unlimited)"))
                    }
                }
            ),

            // Permission settings - global defaults for each role
            // These are base permissions that rooms can override with added/removed permissions
            // Admin default: All permissions except System::ADMIN (1073741823 = 0x3FFFFFFF)
            admin_default_permissions: setting!(u64, "permissions.admin_default", storage.clone(), 1073741823),
            // Member default: Basic member permissions (262143 = 0x3FFFF)
            member_default_permissions: setting!(u64, "permissions.member_default", storage.clone(), 262143),
            // Guest default: Read-only permissions (511 = 0x1FF)
            guest_default_permissions: setting!(u64, "permissions.guest_default", storage.clone(), 511),

            // Room settings
            disable_create_room: setting!(bool, "room.disable_create_room", storage.clone(), false),
            create_room_need_review: setting!(bool, "room.create_room_need_review", storage.clone(), false),
            room_ttl: setting!(i64, "room.room_ttl", storage.clone(), 172800, // 48 hours in seconds
                |v: &i64| -> anyhow::Result<()> {
                    if *v >= 0 {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("room_ttl must be non-negative (0 = never expire)"))
                    }
                }
            ),
            room_must_need_pwd: setting!(bool, "room.room_must_need_pwd", storage.clone(), false),
            room_must_no_need_pwd: setting!(bool, "room.room_must_no_need_pwd", storage.clone(), false),

            // User settings
            signup_need_review: setting!(bool, "user.signup_need_review", storage.clone(), false),
            enable_password_signup: setting!(bool, "user.enable_password_signup", storage.clone(), true),
            password_signup_need_review: setting!(bool, "user.password_signup_need_review", storage.clone(), false),
            enable_guest: setting!(bool, "user.enable_guest", storage.clone(), true),

            // Proxy settings
            movie_proxy: setting!(bool, "proxy.movie_proxy", storage.clone(), true),
            live_proxy: setting!(bool, "proxy.live_proxy", storage.clone(), true),
            allow_proxy_to_local: setting!(bool, "proxy.allow_proxy_to_local", storage.clone(), false),
            proxy_cache_enable: setting!(bool, "proxy.proxy_cache_enable", storage.clone(), false),

            // RTMP settings
            rtmp_player: setting!(bool, "rtmp.rtmp_player", storage.clone(), false),
            custom_publish_host: setting!(String, "rtmp.custom_publish_host", storage.clone(), String::new()),
            ts_disguised_as_png: setting!(bool, "rtmp.ts_disguised_as_png", storage.clone(), true),

            // Email settings
            email_whitelist_enabled: setting!(bool, "email.whitelist_enabled", storage.clone(), false),
            email_whitelist: setting!(String, "email.whitelist", storage.clone(), String::new()),

            // Server/network settings
            p2p_zone: setting!(String, "server.p2p_zone", storage, "hk".to_string()),
        }
    }

    /// Initialize storage from database
    pub async fn init(&self) -> anyhow::Result<()> {
        // Load raw values from database into shared storage
        // Individual settings will lazy-load on first get()
        self.storage.init().await?;
        Ok(())
    }

    /// Build a `PublicSettings` snapshot from the current registry values.
    #[must_use]
    pub fn to_public_settings(&self) -> PublicSettings {
        PublicSettings {
            signup_enabled: self.signup_enabled.get().unwrap_or(true),
            allow_room_creation: self.allow_room_creation.get().unwrap_or(true),
            max_rooms_per_user: self.max_rooms_per_user.get().unwrap_or(10),
            max_members_per_room: self.max_members_per_room.get().unwrap_or(100),
            disable_create_room: self.disable_create_room.get().unwrap_or(false),
            create_room_need_review: self.create_room_need_review.get().unwrap_or(false),
            room_ttl: self.room_ttl.get().unwrap_or(172800),
            room_must_need_pwd: self.room_must_need_pwd.get().unwrap_or(false),
            signup_need_review: self.signup_need_review.get().unwrap_or(false),
            enable_password_signup: self.enable_password_signup.get().unwrap_or(true),
            enable_guest: self.enable_guest.get().unwrap_or(true),
            movie_proxy: self.movie_proxy.get().unwrap_or(true),
            live_proxy: self.live_proxy.get().unwrap_or(true),
            rtmp_player: self.rtmp_player.get().unwrap_or(false),
            ts_disguised_as_png: self.ts_disguised_as_png.get().unwrap_or(true),
            custom_publish_host: self.custom_publish_host.get().unwrap_or_default(),
            email_whitelist_enabled: self.email_whitelist_enabled.get().unwrap_or(false),
            p2p_zone: self.p2p_zone.get().unwrap_or_else(|_| "hk".to_string()),
        }
    }
}

