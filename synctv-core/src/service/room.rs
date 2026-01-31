use sqlx::PgPool;
use serde_json::json;

use crate::{
    models::{
        Room, RoomId, RoomMember, RoomMemberWithUser, RoomSettings, RoomStatus, UserId,
        PermissionBits, Role, RoomPlaybackState, Media, MediaId, ProviderType,
        RoomListQuery,
    },
    repository::{RoomRepository, RoomMemberRepository, MediaRepository, RoomPlaybackStateRepository},
    service::auth::password::{hash_password, verify_password},
    Error, Result,
};

/// Room service for business logic
#[derive(Clone)]
pub struct RoomService {
    room_repo: RoomRepository,
    member_repo: RoomMemberRepository,
    media_repo: MediaRepository,
    playback_repo: RoomPlaybackStateRepository,
}

impl std::fmt::Debug for RoomService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoomService").finish()
    }
}

impl RoomService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            room_repo: RoomRepository::new(pool.clone()),
            member_repo: RoomMemberRepository::new(pool.clone()),
            media_repo: MediaRepository::new(pool.clone()),
            playback_repo: RoomPlaybackStateRepository::new(pool),
        }
    }

    /// Create a new room
    pub async fn create_room(
        &self,
        name: String,
        created_by: UserId,
        password: Option<String>,
        settings: Option<RoomSettings>,
    ) -> Result<(Room, RoomMember)> {
        // Validate room name
        if name.is_empty() {
            return Err(Error::InvalidInput("Room name cannot be empty".to_string()));
        }
        if name.len() > 255 {
            return Err(Error::InvalidInput("Room name too long".to_string()));
        }

        // Build settings
        let mut room_settings = settings.unwrap_or_default();
        room_settings.require_password = password.is_some();

        // Hash password if provided
        let hashed_password = if let Some(pwd) = password {
            Some(hash_password(&pwd).await?)
        } else {
            None
        };

        let settings_json = json!({
            "require_password": room_settings.require_password,
            "password": hashed_password,
            "auto_play_next": room_settings.auto_play_next,
            "loop_playlist": room_settings.loop_playlist,
            "shuffle_playlist": room_settings.shuffle_playlist,
            "allow_guest_join": room_settings.allow_guest_join,
            "max_members": room_settings.max_members,
            "chat_enabled": room_settings.chat_enabled,
            "danmaku_enabled": room_settings.danmaku_enabled,
        });

        // Create room
        let room = Room::new(name, created_by.clone(), settings_json);
        let created_room = self.room_repo.create(&room).await?;

        // Add creator as member with full permissions
        let member = RoomMember::new(
            created_room.id.clone(),
            created_by,
            Role::Creator.permissions(),
        );
        let created_member = self.member_repo.add(&member).await?;

        // Initialize playback state
        self.playback_repo.create_or_get(&created_room.id).await?;

        Ok((created_room, created_member))
    }

    /// Join a room
    pub async fn join_room(
        &self,
        room_id: RoomId,
        user_id: UserId,
        password: Option<String>,
    ) -> Result<(Room, RoomMember, Vec<crate::models::RoomMemberWithUser>)> {
        // Get room
        let room = self
            .room_repo
            .get_by_id(&room_id)
            .await?
            .ok_or_else(|| Error::NotFound("Room not found".to_string()))?;

        // Check if room is active
        if room.status != RoomStatus::Active {
            return Err(Error::InvalidInput("Room is closed".to_string()));
        }

        // Check password if required
        if let Some(password_hash) = room.settings.get("password").and_then(|v| v.as_str()) {
            let provided_password = password.ok_or_else(|| Error::Authorization("Password required".to_string()))?;

            // Verify password using Argon2id
            let is_valid = verify_password(&provided_password, password_hash).await?;
            if !is_valid {
                return Err(Error::Authorization("Invalid password".to_string()));
            }
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

        // Add as member with default member permissions
        let member = RoomMember::new(room_id.clone(), user_id, Role::Member.permissions());
        let created_member = self.member_repo.add(&member).await?;

        // Get all members
        let members = self.member_repo.list_by_room(&room_id).await?;

        Ok((room, created_member, members))
    }

    /// Leave a room
    pub async fn leave_room(&self, room_id: RoomId, user_id: UserId) -> Result<()> {
        // Check if member
        if !self.member_repo.is_member(&room_id, &user_id).await? {
            return Err(Error::NotFound("Not a member of this room".to_string()));
        }

        // Remove member
        self.member_repo.remove(&room_id, &user_id).await?;

        Ok(())
    }

    /// Delete a room (creator only)
    pub async fn delete_room(&self, room_id: RoomId, user_id: UserId) -> Result<()> {
        // Check permission
        self.check_permission(&room_id, &user_id, PermissionBits::DELETE_ROOM).await?;

        // Delete room
        self.room_repo.delete(&room_id).await?;

        Ok(())
    }

    /// Update room settings
    pub async fn update_settings(
        &self,
        room_id: RoomId,
        user_id: UserId,
        settings: RoomSettings,
    ) -> Result<Room> {
        // Check permission
        self.check_permission(&room_id, &user_id, PermissionBits::UPDATE_ROOM_SETTINGS).await?;

        // Get room
        let mut room = self
            .room_repo
            .get_by_id(&room_id)
            .await?
            .ok_or_else(|| Error::NotFound("Room not found".to_string()))?;

        // Update settings
        room.settings = json!({
            "auto_play_next": settings.auto_play_next,
            "loop_playlist": settings.loop_playlist,
            "shuffle_playlist": settings.shuffle_playlist,
            "allow_guest_join": settings.allow_guest_join,
            "max_members": settings.max_members,
            "chat_enabled": settings.chat_enabled,
            "danmaku_enabled": settings.danmaku_enabled,
        });

        // Save
        let updated_room = self.room_repo.update(&room).await?;

        Ok(updated_room)
    }

    /// Add movie to playlist
    pub async fn add_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        url: String,
        provider: ProviderType,
        title: String,
    ) -> Result<Media> {
        // Check permission
        self.check_permission(&room_id, &user_id, PermissionBits::ADD_MEDIA).await?;

        // Get next position
        let position = self.media_repo.get_next_position(&room_id).await?;

        // Create movie
        let movie = Media::new(
            room_id,
            url,
            provider,
            title,
            json!({}),
            position,
            user_id,
        );

        let created_movie = self.media_repo.create(&movie).await?;

        Ok(created_movie)
    }

    /// Remove movie from playlist
    pub async fn remove_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        media_id: MediaId,
    ) -> Result<()> {
        // Check permission
        self.check_permission(&room_id, &user_id, PermissionBits::REMOVE_MEDIA).await?;

        // Delete movie
        self.media_repo.delete(&media_id).await?;

        Ok(())
    }

    /// Get playlist
    pub async fn get_playlist(&self, room_id: RoomId) -> Result<Vec<Media>> {
        self.media_repo.get_playlist(&room_id).await
    }

    /// Update playback state (play/pause/seek/etc)
    pub async fn update_playback(
        &self,
        room_id: RoomId,
        user_id: UserId,
        update_fn: impl FnOnce(&mut RoomPlaybackState),
        required_permission: i64,
    ) -> Result<RoomPlaybackState> {
        // Check permission
        self.check_permission(&room_id, &user_id, required_permission).await?;

        // Get current state
        let mut state = self
            .playback_repo
            .create_or_get(&room_id)
            .await?;

        // Apply update
        update_fn(&mut state);

        // Save with optimistic locking
        let updated_state = self.playback_repo.update(&state).await?;

        Ok(updated_state)
    }

    /// Check if user has permission in room
    pub async fn check_permission(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        permission: i64,
    ) -> Result<()> {
        let member = self
            .member_repo
            .get(room_id, user_id)
            .await?
            .ok_or_else(|| Error::Authorization("Not a member of this room".to_string()))?;

        if !member.has_permission(permission) {
            return Err(Error::Authorization("Permission denied".to_string()));
        }

        Ok(())
    }

    /// Get room with details
    pub async fn get_room(&self, room_id: &RoomId) -> Result<Room> {
        self.room_repo
            .get_by_id(room_id)
            .await?
            .ok_or_else(|| Error::NotFound("Room not found".to_string()))
    }

    /// Get playback state
    pub async fn get_playback_state(&self, room_id: &RoomId) -> Result<RoomPlaybackState> {
        self.playback_repo
            .create_or_get(room_id)
            .await
    }

    /// Grant permission to user
    pub async fn grant_permission(
        &self,
        room_id: RoomId,
        granter_id: UserId,
        target_user_id: UserId,
        permission: i64,
    ) -> Result<RoomMember> {
        // Check if granter has permission to grant
        self.check_permission(&room_id, &granter_id, PermissionBits::GRANT_PERMISSION).await?;

        // Get target member
        let mut member = self
            .member_repo
            .get(&room_id, &target_user_id)
            .await?
            .ok_or_else(|| Error::NotFound("User is not a member of this room".to_string()))?;

        // Grant permission
        member.permissions.grant(permission);

        // Update
        self.member_repo
            .update_permissions(&room_id, &target_user_id, member.permissions)
            .await
    }

    /// List all rooms (paginated)
    pub async fn list_rooms(&self, query: &RoomListQuery) -> Result<(Vec<Room>, i64)> {
        self.room_repo.list(query).await
    }

    /// Get room members with user info
    pub async fn get_room_members(&self, room_id: &RoomId) -> Result<Vec<RoomMemberWithUser>> {
        self.member_repo.list_by_room(room_id).await
    }

    /// Get member count for a room
    pub async fn get_member_count(&self, room_id: &RoomId) -> Result<i32> {
        self.member_repo.count_by_room(room_id).await
    }

    /// Update member permissions
    pub async fn update_member_permission(
        &self,
        room_id: RoomId,
        granter_id: UserId,
        target_user_id: UserId,
        permissions: PermissionBits,
    ) -> Result<RoomMember> {
        // Check if granter has permission to modify permissions
        self.check_permission(&room_id, &granter_id, PermissionBits::GRANT_PERMISSION).await?;

        // Verify target is a member
        if !self.member_repo.is_member(&room_id, &target_user_id).await? {
            return Err(Error::NotFound("User is not a member of this room".to_string()));
        }

        // Update permissions
        self.member_repo
            .update_permissions(&room_id, &target_user_id, permissions)
            .await
    }

    /// Kick member from room
    pub async fn kick_member(
        &self,
        room_id: RoomId,
        kicker_id: UserId,
        target_user_id: UserId,
    ) -> Result<()> {
        // Check if kicker has permission to kick
        self.check_permission(&room_id, &kicker_id, PermissionBits::KICK_USER).await?;

        // Can't kick yourself
        if kicker_id == target_user_id {
            return Err(Error::InvalidInput("Cannot kick yourself".to_string()));
        }

        // Remove member
        let removed = self.member_repo.remove(&room_id, &target_user_id).await?;
        if !removed {
            return Err(Error::NotFound("User is not a member of this room".to_string()));
        }

        Ok(())
    }

    /// Swap positions of two media items in playlist
    pub async fn swap_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        media_id1: MediaId,
        media_id2: MediaId,
    ) -> Result<()> {
        // Check permission - swapping media requires ADD_MEDIA permission
        self.check_permission(&room_id, &user_id, PermissionBits::ADD_MEDIA).await?;

        // Swap positions
        self.media_repo.swap_positions(&media_id1, &media_id2).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_create_room() {
        // Integration test placeholder
    }

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_permission_check() {
        // Test permission verification
    }
}
