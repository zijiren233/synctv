use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::id::{RoomId, UserId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String, // nanoid(12)
    pub room_id: RoomId,
    pub user_id: UserId,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

impl ChatMessage {
    #[must_use]
    pub fn new(room_id: RoomId, user_id: UserId, content: String) -> Self {
        Self {
            id: super::id::generate_id(),
            room_id,
            user_id,
            content,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendChatRequest {
    pub room_id: RoomId,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatHistoryQuery {
    pub room_id: RoomId,
    pub limit: i32,
    pub before: Option<DateTime<Utc>>,
}

impl Default for ChatHistoryQuery {
    fn default() -> Self {
        Self {
            room_id: RoomId::from_string(String::new()),
            limit: 100,
            before: None,
        }
    }
}

/// Danmaku message (memory-only, not persisted)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DanmakuMessage {
    pub room_id: RoomId,
    pub user_id: UserId,
    pub content: String,
    pub color: String, // hex color
    pub position: DanmakuPosition,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DanmakuPosition {
    Top = 0,
    Bottom = 1,
    Scroll = 2,
}

impl DanmakuMessage {
    #[must_use] 
    pub fn new(
        room_id: RoomId,
        user_id: UserId,
        content: String,
        color: String,
        position: DanmakuPosition,
    ) -> Self {
        Self {
            room_id,
            user_id,
            content,
            color,
            position,
            timestamp: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendDanmakuRequest {
    pub room_id: RoomId,
    pub content: String,
    pub color: String,
    pub position: DanmakuPosition,
}
