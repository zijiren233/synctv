//! Member management service
//!
//! Handles room member operations including joining, leaving, kicking,
//! and role management.

use crate::{
    models::{Room, RoomId, RoomMember, RoomMemberWithUser, UserId, PermissionBits, Role},
    repository::{RoomMemberRepository, RoomRepository},
    service::permission::PermissionService,
    Error, Result,
};

/// Member management service
///
/// Responsible for all member-related operations within rooms.
#[derive(Clone)]
pub struct MemberService {
    member_repo: RoomMemberRepository,
    room_repo: RoomRepository,
    permission_service: PermissionService,
}

impl std::fmt::Debug for MemberService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberService").finish()
    }
}

impl MemberService {
    /// Create a new member service
    pub fn new(
        member_repo: RoomMemberRepository,
        room_repo: RoomRepository,
        permission_service: PermissionService,
    ) -> Self {
        Self {
            member_repo,
            room_repo,
            permission_service,
        }
    }

    /// Add a user as a member to a room
    pub async fn add_member(
        &self,
        room_id: RoomId,
        user_id: UserId,
        role: Role,
    ) -> Result<RoomMember> {
        // Check if room exists
        let room = self
            .room_repo
            .get_by_id(&room_id)
            .await?
            .ok_or_else(|| Error::NotFound("Room not found".to_string()))?;

        // Check if room is active
        if room.status != crate::models::RoomStatus::Active {
            return Err(Error::InvalidInput("Room is closed".to_string()));
        }

        // Check if already a member
        if self.member_repo.is_member(&room_id, &user_id).await? {
            return Err(Error::InvalidInput("Already a member of this room".to_string()));
        }

        // Check max members
        if let Some(max_members) = room.settings.get("max_members").and_then(|v| v.as_i64()) {
            let current_count = self.member_repo.count_by_room(&room_id).await?;
            if current_count >= max_members as i32 {
                return Err(Error::InvalidInput("Room is full".to_string()));
            }
        }

        // Add as member with role permissions
        let member = RoomMember::new(room_id.clone(), user_id, role.permissions());
        let created_member = self.member_repo.add(&member).await?;

        Ok(created_member)
    }

    /// Remove a member from a room
    pub async fn remove_member(&self, room_id: RoomId, user_id: UserId) -> Result<()> {
        // Check if member
        if !self.member_repo.is_member(&room_id, &user_id).await? {
            return Err(Error::NotFound("Not a member of this room".to_string()));
        }

        // Remove member
        self.member_repo.remove(&room_id, &user_id).await?;

        // Invalidate permission cache
        self.permission_service.invalidate_cache(&room_id, &user_id).await;

        Ok(())
    }

    /// Kick a member from a room (requires permission)
    pub async fn kick_member(
        &self,
        room_id: RoomId,
        kicker_id: UserId,
        target_user_id: UserId,
    ) -> Result<()> {
        // Check if kicker has permission to kick
        self.permission_service
            .check_permission(&room_id, &kicker_id, PermissionBits::KICK_USER)
            .await?;

        // Can't kick yourself
        if kicker_id == target_user_id {
            return Err(Error::InvalidInput("Cannot kick yourself".to_string()));
        }

        // Verify target is a member
        if !self.member_repo.is_member(&room_id, &target_user_id).await? {
            return Err(Error::NotFound("User is not a member of this room".to_string()));
        }

        // Remove member
        let removed = self.member_repo.remove(&room_id, &target_user_id).await?;
        if !removed {
            return Err(Error::NotFound("User is not a member of this room".to_string()));
        }

        // Invalidate permission cache for kicked user
        self.permission_service
            .invalidate_cache(&room_id, &target_user_id)
            .await;

        Ok(())
    }

    /// Update member permissions
    pub async fn update_member_permissions(
        &self,
        room_id: RoomId,
        granter_id: UserId,
        target_user_id: UserId,
        permissions: PermissionBits,
    ) -> Result<RoomMember> {
        // Check if granter has permission to modify permissions
        self.permission_service
            .check_permission(&room_id, &granter_id, PermissionBits::GRANT_PERMISSION)
            .await?;

        // Verify target is a member
        if !self.member_repo.is_member(&room_id, &target_user_id).await? {
            return Err(Error::NotFound("User is not a member of this room".to_string()));
        }

        // Update permissions
        let member = self
            .member_repo
            .update_permissions(&room_id, &target_user_id, permissions)
            .await?;

        // Invalidate permission cache for target user
        self.permission_service
            .invalidate_cache(&room_id, &target_user_id)
            .await;

        Ok(member)
    }

    /// Grant a specific permission to a member
    pub async fn grant_permission(
        &self,
        room_id: RoomId,
        granter_id: UserId,
        target_user_id: UserId,
        permission: i64,
    ) -> Result<RoomMember> {
        // Get current permissions
        let member = self
            .member_repo
            .get(&room_id, &target_user_id)
            .await?
            .ok_or_else(|| Error::NotFound("User is not a member of this room".to_string()))?;

        // Grant permission
        let mut permissions = member.permissions;
        permissions.grant(permission);

        // Update
        self.update_member_permissions(room_id, granter_id, target_user_id, permissions)
            .await
    }

    /// Revoke a specific permission from a member
    pub async fn revoke_permission(
        &self,
        room_id: RoomId,
        granter_id: UserId,
        target_user_id: UserId,
        permission: i64,
    ) -> Result<RoomMember> {
        // Get current permissions
        let member = self
            .member_repo
            .get(&room_id, &target_user_id)
            .await?
            .ok_or_else(|| Error::NotFound("User is not a member of this room".to_string()))?;

        // Revoke permission
        let mut permissions = member.permissions;
        permissions.revoke(permission);

        // Update
        self.update_member_permissions(room_id, granter_id, target_user_id, permissions)
            .await
    }

    /// Get all members of a room with user info
    pub async fn list_members(&self, room_id: &RoomId) -> Result<Vec<RoomMemberWithUser>> {
        self.member_repo.list_by_room(room_id).await
    }

    /// Get member count for a room
    pub async fn count_members(&self, room_id: &RoomId) -> Result<i32> {
        self.member_repo.count_by_room(room_id).await
    }

    /// Check if a user is a member of a room
    pub async fn is_member(&self, room_id: &RoomId, user_id: &UserId) -> Result<bool> {
        self.member_repo.is_member(room_id, user_id).await
    }

    /// Get a specific member
    pub async fn get_member(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<Option<RoomMember>> {
        self.member_repo.get(room_id, user_id).await
    }

    /// List all rooms a user is a member of
    pub async fn list_user_rooms(
        &self,
        user_id: &UserId,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<RoomId>, i64)> {
        self.member_repo.list_by_user(user_id, page, page_size).await
    }

    /// List all rooms a user is a member of with full details
    pub async fn list_user_rooms_with_details(
        &self,
        user_id: &UserId,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<(Room, PermissionBits, i32)>, i64)> {
        self.member_repo
            .list_by_user_with_details(user_id, page, page_size)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_add_member() {
        // Integration test placeholder
    }

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_kick_member() {
        // Integration test placeholder
    }
}
