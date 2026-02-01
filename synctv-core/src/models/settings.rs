//! System settings model for runtime configuration
//!
//! Settings are organized by groups (e.g., "server", "email", "oauth")
//! Each group contains JSON settings that can be updated at runtime

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// System settings group
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsGroup {
    pub id: i64,
    pub group_name: String,
    pub settings_json: JsonValue,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SettingsGroup {
    /// Create a new settings group
    pub fn new(group_name: String, settings_json: JsonValue, description: Option<String>) -> Self {
        Self {
            id: 0, // Will be set by database
            group_name,
            settings_json,
            description,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Get a specific setting value by key path (e.g., "server.allow_registration")
    pub fn get(&self, path: &str) -> Option<&JsonValue> {
        let parts: Vec<&str> = path.split('.').collect();
        let mut current = &self.settings_json;

        for part in parts {
            match current.get(part) {
                Some(value) => current = value,
                None => return None,
            }
        }

        Some(current)
    }

    /// Get a boolean setting value
    pub fn get_bool(&self, path: &str) -> Option<bool> {
        self.get(path).and_then(|v| v.as_bool())
    }

    /// Get a string setting value
    pub fn get_str(&self, path: &str) -> Option<&str> {
        self.get(path).and_then(|v| v.as_str())
    }

    /// Get an integer setting value
    pub fn get_i64(&self, path: &str) -> Option<i64> {
        self.get(path).and_then(|v| v.as_i64())
    }

    /// Get a nested object setting value
    pub fn get_object(&self, path: &str) -> Option<&serde_json::Map<String, JsonValue>> {
        self.get(path).and_then(|v| v.as_object())
    }

    /// Update a specific setting value by key path
    pub fn set(&mut self, path: &str, value: JsonValue) -> Result<(), SettingsError> {
        let parts: Vec<&str> = path.split('.').collect();
        let current = &mut self.settings_json;

        // Navigate to the parent object
        let mut target = current;
        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                // Last part - set the value
                if let Some(obj) = target.as_object_mut() {
                    obj.insert(part.to_string(), value);
                    self.updated_at = Utc::now();
                    return Ok(());
                }
                return Err(SettingsError::InvalidPath(path.to_string()));
            }

            // Navigate deeper
            if !target.get(part).is_some() {
                // Create missing object
                if let Some(obj) = target.as_object_mut() {
                    obj.insert(part.to_string(), JsonValue::Object(serde_json::Map::new()));
                }
            }

            target = target.get_mut(part).ok_or_else(|| {
                SettingsError::InvalidPath(format!("{}.{}", path, part))
            })?;
        }

        Err(SettingsError::InvalidPath(path.to_string()))
    }

    /// Merge JSON settings into existing settings
    pub fn merge(&mut self, new_settings: JsonValue) -> Result<(), SettingsError> {
        if let Some(obj) = self.settings_json.as_object_mut() {
            if let Some(new_obj) = new_settings.as_object() {
                for (key, value) in new_obj {
                    obj.insert(key.to_string(), value.clone());
                }
                self.updated_at = Utc::now();
                return Ok(());
            }
        }
        Err(SettingsError::MergeFailed)
    }

    /// Convert to protobuf bytes
    pub fn to_proto_bytes(&self) -> Result<Vec<u8>, SettingsError> {
        serde_json::to_vec(&self.settings_json)
            .map_err(SettingsError::SerializationError)
    }

    /// Create from protobuf bytes
    pub fn from_proto_bytes(group_name: String, bytes: &[u8]) -> Result<Self, SettingsError> {
        let settings_json: JsonValue = serde_json::from_slice(bytes)
            .map_err(SettingsError::DeserializationError)?;

        Ok(Self {
            id: 0,
            group_name,
            settings_json,
            description: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
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
pub fn default_oauth_settings() -> JsonValue {
    serde_json::json!({
        "github_enabled": false,
        "google_enabled": false,
        "microsoft_enabled": false,
        "discord_enabled": false
    })
}

/// Get default settings for a group
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
    fn test_get_setting() {
        let settings = SettingsGroup::new(
            "server".to_string(),
            default_server_settings(),
            Some("Server settings".to_string()),
        );

        assert_eq!(settings.get_bool("server.allow_registration"), Some(true));
        assert_eq!(settings.get_i64("server.max_rooms_per_user"), Some(10));
        assert!(settings.get_object("server.default_room_settings").is_some());
    }

    #[test]
    fn test_set_setting() {
        let mut settings = SettingsGroup::new(
            "server".to_string(),
            default_server_settings(),
            Some("Server settings".to_string()),
        );

        settings
            .set("server.allow_registration", JsonValue::Bool(false))
            .unwrap();
        assert_eq!(settings.get_bool("server.allow_registration"), Some(false));
    }

    #[test]
    fn test_merge_settings() {
        let mut settings = SettingsGroup::new(
            "server".to_string(),
            serde_json::json!({"key1": "value1"}),
            Some("Test settings".to_string()),
        );

        let new_settings = serde_json::json!({"key2": "value2"});
        settings.merge(new_settings).unwrap();

        assert_eq!(settings.get_str("key1"), Some("value1"));
        assert_eq!(settings.get_str("key2"), Some("value2"));
    }

    #[test]
    fn test_proto_bytes() {
        let settings = SettingsGroup::new(
            "server".to_string(),
            default_server_settings(),
            Some("Server settings".to_string()),
        );

        let bytes = settings.to_proto_bytes().unwrap();
        let restored = SettingsGroup::from_proto_bytes("server".to_string(), &bytes).unwrap();

        assert_eq!(settings.settings_json, restored.settings_json);
    }
}
