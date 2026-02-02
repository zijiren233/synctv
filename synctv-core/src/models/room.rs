use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use super::id::{RoomId, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum RoomStatus {
    #[default]
    Pending,
    Active,
    Banned,
}

impl RoomStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Banned => "banned",
        }
    }

    pub fn is_banned(&self) -> bool {
        matches!(self, Self::Banned)
    }

    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }
}

/// Playback mode for auto-play
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayMode {
    /// Sequential play (stop after last item)
    Sequential,
    /// Repeat single item
    RepeatOne,
    /// Repeat all items (loop back to start)
    RepeatAll,
    /// Random playback
    Shuffle,
}

impl Default for PlayMode {
    fn default() -> Self {
        Self::Sequential
    }
}

/// Auto-play settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoPlaySettings {
    /// Whether auto-play is enabled
    pub enabled: bool,

    /// Playback mode
    pub mode: PlayMode,

    /// Delay before playing next item (seconds)
    pub delay: u32,
}

impl Default for AutoPlaySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: PlayMode::Sequential,
            delay: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub id: RoomId,
    pub name: String,
    pub created_by: UserId,
    pub status: RoomStatus,
    pub settings: JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Room {
    pub fn new(name: String, created_by: UserId, settings: JsonValue) -> Self {
        let now = Utc::now();
        Self {
            id: RoomId::new(),
            name,
            created_by,
            status: RoomStatus::Active,
            settings,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.status == RoomStatus::Active && self.deleted_at.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRoomRequest {
    pub name: String,
    pub password: Option<String>,
    pub settings: Option<JsonValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateRoomRequest {
    pub name: Option<String>,
    pub status: Option<RoomStatus>,
    pub settings: Option<JsonValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomListQuery {
    pub page: i32,
    pub page_size: i32,
    pub status: Option<RoomStatus>,
    pub search: Option<String>,
}

impl Default for RoomListQuery {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: 20,
            status: Some(RoomStatus::Active),
            search: None,
        }
    }
}

/// Room settings structure (stored as JSON in database)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoomSettings {
    pub require_password: bool,
    /// Auto-play settings (deprecated, use auto_play)
    #[serde(default)]
    pub auto_play_next: bool,
    /// Auto-play settings
    #[serde(default)]
    pub auto_play: AutoPlaySettings,
    /// Legacy: loop playlist (use auto_play.mode instead)
    #[serde(default)]
    pub loop_playlist: bool,
    /// Legacy: shuffle playlist (use auto_play.mode instead)
    #[serde(default)]
    pub shuffle_playlist: bool,
    pub allow_guest_join: bool,
    pub max_members: Option<i32>,
    pub chat_enabled: bool,
    pub danmaku_enabled: bool,
}

/// Room with member count (for efficient queries with JOIN)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomWithCount {
    #[serde(flatten)]
    pub room: Room,
    pub member_count: i32,
}
