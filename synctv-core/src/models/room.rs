use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use super::id::{RoomId, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum RoomStatus {
    #[default]
    Active,
    Closed,
}

impl RoomStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Closed => "closed",
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

    pub fn close(&mut self) {
        self.status = RoomStatus::Closed;
        self.updated_at = Utc::now();
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
    pub auto_play_next: bool,
    pub loop_playlist: bool,
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
