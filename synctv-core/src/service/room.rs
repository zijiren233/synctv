//! Room management service
//!
//! Handles core room CRUD operations and coordinates with domain services.

use sqlx::PgPool;
use serde_json::json;
use chrono::{DateTime, Utc};

use crate::{
    models::{
        Room, RoomId, RoomMember, RoomSettings, RoomStatus, RoomWithCount, UserId,
        PermissionBits, Role, RoomPlaybackState, Media, MediaId,
        RoomListQuery, ChatMessage,
    },
    repository::{RoomRepository, RoomMemberRepository, MediaRepository, RoomPlaybackStateRepository, ChatRepository},
    service::{
        auth::password::{hash_password, verify_password},
        permission::PermissionService,
        member::MemberService,
        media::MediaService,
        playback::PlaybackService,
        notification::NotificationService,
    },
    Error, Result,
};

/// Room service for business logic
///
/// This is the main service that coordinates between domain services.
/// Core room operations are handled here, while specific domains are delegated.
#[derive(Clone)]
pub struct RoomService {
    // Core repositories
    room_repo: RoomRepository,
    playback_repo: RoomPlaybackStateRepository,
    chat_repo: ChatRepository,

    // Domain services
    member_service: MemberService,
    permission_service: PermissionService,
    media_service: MediaService,
    playback_service: PlaybackService,
    notification_service: NotificationService,
}

impl std::fmt::Debug for RoomService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoomService").finish()
    }
}

impl RoomService {
    pub fn new(pool: PgPool) -> Self {
        // Initialize repositories
        let room_repo = RoomRepository::new(pool.clone());
        let member_repo = RoomMemberRepository::new(pool.clone());
        let media_repo = MediaRepository::new(pool.clone());
        let playback_repo = RoomPlaybackStateRepository::new(pool.clone());
        let chat_repo = ChatRepository::new(pool);

        // Initialize permission service with caching
        let permission_service = PermissionService::new(member_repo.clone(), 10000, 300);

        // Initialize domain services
        let member_service = MemberService::new(member_repo.clone(), room_repo.clone(), permission_service.clone());
        let media_service = MediaService::new(media_repo, permission_service.clone());
        let playback_service = PlaybackService::new(playback_repo.clone(), permission_service.clone(), media_service.clone());
        let notification_service = NotificationService::new();

        Self {
            room_repo,
            playback_repo,
            chat_repo,
            member_service,
            permission_service,
            media_service,
            playback_service,
            notification_service,
        }
    }

    // ========== Core Room Operations ==========

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
        let created_member = self.member_service.add_member(
            created_room.id.clone(),
            member.user_id.clone(),
            Role::Creator,
        ).await?;

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

        // Add member (will check if already member and max members)
        let created_member = self.member_service.add_member(room_id.clone(), user_id.clone(), Role::Member).await?;

        // Get all members
        let members = self.member_service.list_members(&room_id).await?;

        // Notify room members
        // TODO: Get username from user service
        let _ = self.notification_service.notify_user_joined(&room_id, &user_id, "user").await;

