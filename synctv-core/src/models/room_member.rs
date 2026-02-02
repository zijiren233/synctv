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

    /// Allow/Deny permission pattern for member role
    /// - effective_permissions = (role_default | added) & ~removed
    /// - None = use role default permissions
    pub added_permissions: Option<i64>,
    pub removed_permissions: Option<i64>,

    /// Allow/Deny permission pattern for admin role (overrides member-level permissions)
    /// - Only applies when role = Admin
    /// - effective_permissions = (admin_default | admin_added) & ~admin_removed
    pub admin_added_permissions: Option<i64>,
    pub admin_removed_permissions: Option<i64>,

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
            admin_added_permissions: None,
            admin_removed_permissions: None,
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
    /// Permission inheritance chain (three-layer override system):
    /// 1. Global default permissions (from SettingsRegistry)
    /// 2. Room-level override: (global | room_added) & ~room_removed
    /// 3. Member-level override: (room_level | member_added/admin_added) & ~(member_removed/admin_removed)
    ///
    /// Arguments:
    /// - role_default: Already-calculated permissions for this role
    ///   (global default with room-level overrides applied)
    ///
    /// This method then applies member-level overrides to get final permissions
    pub fn effective_permissions(&self, role_default: PermissionBits) -> PermissionBits {
        match self.role {
            RoomRole::Creator => {
                // Creator has all permissions (fixed, cannot be modified)
                PermissionBits(PermissionBits::ALL)
            }
            RoomRole::Admin => {
                // Start with role default (already has global + room overrides)
                let mut result = role_default.0;

                // Apply admin-specific Allow/Deny modifications
                if let Some(added) = self.admin_added_permissions {
                    result |= added;
                }
                if let Some(removed) = self.admin_removed_permissions {
                    result &= !removed;
                }

                PermissionBits(result)
            }
            RoomRole::Member | RoomRole::Guest => {
                // Start with role default (already has global + room overrides)
                let mut result = role_default.0;

                // Apply member-level Allow/Deny modifications
                if let Some(added) = self.added_permissions {
                    result |= added;
                }
                if let Some(removed) = self.removed_permissions {
                    result &= !removed;
                }

                PermissionBits(result)
            }
        }
    }

    /// Check if member has a specific permission (considers both status and effective permissions)
    pub fn has_permission(&self, permission: i64, role_default: PermissionBits) -> bool {
        if !self.status.is_active() {
            return false;
        }

        self.effective_permissions(role_default).has(permission)
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
    pub admin_added_permissions: Option<i64>,
    pub admin_removed_permissions: Option<i64>,
    pub joined_at: DateTime<Utc>,
    pub is_online: bool,
    pub banned_at: Option<DateTime<Utc>>,
    pub banned_reason: Option<String>,
}

impl RoomMemberWithUser {
    /// Calculate effective permissions for display
    ///
    /// Arguments:
    /// - role_default: Already-calculated permissions for this role
    ///   (global default with room-level overrides applied)
    pub fn effective_permissions(&self, role_default: PermissionBits) -> PermissionBits {
        let member = RoomMember {
            room_id: self.room_id.clone(),
            user_id: self.user_id.clone(),
            role: self.role,
            status: self.status,
            added_permissions: self.added_permissions,
            removed_permissions: self.removed_permissions,
            admin_added_permissions: self.admin_added_permissions,
            admin_removed_permissions: self.admin_removed_permissions,
            joined_at: self.joined_at,
            left_at: None,
            version: 0,
            banned_at: self.banned_at,
            banned_by: None,
            banned_reason: self.banned_reason.clone(),
        };

        member.effective_permissions(role_default)
    }
}
