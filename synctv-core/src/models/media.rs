use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use super::id::{MediaId, RoomId, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    Bilibili,
    Alist,
    Emby,
    DirectUrl,
}

impl ProviderType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "bilibili" => Some(Self::Bilibili),
            "alist" => Some(Self::Alist),
            "emby" => Some(Self::Emby),
            "directurl" | "direct" => Some(Self::DirectUrl),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bilibili => "bilibili",
            Self::Alist => "alist",
            Self::Emby => "emby",
            Self::DirectUrl => "directurl",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Media {
    pub id: MediaId,
    pub room_id: RoomId,
    pub url: String,
    pub provider: ProviderType,
    pub title: String,
    pub metadata: JsonValue,
    pub position: i32,
    pub added_at: DateTime<Utc>,
    pub added_by: UserId,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Media {
    pub fn new(
        room_id: RoomId,
        url: String,
        provider: ProviderType,
        title: String,
        metadata: JsonValue,
        position: i32,
        added_by: UserId,
    ) -> Self {
        Self {
            id: MediaId::new(),
            room_id,
            url,
            provider,
            title,
            metadata,
            position,
            added_at: Utc::now(),
            added_by,
            deleted_at: None,
        }
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddMediaRequest {
    pub url: String,
    pub provider: Option<ProviderType>,
    pub title: Option<String>,
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
