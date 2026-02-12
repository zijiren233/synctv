//! Permission management service
//!
//! Centralized permission checking and management with Allow/Deny pattern and caching.

use std::sync::Arc;
use std::time::Duration;

use crate::{
    models::{RoomId, UserId, PermissionBits, RoomSettings},
    repository::{RoomMemberRepository, RoomRepository, RoomSettingsRepository},
    service::SettingsRegistry,
    Error, Result,
};

/// Permission management service
///
/// Handles permission checking with Allow/Deny pattern, optional caching and role inheritance.
#[derive(Clone)]
pub struct PermissionService {
    member_repo: RoomMemberRepository,
    room_settings_repo: Option<RoomSettingsRepository>,
    cache: Arc<moka::future::Cache<String, PermissionBits>>,
    settings_registry: Option<Arc<SettingsRegistry>>,
}

impl std::fmt::Debug for PermissionService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionService").finish()
    }
}

impl PermissionService {
    /// Default permission cache capacity (max entries)
    pub const DEFAULT_CACHE_SIZE: u64 = 10_000;
    /// Default permission cache TTL in seconds (5 minutes)
    pub const DEFAULT_CACHE_TTL_SECS: u64 = 300;

    /// Create a new permission service with caching
    #[must_use]
    pub fn new(
        member_repo: RoomMemberRepository,
        _room_repo: RoomRepository,
        settings_registry: Option<Arc<SettingsRegistry>>,
        cache_size: u64,
        cache_ttl_secs: u64,
    ) -> Self {
        Self {
            member_repo,
            room_settings_repo: None, // Will be set later if needed
            cache: Arc::new(
                moka::future::CacheBuilder::new(cache_size)
                    .time_to_live(Duration::from_secs(cache_ttl_secs))
                    .build(),
            ),
            settings_registry,
        }
    }

    /// Create a permission service without caching
    #[must_use] 
    pub fn without_cache(
        member_repo: RoomMemberRepository,
        _room_repo: RoomRepository,
        settings_registry: Option<Arc<SettingsRegistry>>,
    ) -> Self {
        Self {
            member_repo,
            room_settings_repo: None,
            cache: Arc::new(
                moka::future::CacheBuilder::new(1)
                    .time_to_live(Duration::from_secs(1))
                    .build(),
            ),
            settings_registry,
        }
    }

    /// Set the room settings repository
    pub fn set_room_settings_repo(&mut self, repo: RoomSettingsRepository) {
        self.room_settings_repo = Some(repo);
    }

    /// Get global default permissions for a role from `SettingsRegistry`
    fn get_global_default_permissions(&self, role: &crate::models::RoomRole) -> PermissionBits {
        if let Some(registry) = &self.settings_registry {
            match role {
                crate::models::RoomRole::Admin => {
                    PermissionBits(registry.admin_default_permissions.get().unwrap_or(PermissionBits::DEFAULT_ADMIN))
                }
                crate::models::RoomRole::Member => {
                    PermissionBits(registry.member_default_permissions.get().unwrap_or(PermissionBits::DEFAULT_MEMBER))
                }
                crate::models::RoomRole::Guest => {
                    PermissionBits(registry.guest_default_permissions.get().unwrap_or(PermissionBits::DEFAULT_GUEST))
                }
                crate::models::RoomRole::Creator => PermissionBits(crate::models::PermissionBits::ALL),
            }
        } else {
            // Fallback to hardcoded defaults if SettingsRegistry not available
            match role {
                crate::models::RoomRole::Admin => {
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
                    perms.grant(PermissionBits::DELETE_MESSAGE);
                    perms.grant(PermissionBits::GRANT_PERMISSION);
                    perms.grant(PermissionBits::REVOKE_PERMISSION);
                    perms
                }
                crate::models::RoomRole::Member => {
                    let mut perms = PermissionBits::empty();
                    perms.grant(PermissionBits::ADD_MEDIA);
                    perms.grant(PermissionBits::PLAY_PAUSE);
                    perms.grant(PermissionBits::SEEK);
                    perms.grant(PermissionBits::SEND_CHAT);
                    perms
                }
                crate::models::RoomRole::Guest => PermissionBits(crate::models::PermissionBits::VIEW_PLAYLIST),
                crate::models::RoomRole::Creator => PermissionBits(crate::models::PermissionBits::ALL),
            }
        }
    }