        Ok((room, created_member, members))
    }

    /// Leave a room
    pub async fn leave_room(&self, room_id: RoomId, user_id: UserId) -> Result<()> {
        self.member_service.remove_member(room_id.clone(), user_id.clone()).await?;

        // Notify room members
        // TODO: Get username from user service
        let _ = self.notification_service.notify_user_left(&room_id, &user_id, "user").await;

        Ok(())
    }

    /// Delete a room (creator only)
    pub async fn delete_room(&self, room_id: RoomId, user_id: UserId) -> Result<()> {
        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::DELETE_ROOM)
            .await?;

        // Notify before deletion
        let _ = self.notification_service.notify_room_deleted(&room_id).await;

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
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::UPDATE_ROOM_SETTINGS)
            .await?;

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

        // Notify room members
        let _ = self.notification_service.notify_settings_updated(&room_id, room.settings).await;

        Ok(updated_room)
    }

    // ========== Query Operations ==========

    /// Get room with details
    pub async fn get_room(&self, room_id: &RoomId) -> Result<Room> {
        self.room_repo
            .get_by_id(room_id)
            .await?
            .ok_or_else(|| Error::NotFound("Room not found".to_string()))
    }

    /// List all rooms (paginated)
    pub async fn list_rooms(&self, query: &RoomListQuery) -> Result<(Vec<Room>, i64)> {
        self.room_repo.list(query).await
    }

    /// List all rooms with member count (optimized, single query)
    pub async fn list_rooms_with_count(&self, query: &RoomListQuery) -> Result<(Vec<RoomWithCount>, i64)> {
        self.room_repo.list_with_count(query).await
    }

    /// List rooms created by a specific user
    pub async fn list_rooms_by_creator(&self, creator_id: &UserId, page: i64, page_size: i64) -> Result<(Vec<Room>, i64)> {
        self.room_repo.list_by_creator(creator_id, page, page_size).await
    }

    /// List rooms created by a specific user with member count (optimized)
    pub async fn list_rooms_by_creator_with_count(
        &self,
        creator_id: &UserId,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<RoomWithCount>, i64)> {
        self.room_repo.list_by_creator_with_count(creator_id, page, page_size).await
    }

    /// List rooms where a user is a member
    pub async fn list_joined_rooms(&self, user_id: &UserId, page: i64, page_size: i64) -> Result<(Vec<RoomId>, i64)> {
        self.member_service.list_user_rooms(user_id, page, page_size).await
    }

    /// List rooms where a user is a member with full details (optimized)
    pub async fn list_joined_rooms_with_details(
        &self,
        user_id: &UserId,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<(Room, PermissionBits, i32)>, i64)> {
        self.member_service.list_user_rooms_with_details(user_id, page, page_size).await
    }

    // ========== Member Operations (delegated) ==========

    /// Grant permission to user
    pub async fn grant_permission(
        &self,
        room_id: RoomId,
        granter_id: UserId,
        target_user_id: UserId,
        permission: i64,
    ) -> Result<crate::models::RoomMember> {
        self.member_service.grant_permission(room_id, granter_id, target_user_id, permission).await
    }

    /// Update member permissions
    pub async fn update_member_permission(
        &self,
        room_id: RoomId,
        granter_id: UserId,
        target_user_id: UserId,
        permissions: PermissionBits,
    ) -> Result<crate::models::RoomMember> {
        self.member_service.update_member_permissions(room_id, granter_id, target_user_id, permissions).await
    }

    /// Kick member from room
    pub async fn kick_member(
        &self,
        room_id: RoomId,
        kicker_id: UserId,
        target_user_id: UserId,
    ) -> Result<()> {
        self.member_service.kick_member(room_id, kicker_id, target_user_id).await
    }

    /// Get room members with user info
    pub async fn get_room_members(&self, room_id: &RoomId) -> Result<Vec<crate::models::RoomMemberWithUser>> {
        self.member_service.list_members(room_id).await
    }

    /// Get member count for a room
    pub async fn get_member_count(&self, room_id: &RoomId) -> Result<i32> {
        self.member_service.count_members(room_id).await
    }

    /// Check if user is a member of the room
    pub async fn check_membership(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<()> {
        if self.member_service.is_member(room_id, user_id).await? {
            Ok(())
        } else {
            Err(Error::PermissionDenied("Not a member of this room".to_string()))
        }
    }

    // ========== Media Operations (delegated) ==========

    /// Add media to playlist
    pub async fn add_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        url: String,
        provider: crate::models::ProviderType,
        title: String,
    ) -> Result<Media> {
        self.media_service.add_media(room_id, user_id, url, provider, title).await
    }

    /// Remove media from playlist
    pub async fn remove_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        media_id: MediaId,
    ) -> Result<()> {
        self.media_service.remove_media(room_id, user_id, media_id).await
    }

    /// Get playlist
    pub async fn get_playlist(&self, room_id: &RoomId) -> Result<Vec<Media>> {
        self.media_service.get_playlist(room_id).await
    }

    /// Swap positions of two media items in playlist
    pub async fn swap_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        media_id1: MediaId,
        media_id2: MediaId,
    ) -> Result<()> {
        self.media_service.swap_media(room_id, user_id, media_id1, media_id2).await
    }

    // ========== Playback Operations (delegated) ==========

    /// Update playback state (play/pause/seek/etc)
    pub async fn update_playback(
        &self,
        room_id: RoomId,
        user_id: UserId,
        update_fn: impl FnOnce(&mut RoomPlaybackState),
        required_permission: i64,
    ) -> Result<RoomPlaybackState> {
        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, required_permission)
            .await?;

        // Get current state and apply update
        self.playback_service.update_state(room_id, update_fn).await
    }

    /// Get playback state
    pub async fn get_playback_state(&self, room_id: &RoomId) -> Result<RoomPlaybackState> {
        self.playback_service.get_state(room_id).await
    }

    // ========== Chat Operations ==========

    /// Get chat history for a room
    pub async fn get_chat_history(
        &self,
        room_id: &RoomId,
        before: Option<DateTime<Utc>>,
        limit: i32,
    ) -> Result<Vec<ChatMessage>> {
        self.chat_repo.list_by_room(room_id, before, limit).await
    }

    /// Save a chat message to the database
    pub async fn save_chat_message(
        &self,
        room_id: RoomId,
        user_id: UserId,
        content: String,
    ) -> Result<ChatMessage> {
        let message = ChatMessage {
            id: nanoid::nanoid!(12),
            room_id,
            user_id,
            content,
            created_at: Utc::now(),
            deleted_at: None,
        };
        self.chat_repo.create(&message).await
    }

    // ========== Permission Operations (delegated) ==========

    /// Check if user has permission in room
    pub async fn check_permission(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        permission: i64,
    ) -> Result<()> {
        self.permission_service.check_permission(room_id, user_id, permission).await
    }

    // ========== Admin Operations ==========

    /// Update room directly (admin use, bypasses permission checks)
    pub async fn admin_update_room(&self, room: &Room) -> Result<Room> {
        self.room_repo.update(room).await
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
