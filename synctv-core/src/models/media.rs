use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::str::FromStr;

use super::id::{MediaId, PlaylistId, RoomId, UserId};

/// Media provider type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    DirectUrl,
    Bilibili,
    Alist,
    Emby,
}

impl FromStr for ProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "direct_url" | "directurl" => Ok(Self::DirectUrl),
            "bilibili" => Ok(Self::Bilibili),
            "alist" => Ok(Self::Alist),
            "emby" => Ok(Self::Emby),
            _ => Err(format!("Unknown provider type: {s}")),
        }
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DirectUrl => write!(f, "direct_url"),
            Self::Bilibili => write!(f, "bilibili"),
            Self::Alist => write!(f, "alist"),
            Self::Emby => write!(f, "emby"),
        }
    }
}

/// Media file (video/audio)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Media {
    pub id: MediaId,
    pub playlist_id: PlaylistId,
    pub room_id: RoomId,
    pub creator_id: UserId,
    pub name: String,
    pub position: i32,
    /// Provider type name (e.g., "bilibili", "alist", "emby", "`direct_url`")
    /// Stored as string for flexibility, not an enum
    pub source_provider: String,
    pub source_config: JsonValue,
    pub metadata: JsonValue,
    /// Provider instance name (e.g., "`bilibili_main`", "`alist_company`")
    /// Used to look up the provider from the registry at playback time
    pub provider_instance_name: Option<String>,
    pub added_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Parameters for creating media from a provider
#[derive(Debug, Clone)]
pub struct FromProviderParams {
    pub playlist_id: PlaylistId,
    pub room_id: RoomId,
    pub creator_id: UserId,
    pub name: String,
    pub source_config: JsonValue,
    pub provider_name: String,
    pub provider_instance_name: String,
    pub position: i32,
}

impl Media {
    /// Create media from provider instance (registry pattern)
    ///
    /// This is the preferred way to create media when using the provider registry.
    /// The `provider_instance_name` is used to look up the provider at playback time.
    ///
    /// # Arguments
    /// * `provider_name` - Provider type name from `provider.name()` (e.g., "bilibili")
    /// * `provider_instance_name` - Instance name for lookup (e.g., "`bilibili_main`")
    ///
    /// # Example
    /// ```rust
    /// let provider = providers_manager.get_provider("bilibili_main").await?;
    /// let media = Media::from_provider(..., provider.name(), "bilibili_main", ...);
    /// ```
    #[allow(clippy::too_many_arguments)]
    #[must_use] 
    pub fn from_provider(
        playlist_id: PlaylistId,
        room_id: RoomId,
        creator_id: UserId,
        name: String,
        source_config: JsonValue,
        provider_name: &str,
        provider_instance_name: String,
        position: i32,
    ) -> Self {
        Self {
            id: MediaId::new(),
            playlist_id,
            room_id,
            creator_id,
            name,
            position,
            source_provider: provider_name.to_string(),
            source_config,
            metadata: JsonValue::default(),
            provider_instance_name: Some(provider_instance_name),
            added_at: Utc::now(),
            deleted_at: None,
        }
    }

    /// Create media from provider with parameters struct
    #[must_use] 
    pub fn from_provider_with_params(params: FromProviderParams) -> Self {
        Self {
            id: MediaId::new(),
            playlist_id: params.playlist_id,
            room_id: params.room_id,
            creator_id: params.creator_id,
            name: params.name,
            position: params.position,
            source_provider: params.provider_name,
            source_config: params.source_config,
            metadata: JsonValue::default(),
            provider_instance_name: Some(params.provider_instance_name),
            added_at: Utc::now(),
            deleted_at: None,
        }
    }

    #[must_use] 
    pub const fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }
}

/// Media metadata structure (stored as JSON in database)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MediaMetadata {
    pub title: String,
    pub duration: Option<f64>, // seconds
    pub thumbnail: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub provider_data: JsonValue,
}