    /// Calculate role default permissions with room-level overrides applied
    ///
    /// This combines:
    /// 1. Global default permissions (from `SettingsRegistry`)
    /// 2. Room-level overrides: (global | `room_added`) & ~`room_removed`
    #[must_use] 
    pub fn calculate_role_default_permissions(
        &self,
        role: &crate::models::RoomRole,
        room_settings: &RoomSettings,
    ) -> PermissionBits {
        let global_default = self.get_global_default_permissions(role);

        match role {
            crate::models::RoomRole::Creator => PermissionBits(crate::models::PermissionBits::ALL),
            crate::models::RoomRole::Admin => {
                room_settings.admin_permissions(global_default)
            }
            crate::models::RoomRole::Member => {
                room_settings.member_permissions(global_default)
            }
            crate::models::RoomRole::Guest => {
                room_settings.guest_permissions(global_default)
            }
        }
    }

    /// Generate cache key for room + user
    fn cache_key(room_id: &RoomId, user_id: &UserId) -> String {
        format!("{}:{}", room_id.0, user_id.0)
    }

    /// Check if a user has a specific permission in a room
    pub async fn check_permission(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        permission: u64,
    ) -> Result<()> {
        let permissions = self.get_user_permissions(room_id, user_id).await?;

        if !permissions.has_all(permission) {
            return Err(Error::Authorization("Permission denied".to_string()));
        }

        Ok(())
    }

    /// Get user's effective permissions in a room (with caching)
    ///
    /// This implements the Allow/Deny permission pattern:
    /// `effective_permissions` = (`role_default` | added) & ~removed
    pub async fn get_user_permissions(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<PermissionBits> {
        let cache_key = Self::cache_key(room_id, user_id);

        // Check cache first
        if let Some(permissions) = self.cache.get(&cache_key).await {
            return Ok(permissions);
        }

        // Fetch from database
        let member = self
            .member_repo
            .get(room_id, user_id)
            .await?
            .ok_or_else(|| Error::Authorization("Not a member of this room".to_string()))?;

        // Get room settings for role defaults
        let room_settings = if let Some(ref settings_repo) = self.room_settings_repo {
            settings_repo.get(room_id).await?
        } else {
            RoomSettings::default()
        };

        // Calculate role default permissions (global + room-level overrides)
        let role_default = self.calculate_role_default_permissions(&member.role, &room_settings);

        // Apply member-level overrides
        let permissions = member.effective_permissions(role_default);

        // Update cache
        self.cache.insert(cache_key, permissions).await;

        Ok(permissions)
    }

    /// Invalidate cache for a specific user in a room
    pub async fn invalidate_cache(&self, room_id: &RoomId, user_id: &UserId) {
        let cache_key = Self::cache_key(room_id, user_id);
        self.cache.invalidate(&cache_key).await;
    }

    /// Invalidate permission cache for all users in a room.
    /// Called when room-level permission settings change (e.g., admin/member/guest
    /// added/removed permissions), since these affect all members' effective permissions.
    pub async fn invalidate_room_cache(&self, room_id: &RoomId) {
        let prefix = format!("{}:", room_id.0);
        let _ = self.cache.invalidate_entries_if(move |key, _| key.starts_with(&prefix));
    }

    /// Clear all permission cache
    pub async fn clear_cache(&self) {
        self.cache.invalidate_all();
    }

    /// Check if user can perform an action (alias for `check_permission`)
    pub async fn can(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        permission: u64,
    ) -> Result<bool> {
        match self.check_permission(room_id, user_id, permission).await {
            Ok(()) => Ok(true),
            Err(Error::Authorization(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Check multiple permissions at once
    pub async fn check_permissions(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        permissions: &[u64],
    ) -> Result<()> {
        let user_permissions = self.get_user_permissions(room_id, user_id).await?;

        for &permission in permissions {
            if !user_permissions.has(permission) {
                return Err(Error::Authorization("Permission denied".to_string()));
            }
        }

        Ok(())
    }

    /// Check if user has a specific role in room
    pub async fn check_role(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        expected_role: crate::models::RoomRole,
    ) -> Result<()> {
        let member = self
            .member_repo
            .get(room_id, user_id)
            .await?
            .ok_or_else(|| Error::Authorization("Not a member of this room".to_string()))?;

        if member.role != expected_role {
            return Err(Error::Authorization("Insufficient permissions".to_string()));
        }

        Ok(())
    }

    /// Check if user is room creator
    pub async fn is_creator(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<bool> {
        let member = self
            .member_repo
            .get(room_id, user_id)
            .await?;

        Ok(member.is_some_and(|m| m.role == crate::models::RoomRole::Creator))
    }

    /// Check if user is room admin or creator
    pub async fn is_admin_or_creator(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<bool> {
        let member = self
            .member_repo
            .get(room_id, user_id)
            .await?;

        Ok(member.is_some_and(|m| matches!(m.role, crate::models::RoomRole::Admin | crate::models::RoomRole::Creator)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_generation() {
        let room_id = RoomId("room123".to_string());
        let user_id = UserId("user456".to_string());
        let key = PermissionService::cache_key(&room_id, &user_id);
        assert_eq!(key, "room123:user456");
    }
}
