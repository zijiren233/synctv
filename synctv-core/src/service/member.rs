//! Member management service
//!
//! Handles room member operations including joining, leaving, kicking,
//! and role management with Allow/Deny permission pattern.

use crate::{
    models::{Room, RoomId, RoomMember, RoomMemberWithUser, UserId, PermissionBits, RoomRole, MemberStatus, RoomSettings},
    repository::{RoomMemberRepository, RoomRepository, RoomSettingsRepository},
    service::permission::PermissionService,
    Error, Result,
};

/// Options for adding a member to a room
///
/// # Examples
///
/// ```rust,ignore
/// // Default options (all checks enabled)
/// let options = AddMemberOptions::new();
///
/// // Skip max members check
/// let options = AddMemberOptions::new().skip_max_members_check();
///
/// // Set custom max members limit
/// let options = AddMemberOptions::new().with_max_members(100);
///
/// // Skip cache invalidation
/// let options = AddMemberOptions::new().skip_cache_invalidation();
///
/// // Combine options
/// let options = AddMemberOptions::new()
///     .skip_max_members_check()
///     .skip_cache_invalidation();
/// ```
#[derive(Debug, Clone, Default)]
pub struct AddMemberOptions {
    /// Check if room is active
    pub check_room_active: bool,
    /// Check for duplicate membership
    pub check_duplicate: bool,
    /// Check max members limit
    pub check_max_members: bool,
    /// Maximum number of members allowed (0 = no limit)
    pub max_members: u64,
    /// Invalidate permission cache after adding
    pub invalidate_cache: bool,
}

impl AddMemberOptions {
    /// Create default options (all checks enabled, no max limit)
    #[must_use] 
    pub const fn new() -> Self {
        Self {
            check_room_active: true,
            check_duplicate: true,
            check_max_members: false,  // disabled by default
            max_members: 0,           // 0 means no limit
            invalidate_cache: true,
        }
    }

    /// Set max members limit (enables the check)
    #[must_use] 
    pub const fn with_max_members(mut self, max: u64) -> Self {
        self.max_members = max;
        self.check_max_members = true;
        self
    }

    /// Skip max members check
    #[must_use] 
    pub const fn skip_max_members_check(mut self) -> Self {
        self.check_max_members = false;
        self
    }

    /// Skip room active check
    #[must_use] 
    pub const fn skip_active_check(mut self) -> Self {
        self.check_room_active = false;
        self
    }

    /// Skip duplicate membership check
    #[must_use] 
    pub const fn skip_duplicate_check(mut self) -> Self {
        self.check_duplicate = false;
        self
    }

    /// Skip cache invalidation
    #[must_use] 
    pub const fn skip_cache_invalidation(mut self) -> Self {
        self.invalidate_cache = false;
        self
    }
}

/// Member management service
///
/// Responsible for all member-related operations within rooms.
#[derive(Clone)]
pub struct MemberService {
    member_repo: RoomMemberRepository,
    room_repo: RoomRepository,
    room_settings_repo: Option<RoomSettingsRepository>,
    permission_service: PermissionService,
}

impl std::fmt::Debug for MemberService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberService").finish()
    }
}

impl MemberService {
    /// Create a new member service
    #[must_use] 
    pub const fn new(
        member_repo: RoomMemberRepository,
        room_repo: RoomRepository,
        permission_service: PermissionService,
    ) -> Self {
        Self {
            member_repo,
            room_repo,
            room_settings_repo: None,
            permission_service,
        }
    }

    /// Set the room settings repository
    pub fn set_room_settings_repo(&mut self, repo: RoomSettingsRepository) {
        self.room_settings_repo = Some(repo);
    }

    /// Add a user as a member to a room (with default options)
    ///
    /// This is a convenience method that uses default options.
    pub async fn add_member(
        &self,
        room_id: RoomId,
        user_id: UserId,
        role: RoomRole,
    ) -> Result<RoomMember> {
        self.add_member_with_options(room_id, user_id, role, AddMemberOptions::new())
            .await
    }

