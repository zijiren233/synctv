//! Permission management service
//!
//! Centralized permission checking and management with Allow/Deny pattern and caching.

use std::sync::Arc;
use std::time::Duration;

use crate::{
    models::{RoomId, UserId, PermissionBits, Room},
    repository::{RoomMemberRepository, RoomRepository},
    Error, Result,
};

/// Permission management service
///
/// Handles permission checking with Allow/Deny pattern, optional caching and role inheritance.
#[derive(Clone)]
pub struct PermissionService {
    member_repo: RoomMemberRepository,
    room_repo: RoomRepository,
    cache: Arc<moka::future::Cache<String, PermissionBits>>,
}

impl std::fmt::Debug for PermissionService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionService").finish()
    }
}

impl PermissionService {
    /// Create a new permission service with caching
    pub fn new(member_repo: RoomMemberRepository, room_repo: RoomRepository, cache_size: u64, cache_ttl_secs: u64) -> Self {
        Self {
            member_repo,
            room_repo,
            cache: Arc::new(
                moka::future::CacheBuilder::new(cache_size)
                    .time_to_live(Duration::from_secs(cache_ttl_secs))
                    .build(),
            ),
        }
    }

    /// Create a permission service without caching
    pub fn without_cache(member_repo: RoomMemberRepository, room_repo: RoomRepository) -> Self {
        Self {
            member_repo,
            room_repo,
            cache: Arc::new(
                moka::future::CacheBuilder::new(1)
                    .time_to_live(Duration::from_secs(1))
                    .build(),
            ),
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
        permission: i64,
    ) -> Result<()> {
        let permissions = self.get_user_permissions(room_id, user_id).await?;

        if !permissions.has(permission) {
            return Err(Error::Authorization("Permission denied".to_string()));
        }

        Ok(())
    }

    /// Get user's effective permissions in a room (with caching)
    ///
    /// This implements the Allow/Deny permission pattern:
    /// effective_permissions = (role_default | added) & ~removed
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
        let room = self
            .room_repo
            .get_by_id(room_id)
            .await?
            .ok_or_else(|| Error::NotFound("Room not found".to_string()))?;

        // Get room default permissions from settings
        let room_settings: serde_json::Value = serde_json::from_value(room.settings.clone())
            .unwrap_or(serde_json::json!({}));

        let room_default_permissions = match member.role {
            crate::models::RoomRole::Admin => {
                room_settings["default_admin_permissions"].as_i64()
            }
            crate::models::RoomRole::Member => {
                room_settings["default_member_permissions"].as_i64()
            }
            crate::models::RoomRole::Guest => {
                room_settings["guest_permissions"].as_i64()
            }
            crate::models::RoomRole::Creator => None, // Creator doesn't need defaults
        };

        // Calculate effective permissions
        let permissions = member.effective_permissions(room_default_permissions);

        // Update cache
        self.cache.insert(cache_key, permissions).await;

        Ok(permissions)
    }

    /// Invalidate cache for a specific user in a room
    pub async fn invalidate_cache(&self, room_id: &RoomId, user_id: &UserId) {
        let cache_key = Self::cache_key(room_id, user_id);
        self.cache.invalidate(&cache_key).await;
    }

    /// Clear all permission cache
    pub async fn clear_cache(&self) {
        self.cache.invalidate_all();
    }

    /// Check if user can perform an action (alias for check_permission)
    pub async fn can(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        permission: i64,
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
        permissions: &[i64],
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

        Ok(member.map(|m| m.role == crate::models::RoomRole::Creator).unwrap_or(false))
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

        Ok(member.map(|m| matches!(m.role, crate::models::RoomRole::Admin | crate::models::RoomRole::Creator)).unwrap_or(false))
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
