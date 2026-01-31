use serde::{Deserialize, Serialize};

/// 64-bit permission bitmask
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PermissionBits(pub i64);

impl PermissionBits {
    // Room management permissions (bits 0-9)
    pub const CREATE_ROOM: i64 = 1 << 0;
    pub const DELETE_ROOM: i64 = 1 << 1;
    pub const UPDATE_ROOM_SETTINGS: i64 = 1 << 2;
    pub const INVITE_USER: i64 = 1 << 3;
    pub const KICK_USER: i64 = 1 << 4;

    // Playlist permissions (bits 10-19)
    pub const ADD_MEDIA: i64 = 1 << 10;
    pub const REMOVE_MEDIA: i64 = 1 << 11;
    pub const REORDER_PLAYLIST: i64 = 1 << 12;
    pub const SWITCH_MEDIA: i64 = 1 << 13;

    // Playback control permissions (bits 20-29)
    pub const PLAY_PAUSE: i64 = 1 << 20;
    pub const SEEK: i64 = 1 << 21;
    pub const CHANGE_SPEED: i64 = 1 << 22;

    // Chat permissions (bits 30-39)
    pub const SEND_CHAT: i64 = 1 << 30;
    pub const SEND_DANMAKU: i64 = 1 << 31;
    pub const DELETE_MESSAGE: i64 = 1 << 32;

    // Live streaming permissions (bits 40-49)
    pub const START_LIVE: i64 = 1 << 40;
    pub const STOP_LIVE: i64 = 1 << 41;

    // Admin permissions (bits 50-59)
    pub const GRANT_PERMISSION: i64 = 1 << 50;
    pub const REVOKE_PERMISSION: i64 = 1 << 51;

    // System admin permissions (bits 60-63)
    pub const SYSTEM_ADMIN: i64 = 1 << 60;

    pub const NONE: i64 = 0;
    pub const ALL: i64 = !0; // All bits set

    pub fn new(bits: i64) -> Self {
        Self(bits)
    }

    pub fn empty() -> Self {
        Self(Self::NONE)
    }

    pub fn has(&self, permission: i64) -> bool {
        (self.0 & permission) == permission
    }

    pub fn grant(&mut self, permission: i64) {
        self.0 |= permission;
    }

    pub fn revoke(&mut self, permission: i64) {
        self.0 &= !permission;
    }

    pub fn set(&mut self, permission: i64, enabled: bool) {
        if enabled {
            self.grant(permission);
        } else {
            self.revoke(permission);
        }
    }
}

impl Default for PermissionBits {
    fn default() -> Self {
        Self::empty()
    }
}

/// Role presets with permission inheritance
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Room creator - full control
    Creator,
    /// Room administrator - can manage most aspects
    Admin,
    /// Regular member - basic permissions
    Member,
    /// Guest - limited permissions
    Guest,
}

impl Role {
    pub fn permissions(&self) -> PermissionBits {
        match self {
            Role::Creator => {
                // Creator has all permissions
                PermissionBits(PermissionBits::ALL)
            }
            Role::Admin => {
                // Admin has most permissions except system admin
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
                perms.grant(PermissionBits::START_LIVE);
                perms.grant(PermissionBits::STOP_LIVE);
                perms.grant(PermissionBits::GRANT_PERMISSION);
                perms.grant(PermissionBits::REVOKE_PERMISSION);
                perms
            }
            Role::Member => {
                // Member has basic interaction permissions
                let mut perms = PermissionBits::empty();
                perms.grant(PermissionBits::ADD_MEDIA);
                perms.grant(PermissionBits::PLAY_PAUSE);
                perms.grant(PermissionBits::SEEK);
                perms.grant(PermissionBits::SEND_CHAT);
                perms.grant(PermissionBits::SEND_DANMAKU);
                perms
            }
            Role::Guest => {
                // Guest can only view and chat
                let mut perms = PermissionBits::empty();
                perms.grant(PermissionBits::SEND_CHAT);
                perms
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_has() {
        let mut perms = PermissionBits::empty();
        assert!(!perms.has(PermissionBits::SEND_CHAT));

        perms.grant(PermissionBits::SEND_CHAT);
        assert!(perms.has(PermissionBits::SEND_CHAT));
    }

    #[test]
    fn test_permission_grant_revoke() {
        let mut perms = PermissionBits::empty();
        perms.grant(PermissionBits::SEND_CHAT);
        perms.grant(PermissionBits::SEND_DANMAKU);

        assert!(perms.has(PermissionBits::SEND_CHAT));
        assert!(perms.has(PermissionBits::SEND_DANMAKU));

        perms.revoke(PermissionBits::SEND_CHAT);
        assert!(!perms.has(PermissionBits::SEND_CHAT));
        assert!(perms.has(PermissionBits::SEND_DANMAKU));
    }

    #[test]
    fn test_role_permissions() {
        let creator_perms = Role::Creator.permissions();
        assert!(creator_perms.has(PermissionBits::DELETE_ROOM));
        assert!(creator_perms.has(PermissionBits::SEND_CHAT));

        let member_perms = Role::Member.permissions();
        assert!(member_perms.has(PermissionBits::SEND_CHAT));
        assert!(!member_perms.has(PermissionBits::DELETE_ROOM));

        let guest_perms = Role::Guest.permissions();
        assert!(guest_perms.has(PermissionBits::SEND_CHAT));
        assert!(!guest_perms.has(PermissionBits::ADD_MEDIA));
    }
}
