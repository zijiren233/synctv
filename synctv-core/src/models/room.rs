use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::fmt::Display;

use super::id::{RoomId, UserId};
use super::permission::PermissionBits;
use crate::Error;

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
#[derive(Default)]
pub enum PlayMode {
    /// Sequential play (stop after last item)
    #[default]
    Sequential,
    /// Repeat single item
    RepeatOne,
    /// Repeat all items (loop back to start)
    RepeatAll,
    /// Random playback
    Shuffle,
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Room {
    pub fn new(name: String, created_by: UserId) -> Self {
        let now = Utc::now();
        Self {
            id: RoomId::new(),
            name,
            created_by,
            status: RoomStatus::Active,
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

/// Room with settings loaded from room_settings table
#[derive(Debug, Clone)]
pub struct RoomWithSettings {
    pub room: Room,
    pub settings: RoomSettings,
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

    // ===== Permission Override Configuration =====
    //
    // Rooms can override global default permissions from SettingsRegistry
    // Each role has added/removed permissions that modify the global defaults
    // Formula: (global_default | added) & ~removed

    /// Additional permissions for Admin role (on top of global default)
    #[serde(default)]
    pub admin_added_permissions: Option<i64>,

    /// Removed permissions for Admin role (overrides global default)
    #[serde(default)]
    pub admin_removed_permissions: Option<i64>,

    /// Additional permissions for Member role (on top of global default)
    #[serde(default)]
    pub member_added_permissions: Option<i64>,

    /// Removed permissions for Member role (overrides global default)
    #[serde(default)]
    pub member_removed_permissions: Option<i64>,

    /// Additional permissions for Guests (on top of global default)
    #[serde(default)]
    pub guest_added_permissions: Option<i64>,

    /// Removed permissions for Guests (overrides global default)
    #[serde(default)]
    pub guest_removed_permissions: Option<i64>,

    /// Whether room requires approval for new members
    #[serde(default)]
    pub require_approval: bool,

    /// Whether members can auto-join (without invitation)
    #[serde(default = "default_true")]
    pub allow_auto_join: bool,
}

impl RoomSettings {
    /// Calculate effective permissions for a role based on global defaults and room overrides
    ///
    /// Formula: (global_default | added) & ~removed
    ///
    /// Arguments:
    /// - global_default: Default permissions from global settings
    /// - added_permissions: Additional permissions from room settings (Optional)
    /// - removed_permissions: Removed permissions from room settings (Optional)
    pub fn effective_permissions_for_role(
        global_default: PermissionBits,
        added_permissions: Option<i64>,
        removed_permissions: Option<i64>,
    ) -> PermissionBits {
        let mut result = global_default.0;

        // Add extra permissions
        if let Some(added) = added_permissions {
            result |= added;
        }

        // Remove permissions
        if let Some(removed) = removed_permissions {
            result &= !removed;
        }

        PermissionBits(result)
    }

    /// Get effective permissions for Admin role
    ///
    /// Requires global default admin permissions from SettingsRegistry
    pub fn admin_permissions(&self, global_default: PermissionBits) -> PermissionBits {
        Self::effective_permissions_for_role(
            global_default,
            self.admin_added_permissions,
            self.admin_removed_permissions,
        )
    }

    /// Get effective permissions for Member role
    ///
    /// Requires global default member permissions from SettingsRegistry
    pub fn member_permissions(&self, global_default: PermissionBits) -> PermissionBits {
        Self::effective_permissions_for_role(
            global_default,
            self.member_added_permissions,
            self.member_removed_permissions,
        )
    }

    /// Get effective permissions for Guest
    ///
    /// Requires global default guest permissions from SettingsRegistry
    pub fn guest_permissions(&self, global_default: PermissionBits) -> PermissionBits {
        Self::effective_permissions_for_role(
            global_default,
            self.guest_added_permissions,
            self.guest_removed_permissions,
        )
    }
}

fn default_true() -> bool {
    true
}

/// Room with member count (for efficient queries with JOIN)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomWithCount {
    #[serde(flatten)]
    pub room: Room,
    pub member_count: i32,
}

// ==================== Trait Implementations for Settings System ====================

impl Display for PlayMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayMode::Sequential => write!(f, "sequential"),
            PlayMode::RepeatOne => write!(f, "repeat_one"),
            PlayMode::RepeatAll => write!(f, "repeat_all"),
            PlayMode::Shuffle => write!(f, "shuffle"),
        }
    }
}

impl std::str::FromStr for PlayMode {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "sequential" => Ok(PlayMode::Sequential),
            "repeat_one" => Ok(PlayMode::RepeatOne),
            "repeat_all" => Ok(PlayMode::RepeatAll),
            "shuffle" => Ok(PlayMode::Shuffle),
            _ => Err(Error::InvalidInput(format!("Invalid PlayMode: {}", s))),
        }
    }
}

impl Display for AutoPlaySettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Use JSON representation for complex types
        let json = serde_json::to_string(self)
            .map_err(|_| std::fmt::Error)?;
        write!(f, "{}", json)
    }
}

impl std::str::FromStr for AutoPlaySettings {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s)
            .map_err(|e| Error::InvalidInput(format!("Invalid AutoPlaySettings: {}", e)))
    }
}

impl Display for RoomSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Use JSON representation for the entire settings struct
        let json = serde_json::to_string(self)
            .map_err(|_| std::fmt::Error)?;
        write!(f, "{}", json)
    }
}

impl std::str::FromStr for RoomSettings {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s)
            .map_err(|e| Error::InvalidInput(format!("Invalid RoomSettings: {}", e)))
    }
}
