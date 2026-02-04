//! System settings model for runtime configuration
//!
//! Settings are organized by groups (e.g., "server", "email", "oauth")
//! Each group contains JSON settings that can be updated at runtime

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Settings group names constants
pub mod groups {
    pub const SERVER: &str = "server";
    pub const EMAIL: &str = "email";
    pub const OAUTH: &str = "oauth";
    pub const RATE_LIMIT: &str = "rate_limit";
    pub const CONTENT_MODERATION: &str = "content_moderation";
}

/// Server settings key constants
pub mod server {
    pub const ALLOW_REGISTRATION: &str = "allow_registration";
    pub const SIGNUP_ENABLED: &str = "signup_enabled";
    pub const ALLOW_ROOM_CREATION: &str = "allow_room_creation";
    pub const MAX_ROOMS_PER_USER: &str = "max_rooms_per_user";
    pub const MAX_MEMBERS_PER_ROOM: &str = "max_members_per_room";

    pub const DEFAULT_ROOM_SETTINGS: &str = "default_room_settings";
    pub const DEFAULT_ROOM_REQUIRE_PASSWORD: &str = "require_password";
    pub const DEFAULT_ROOM_ALLOW_GUEST: &str = "allow_guest";
}

/// Email settings key constants
pub mod email {
    pub const ENABLED: &str = "enabled";
    pub const SMTP_HOST: &str = "smtp_host";
    pub const SMTP_PORT: &str = "smtp_port";
    pub const SMTP_USERNAME: &str = "smtp_username";
    pub const USE_TLS: &str = "use_tls";
    pub const FROM_ADDRESS: &str = "from_address";
    pub const FROM_NAME: &str = "from_name";
}

/// OAuth settings key constants
pub mod oauth {
    pub const GITHUB_ENABLED: &str = "github_enabled";
    pub const GOOGLE_ENABLED: &str = "google_enabled";
    pub const MICROSOFT_ENABLED: &str = "microsoft_enabled";
    pub const DISCORD_ENABLED: &str = "discord_enabled";
}

/// Rate limit settings key constants
pub mod rate_limit {
    pub const ENABLED: &str = "enabled";
    pub const API_RATE_LIMIT: &str = "api_rate_limit";
    pub const API_RATE_WINDOW: &str = "api_rate_window";
    pub const WS_RATE_LIMIT: &str = "ws_rate_limit";
    pub const WS_RATE_WINDOW: &str = "ws_rate_window";
}

/// Content moderation settings key constants
pub mod content_moderation {
    pub const ENABLED: &str = "enabled";
    pub const FILTER_PROFANITY: &str = "filter_profanity";
    pub const MAX_MESSAGE_LENGTH: &str = "max_message_length";
    pub const LINK_FILTER_ENABLED: &str = "link_filter_enabled";
}

/// System settings group
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsGroup {
    pub key: String,
    pub group: String,
    pub value: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SettingsGroup {
    /// Create a new settings group
    #[must_use] 
    pub fn new(group: String, value: String) -> Self {
        Self {
            key: format!("{group}.default"),
            group,
            value,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Parse value as JSON value
    pub fn parse_json(&self) -> anyhow::Result<JsonValue> {
        serde_json::from_str(&self.value)
            .map_err(|e| anyhow::anyhow!("Failed to parse settings value: {e}"))
    }

    /// Get value as JSON object
    pub fn as_object(&self) -> anyhow::Result<serde_json::Map<String, JsonValue>> {
        self.parse_json()?
            .as_object()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("settings value is not an object"))
    }
}

/// Settings error types
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("Invalid settings path: {0}")]
    InvalidPath(String),

    #[error("Failed to merge settings")]
    MergeFailed,

    #[error("Serialization error: {0}")]
    SerializationError(#[source] serde_json::Error),

    #[error("Deserialization error: {0}")]
    DeserializationError(#[source] serde_json::Error),

    #[error("Settings group not found: {0}")]
    NotFound(String),
}

/// Default settings for server group
#[must_use] 
pub fn default_server_settings() -> JsonValue {
    serde_json::json!({
        "allow_registration": true,
        "allow_room_creation": true,
        "max_rooms_per_user": 10,
        "max_members_per_room": 100,
        "default_room_settings": {
            "require_password": false,
            "allow_guest": true
        }
    })
}

/// Default settings for email group
#[must_use] 
pub fn default_email_settings() -> JsonValue {
    serde_json::json!({
        "enabled": false,
        "smtp_host": "",
        "smtp_port": 587,
        "smtp_username": "",
        "use_tls": true,
        "from_address": "noreply@synctv.example.com",
        "from_name": "SyncTV"
    })
}

/// Default settings for OAuth group
#[must_use] 
pub fn default_oauth_settings() -> JsonValue {
    serde_json::json!({
        "github_enabled": false,
        "google_enabled": false,
        "microsoft_enabled": false,
        "discord_enabled": false
    })
}

/// Get default settings for a group
#[must_use] 
pub fn get_default_settings(group_name: &str) -> Option<JsonValue> {
    match group_name {
        "server" => Some(default_server_settings()),
        "email" => Some(default_email_settings()),
        "oauth" => Some(default_oauth_settings()),
        "rate_limit" => Some(serde_json::json!({
            "enabled": true,
            "api_rate_limit": 100,
            "api_rate_window": 60,
            "ws_rate_limit": 50,
            "ws_rate_window": 60
        })),
        "content_moderation" => Some(serde_json::json!({
            "enabled": false,
            "filter_profanity": false,
            "max_message_length": 1000,
            "link_filter_enabled": false
        })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_default_settings() {
        let server_settings = get_default_settings("server");
        assert!(server_settings.is_some());
        assert_eq!(server_settings.unwrap().get("allow_registration").cloned().unwrap_or(JsonValue::Null), JsonValue::Bool(true));
    }

    #[test]
    fn test_parse_json() {
        let settings = SettingsGroup::new(
            "server".to_string(),
            serde_json::json!({"test": true}).to_string(),
        );

        let parsed = settings.parse_json().unwrap();
        assert_eq!(parsed.get("test").cloned().unwrap(), JsonValue::Bool(true));
    }

    #[test]
    fn test_as_object() {
        let settings = SettingsGroup::new(
            "server".to_string(),
            serde_json::json!({"key1": "value1", "key2": 123}).to_string(),
        );

        let obj = settings.as_object().unwrap();
        assert_eq!(obj.get("key1").cloned().unwrap(), JsonValue::String("value1".to_string()));
        assert_eq!(obj.get("key2").cloned().unwrap(), JsonValue::Number(123.into()));
    }
}