    /// Add a user as a member to a room with custom options
    ///
    /// This method uses a database transaction to perform all checks and the insert atomically.
    pub async fn add_member_with_options(
        &self,
        room_id: RoomId,
        user_id: UserId,
        role: RoomRole,
        mut options: AddMemberOptions,
    ) -> Result<RoomMember> {
        // Get room settings and apply to options if max_members check is enabled
        if options.check_max_members {
            let room_settings = if let Some(ref settings_repo) = self.room_settings_repo {
                settings_repo.get(&room_id).await?
            } else {
                RoomSettings::default()
            };

            options.max_members = room_settings.max_members.0;
        }

        // Create member object
        let member = RoomMember::new(room_id.clone(), user_id.clone(), role);

        // Add member with options (transaction happens in repository)
        let created_member = self
            .member_repo
            .add_with_options(&member, &options)
            .await?;

        // Invalidate permission cache (outside transaction)
        if options.invalidate_cache {
            self.permission_service
                .invalidate_cache(&room_id, &user_id)
                .await;
        }

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

    /// Set member Allow/Deny permissions
    ///
    /// This implements the Allow/Deny pattern:
    /// - `added_permissions`: Extra permissions to add to role default
    /// - `removed_permissions`: Permissions to remove from role default
    pub async fn set_member_permissions(
        &self,
        room_id: RoomId,
        granter_id: UserId,
        target_user_id: UserId,
        added_permissions: u64,
        removed_permissions: u64,
    ) -> Result<RoomMember> {
        // Check if granter has permission to modify permissions
        self.permission_service
            .check_permission(&room_id, &granter_id, PermissionBits::GRANT_PERMISSION)
            .await?;

        // Get current member to get version
        let member = self
            .member_repo
            .get(&room_id, &target_user_id)
            .await?
            .ok_or_else(|| Error::NotFound("User is not a member of this room".to_string()))?;

        // Update permissions with optimistic locking
        let updated_member = self
            .member_repo
            .update_permissions(&room_id, &target_user_id, added_permissions, removed_permissions, member.version)
            .await?;

        // Invalidate permission cache for target user
        self.permission_service
            .invalidate_cache(&room_id, &target_user_id)
            .await;

        Ok(updated_member)
    }

    /// Grant a specific permission to a member (Allow pattern)
    pub async fn grant_permission(
        &self,
        room_id: RoomId,
        granter_id: UserId,
        target_user_id: UserId,
        permission: u64,
    ) -> Result<RoomMember> {
        // Get current member
        let member = self
            .member_repo
            .get(&room_id, &target_user_id)
            .await?
            .ok_or_else(|| Error::NotFound("User is not a member of this room".to_string()))?;

        // Add to existing added_permissions
        let new_added = member.added_permissions | permission;

        // Update
        self.set_member_permissions(room_id, granter_id, target_user_id, new_added, member.removed_permissions)
            .await
    }

    /// Revoke a specific permission from a member (Deny pattern)
    pub async fn revoke_permission(
        &self,
        room_id: RoomId,
        granter_id: UserId,
        target_user_id: UserId,
        permission: u64,
    ) -> Result<RoomMember> {
        // Get current member
        let member = self
            .member_repo
            .get(&room_id, &target_user_id)
            .await?
            .ok_or_else(|| Error::NotFound("User is not a member of this room".to_string()))?;

        // Add to existing removed_permissions
        let new_removed = member.removed_permissions | permission;

        // Update
        self.set_member_permissions(room_id, granter_id, target_user_id, member.added_permissions, new_removed)
            .await
    }

    /// Reset member permissions to role default (clear Allow/Deny)
    pub async fn reset_member_permissions(
        &self,
        room_id: RoomId,
        granter_id: UserId,
        target_user_id: UserId,
    ) -> Result<RoomMember> {
        // Check if granter has permission to modify permissions
        self.permission_service
            .check_permission(&room_id, &granter_id, PermissionBits::GRANT_PERMISSION)
            .await?;

        // Get current member to get version
        let member = self
            .member_repo
            .get(&room_id, &target_user_id)
            .await?
            .ok_or_else(|| Error::NotFound("User is not a member of this room".to_string()))?;

        // Reset to role default (clear both added and removed)
        let updated_member = self
            .member_repo
            .reset_permissions(&room_id, &target_user_id, member.version)
            .await?;

        // Invalidate permission cache for target user
        self.permission_service
            .invalidate_cache(&room_id, &target_user_id)
            .await;

        Ok(updated_member)
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

    /// Check if a user is banned from a room
    pub async fn is_banned(&self, room_id: &RoomId, user_id: &UserId) -> Result<bool> {
        self.member_repo.is_banned(room_id, user_id).await
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
    ) -> Result<(Vec<(Room, RoomRole, MemberStatus, i32)>, i64)> {
        self.member_repo
            .list_by_user_with_details(user_id, page, page_size)
            .await
    }

    /// Ban a member from a room
    pub async fn ban_member(
        &self,
        room_id: RoomId,
        admin_id: UserId,
        target_user_id: UserId,
        reason: Option<String>,
    ) -> Result<()> {
        // Check admin permission
        self.permission_service
            .check_permission(&room_id.clone(), &admin_id, PermissionBits::KICK_USER)
            .await?;

        // Ban member
        self.member_repo
            .ban_member(&room_id, &target_user_id, &admin_id, reason)
            .await?;

        // Invalidate permission cache for banned user
        self.permission_service
            .invalidate_cache(&room_id, &target_user_id)
            .await;

        Ok(())
    }

    /// Unban a member from a room
    pub async fn unban_member(
        &self,
        room_id: RoomId,
        admin_id: UserId,
        target_user_id: UserId,
    ) -> Result<()> {
        // Check admin permission
        self.permission_service
            .check_permission(&room_id.clone(), &admin_id, PermissionBits::KICK_USER)
            .await?;

        // Unban member
        self.member_repo
            .unban_member(&room_id, &target_user_id)
            .await?;

        // Invalidate permission cache for unbanned user
        self.permission_service
            .invalidate_cache(&room_id, &target_user_id)
            .await;

        Ok(())
    }

    /// Set member role (member/admin/creator)
    pub async fn set_member_role(
        &self,
        room_id: RoomId,
        creator_id: UserId,
        target_user_id: UserId,
        role: RoomRole,
    ) -> Result<RoomMember> {
        // Check if user is creator (only creator can change roles)
        let room = self
            .room_repo
            .get_by_id(&room_id)
            .await?
            .ok_or_else(|| Error::NotFound("Room not found".to_string()))?;

        if room.created_by != creator_id {
            return Err(Error::Authorization(
                "Only room creator can change member roles".to_string(),
            ));
        }

        // Verify target is a member
        let member = self
            .member_repo
            .get(&room_id, &target_user_id)
            .await?
            .ok_or_else(|| Error::NotFound("User is not a member of this room".to_string()))?;

        // Update role with optimistic locking
        let updated_member = self
            .member_repo
            .update_role(&room_id, &target_user_id, role, member.version)
            .await?;

        // Invalidate permission cache
        self.permission_service
            .invalidate_cache(&room_id, &target_user_id)
            .await;

        Ok(updated_member)
    }

    /// Set member status (active/pending/banned)
    pub async fn set_member_status(
        &self,
        room_id: RoomId,
        admin_id: UserId,
        target_user_id: UserId,
        status: MemberStatus,
    ) -> Result<RoomMember> {
        // Check admin permission
        self.permission_service
            .check_permission(&room_id.clone(), &admin_id, PermissionBits::KICK_USER)
            .await?;

        // Get current member
        let member = self
            .member_repo
            .get(&room_id, &target_user_id)
            .await?
            .ok_or_else(|| Error::NotFound("User is not a member of this room".to_string()))?;

        // Update status with optimistic locking
        let updated_member = self
            .member_repo
            .update_status(&room_id, &target_user_id, status, member.version)
            .await?;

        // Invalidate permission cache
        self.permission_service
            .invalidate_cache(&room_id, &target_user_id)
            .await;

        Ok(updated_member)
    }

    /// List all members including inactive (left) (admin view)
    pub async fn list_members_all(
        &self,
        room_id: &RoomId,
        admin_id: UserId,
    ) -> Result<Vec<RoomMemberWithUser>> {
        // Check admin permission
        self.permission_service
            .check_permission(&room_id.clone(), &admin_id, PermissionBits::KICK_USER)
            .await?;

        // Get all members regardless of left_at status
        self.member_repo.list_by_room_all(room_id).await
    }
}

#[cfg(test)]
mod tests {

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
