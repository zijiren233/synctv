use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::id::{RoomId, UserId};
use super::permission::PermissionBits;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMember {
    pub room_id: RoomId,
    pub user_id: UserId,
    pub permissions: PermissionBits,
    pub joined_at: DateTime<Utc>,
    pub left_at: Option<DateTime<Utc>>,
}

impl RoomMember {
    pub fn new(room_id: RoomId, user_id: UserId, permissions: PermissionBits) -> Self {
        Self {
            room_id,
            user_id,
            permissions,
            joined_at: Utc::now(),
            left_at: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.left_at.is_none()
    }

    pub fn has_permission(&self, permission: i64) -> bool {
        self.permissions.has(permission)
    }

    pub fn leave(&mut self) {
        self.left_at = Some(Utc::now());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMemberWithUser {
    pub room_id: RoomId,
    pub user_id: UserId,
    pub username: String,
    pub permissions: PermissionBits,
    pub joined_at: DateTime<Utc>,
    pub is_online: bool,
}
