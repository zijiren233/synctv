use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use super::id::{RoomId, UserId};
use super::permission::{PermissionBits, Role as RoomRole};

/// Member status in room (independent of role)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum MemberStatus {
    /// Active member
    #[default]
    Active,
    /// Pending approval (if room requires approval)
    Pending,
    /// Banned from room
    Banned,
}


impl MemberStatus {
    #[must_use] 
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Pending => "pending",
            Self::Banned => "banned",
        }
    }

    #[must_use] 
    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    #[must_use] 
    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    #[must_use] 
    pub const fn is_banned(&self) -> bool {
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
            _ => Err(format!("Unknown member status: {s}")),
        }
    }
}

impl std::fmt::Display for MemberStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// Database mapping: MemberStatus -> SMALLINT (1=active, 2=pending, 3=banned)
impl sqlx::Type<sqlx::Postgres> for MemberStatus {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <i16 as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for MemberStatus {
    fn encode_by_ref(&self, buf: &mut sqlx::postgres::PgArgumentBuffer) -> Result<sqlx::encode::IsNull, Box<dyn std::error::Error + Send + Sync>> {
        let val: i16 = match self {
            Self::Active => 1,
            Self::Pending => 2,
            Self::Banned => 3,
        };
        <i16 as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&val, buf)
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for MemberStatus {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let val = <i16 as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        match val {
            1 => Ok(Self::Active),
            2 => Ok(Self::Pending),
            3 => Ok(Self::Banned),
            _ => Err(format!("Invalid MemberStatus value: {val}").into()),
        }
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
    /// - `effective_permissions` = (`role_default` | added) & ~removed
    pub added_permissions: u64,
    pub removed_permissions: u64,

    /// Allow/Deny permission pattern for admin role (overrides member-level permissions)
    /// - Only applies when role = Admin
    /// - `effective_permissions` = (`admin_default` | `admin_added`) & ~`admin_removed`
    pub admin_added_permissions: u64,
    pub admin_removed_permissions: u64,

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
    #[must_use] 
    pub fn new(room_id: RoomId, user_id: UserId, role: RoomRole) -> Self {
        let now = Utc::now();
        Self {
            room_id,
            user_id,
            role,
            status: MemberStatus::Active,
            added_permissions: 0,
            removed_permissions: 0,
            admin_added_permissions: 0,
            admin_removed_permissions: 0,
            joined_at: now,
            left_at: None,
            version: 0,
            banned_at: None,
            banned_by: None,
            banned_reason: None,
        }
    }

    #[must_use] 
    pub const fn is_active(&self) -> bool {
        self.status.is_active() && self.left_at.is_none()
    }

    /// Calculate effective permissions using Allow/Deny pattern
    ///
    /// Permission inheritance chain (three-layer override system):
    /// 1. Global default permissions (from `SettingsRegistry`)
    /// 2. Room-level override: (global | `room_added`) & ~`room_removed`
    /// 3. Member-level override: (`room_level` | `member_added/admin_added`) & ~(`member_removed/admin_removed`)
    ///
    /// Arguments:
    /// - `role_default`: Already-calculated permissions for this role
    ///   (global default with room-level overrides applied)
    ///
    /// This method then applies member-level overrides to get final permissions
    #[must_use] 
    pub const fn effective_permissions(&self, role_default: PermissionBits) -> PermissionBits {
        match self.role {
            RoomRole::Creator => {
                // Creator has all permissions (fixed, cannot be modified)
                PermissionBits(PermissionBits::ALL)
            }
            RoomRole::Admin => {
                // Start with role default (already has global + room overrides)
                let mut result = role_default.0;

                // Apply admin-specific Allow/Deny modifications
                result |= self.admin_added_permissions;
                result &= !self.admin_removed_permissions;

                PermissionBits(result)
            }
            RoomRole::Member | RoomRole::Guest => {
                // Start with role default (already has global + room overrides)
                let mut result = role_default.0;

                // Apply member-level Allow/Deny modifications
                result |= self.added_permissions;
                result &= !self.removed_permissions;

                PermissionBits(result)
            }
        }
    }

    /// Check if member has a specific permission (considers both status and effective permissions)
    #[must_use] 
    pub const fn has_permission(&self, permission: u64, role_default: PermissionBits) -> bool {
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
    pub const fn add_permissions(&mut self, permissions: u64) {
        self.added_permissions |= permissions;
    }

    /// Set removed permissions (Deny pattern)
    pub const fn remove_permissions(&mut self, permissions: u64) {
        self.removed_permissions |= permissions;
    }

    /// Reset to role default (clear both added and removed)
    pub const fn reset_to_role_default(&mut self) {
        self.added_permissions = 0;
        self.removed_permissions = 0;
        self.admin_added_permissions = 0;
        self.admin_removed_permissions = 0;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMemberWithUser {
    pub room_id: RoomId,
    pub user_id: UserId,
    pub username: String,
    pub role: RoomRole,
    pub status: MemberStatus,
    pub added_permissions: u64,
    pub removed_permissions: u64,
    pub admin_added_permissions: u64,
    pub admin_removed_permissions: u64,
    pub joined_at: DateTime<Utc>,
    pub is_online: bool,
    pub banned_at: Option<DateTime<Utc>>,
    pub banned_reason: Option<String>,
}

impl RoomMemberWithUser {
    /// Calculate effective permissions for display
    ///
    /// Arguments:
    /// - `role_default`: Already-calculated permissions for this role
    ///   (global default with room-level overrides applied)
    #[must_use] 
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
