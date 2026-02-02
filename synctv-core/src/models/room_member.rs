use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use super::id::{RoomId, UserId};
use super::permission::{PermissionBits, Role as RoomRole};

/// Member status in room (independent of role)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemberStatus {
    /// Active member
    Active,
    /// Pending approval (if room requires approval)
    Pending,
    /// Banned from room
    Banned,
}

impl Default for MemberStatus {
    fn default() -> Self {
        Self::Active
    }
}

impl MemberStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Pending => "pending",
            Self::Banned => "banned",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    pub fn is_banned(&self) -> bool {
        matches!(self, Self::Banned)
    }
}

impl FromStr for MemberStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "active" => Ok(Self::Active),
            "pending" => Ok(Self::Pending),
            "banned" => Ok(Self::Banned),
            _ => Err(format!("Unknown member status: {}", s)),
        }
    }
}

impl std::fmt::Display for MemberStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMember {
    pub room_id: RoomId,
    pub user_id: UserId,

    /// Room role (permission level)
    pub role: RoomRole,

    /// Member status (account state)
    pub status: MemberStatus,

    /// Allow/Deny permission pattern
    /// - effective_permissions = (role_default | added) & ~removed
    /// - None = use role default permissions
    pub added_permissions: Option<i64>,
    pub removed_permissions: Option<i64>,

    pub joined_at: DateTime<Utc>,
    pub left_at: Option<DateTime<Utc>>,

    /// Version for optimistic locking
    pub version: i64,

    /// Banned info
    pub banned_at: Option<DateTime<Utc>>,
    pub banned_by: Option<UserId>,
    pub banned_reason: Option<String>,
}

impl RoomMember {
    pub fn new(room_id: RoomId, user_id: UserId, role: RoomRole) -> Self {
        let now = Utc::now();
        Self {
            room_id,
            user_id,
            role,
            status: MemberStatus::Active,
            added_permissions: None,
            removed_permissions: None,
            joined_at: now,
            left_at: None,
            version: 0,
            banned_at: None,
            banned_by: None,
            banned_reason: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.status.is_active() && self.left_at.is_none()
    }

    /// Calculate effective permissions using Allow/Deny pattern
    ///
    /// Permission inheritance chain:
    /// 1. Creator → All permissions (fixed)
    /// 2. Admin/Member → (role_default | added) & ~removed
    /// 3. Guest → guest_permissions from room settings
    pub fn effective_permissions(&self, room_default_permissions: Option<i64>) -> PermissionBits {
        match self.role {
            RoomRole::Creator => {
                // Creator has all permissions (fixed, cannot be modified)
                PermissionBits(PermissionBits::ALL)
            }
            RoomRole::Guest => {
                // Guests use room guest permissions (or global default if not configured)
                let base = room_default_permissions.unwrap_or_else(|| {
                    // Global default guest permissions
                    PermissionBits::SEND_CHAT
                });
                PermissionBits(base)
            }
            RoomRole::Admin | RoomRole::Member => {
                // Get role base permissions
                let base = room_default_permissions.unwrap_or_else(|| {
                    // Use global default role permissions
                    match self.role {
                        RoomRole::Admin => {
                            let mut perms = PermissionBits::empty();
                            perms.grant(PermissionBits::DELETE_ROOM);
                            perms.grant(PermissionBits::UPDATE_ROOM_SETTINGS);
                            perms.grant(PermissionBits::INVITE_USER);
                            perms.grant(PermissionBits::KICK_USER);
                            perms.grant(PermissionBits::ADD_MEDIA);
                            perms.grant(PermissionBits::REMOVE_MEDIA);
                            perms.grant(PermissionBits::REORDER_PLAYLIST);
                            perms.grant(PermissionBits::SWITCH_MEDIA);
                            perms.grant(PermissionBits::PLAY_PAUSE);
                            perms.grant(PermissionBits::SEEK);
                            perms.grant(PermissionBits::CHANGE_SPEED);
                            perms.grant(PermissionBits::SEND_CHAT);
                            perms.grant(PermissionBits::SEND_DANMAKU);
                            perms.grant(PermissionBits::DELETE_MESSAGE);
                            perms.grant(PermissionBits::GRANT_PERMISSION);
                            perms.grant(PermissionBits::REVOKE_PERMISSION);
                            perms.0
                        }
                        RoomRole::Member => {
                            let mut perms = PermissionBits::empty();
                            perms.grant(PermissionBits::ADD_MEDIA);
                            perms.grant(PermissionBits::PLAY_PAUSE);
                            perms.grant(PermissionBits::SEEK);
                            perms.grant(PermissionBits::SEND_CHAT);
                            perms.grant(PermissionBits::SEND_DANMAKU);
                            perms.0
                        }
                        _ => 0,
                    }
                });

                // Apply Allow/Deny modifications
                let mut result = base;

                // Add extra permissions
                if let Some(added) = self.added_permissions {
                    result |= added;
                }

                // Remove permissions
                if let Some(removed) = self.removed_permissions {
                    result &= !removed;
                }

                PermissionBits(result)
            }
        }
    }

    /// Check if member has a specific permission (considers both status and effective permissions)
    pub fn has_permission(&self, permission: i64, room_default_permissions: Option<i64>) -> bool {
        if !self.status.is_active() {
            return false;
        }

        self.effective_permissions(room_default_permissions).has(permission)
    }

    pub fn leave(&mut self) {
        self.left_at = Some(Utc::now());
    }

    /// Ban this member from the room
    pub fn ban(&mut self, banned_by: UserId, reason: Option<String>) {
        self.status = MemberStatus::Banned;
        self.banned_at = Some(Utc::now());
        self.banned_by = Some(banned_by);
        self.banned_reason = reason;
    }

    /// Unban this member
    pub fn unban(&mut self) {
        self.status = MemberStatus::Active;
        self.banned_at = None;
        self.banned_by = None;
        self.banned_reason = None;
    }

    /// Set added permissions (Allow pattern)
    pub fn add_permissions(&mut self, permissions: i64) {
        let current = self.added_permissions.unwrap_or(0);
        self.added_permissions = Some(current | permissions);
    }

    /// Set removed permissions (Deny pattern)
    pub fn remove_permissions(&mut self, permissions: i64) {
        let current = self.removed_permissions.unwrap_or(0);
        self.removed_permissions = Some(current | permissions);
    }

    /// Reset to role default (clear both added and removed)
    pub fn reset_to_role_default(&mut self) {
        self.added_permissions = None;
        self.removed_permissions = None;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMemberWithUser {
    pub room_id: RoomId,
    pub user_id: UserId,
    pub username: String,
    pub role: RoomRole,
    pub status: MemberStatus,
    pub added_permissions: Option<i64>,
    pub removed_permissions: Option<i64>,
    pub joined_at: DateTime<Utc>,
    pub is_online: bool,
    pub banned_at: Option<DateTime<Utc>>,
    pub banned_reason: Option<String>,
}

impl RoomMemberWithUser {
    /// Calculate effective permissions for display
    pub fn effective_permissions(&self, room_default_permissions: Option<i64>) -> PermissionBits {
        let member = RoomMember {
            room_id: self.room_id.clone(),
            user_id: self.user_id.clone(),
            role: self.role,
            status: self.status,
            added_permissions: self.added_permissions,
            removed_permissions: self.removed_permissions,
            joined_at: self.joined_at,
            left_at: None,
            version: 0,
            banned_at: self.banned_at,
            banned_by: None,
            banned_reason: self.banned_reason.clone(),
        };

        member.effective_permissions(room_default_permissions)
    }
}
