//! Room management service
//!
//! Handles core room CRUD operations and coordinates with domain services.

use sqlx::PgPool;
use chrono::{DateTime, Utc};

use crate::{
    models::{
        Room, RoomId, RoomMember, RoomSettings, RoomStatus, RoomWithCount, UserId,
        PermissionBits, RoomRole, MemberStatus, RoomPlaybackState, Media, MediaId,
        RoomListQuery, ChatMessage,
    },
    repository::{RoomRepository, RoomMemberRepository, MediaRepository, PlaylistRepository, RoomPlaybackStateRepository, ChatRepository, RoomSettingsRepository},
    service::{
        auth::password::{hash_password, verify_password},
        permission::PermissionService,
        member::MemberService,
        media::MediaService,
        playlist::PlaylistService,
        playback::PlaybackService,
        notification::NotificationService,
        user::UserService,
        ProvidersManager,
    },
    Error, Result,
};
use std::sync::Arc;

// Re-export gRPC types for use in service layer
pub use synctv_proto::admin::{
    GetRoomRequest, GetRoomResponse,
    ListRoomsRequest, ListRoomsResponse,
    DeleteRoomRequest, DeleteRoomResponse,
    UpdateRoomPasswordRequest, UpdateRoomPasswordResponse,
    GetRoomMembersRequest, GetRoomMembersResponse,
    AdminRoom,
};

/// Room service for business logic
///
/// This is the main service that coordinates between domain services.
/// Core room operations are handled here, while specific domains are delegated.
#[derive(Clone)]
pub struct RoomService {
    // Core repositories
    room_repo: RoomRepository,
    room_settings_repo: RoomSettingsRepository,
    playback_repo: RoomPlaybackStateRepository,
    chat_repo: ChatRepository,

    // Domain services
    member_service: MemberService,
    permission_service: PermissionService,
    playlist_service: PlaylistService,
    media_service: MediaService,
    playback_service: PlaybackService,
    notification_service: NotificationService,
    user_service: UserService,
}

impl std::fmt::Debug for RoomService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoomService").finish()
    }
}

impl RoomService {
    /// Get the playlist service
    #[must_use] 
    pub const fn playlist_service(&self) -> &PlaylistService {
        &self.playlist_service
    }

    #[must_use] 
    pub fn new(pool: PgPool, user_service: UserService) -> Self {
        // Initialize repositories
        let room_repo = RoomRepository::new(pool.clone());
        let room_settings_repo = RoomSettingsRepository::new(pool.clone());
        let member_repo = RoomMemberRepository::new(pool.clone());
        let media_repo = MediaRepository::new(pool.clone());
        let playlist_repo = PlaylistRepository::new(pool.clone());
        let playback_repo = RoomPlaybackStateRepository::new(pool.clone());
        let provider_instance_repo = Arc::new(crate::repository::ProviderInstanceRepository::new(pool.clone()));
        let chat_repo = ChatRepository::new(pool);

        // Initialize permission service with caching
        let mut permission_service = PermissionService::new(
            member_repo.clone(),
            room_repo.clone(),
            None, // SettingsRegistry - will be set later if needed
            10000,
            300
        );
        permission_service.set_room_settings_repo(room_settings_repo.clone());

        // Initialize provider instance manager and providers manager
        let provider_instance_manager = Arc::new(crate::service::RemoteProviderManager::new(provider_instance_repo));
        let providers_manager = Arc::new(ProvidersManager::new(provider_instance_manager));

        // Initialize domain services
        let mut member_service = MemberService::new(member_repo, room_repo.clone(), permission_service.clone());
        member_service.set_room_settings_repo(room_settings_repo.clone());
        let playlist_service = PlaylistService::new(playlist_repo.clone(), permission_service.clone());
        let media_service = MediaService::new(
            media_repo.clone(),
            playlist_repo,
            permission_service.clone(),
            providers_manager,
        );
        let playback_service = PlaybackService::new(playback_repo.clone(), permission_service.clone(), media_service.clone(), media_repo);
        let notification_service = NotificationService::default();

        Self {
            room_repo,
            room_settings_repo,
            playback_repo,
            chat_repo,
            member_service,
            permission_service,
            playlist_service,
            media_service,
            playback_service,
            notification_service,
            user_service,
        }
    }

    // ========== Core Room Operations ==========

    /// Create a new room
    pub async fn create_room(
        &self,
        name: String,
        description: String,
        created_by: UserId,
        password: Option<String>,
        settings: Option<RoomSettings>,
    ) -> Result<(Room, RoomMember)> {
        tracing::info!(
            user_id = %created_by,
            room_name = %name,
            has_password = password.is_some(),
            "Creating new room"
        );

        // Validate room name
        if name.is_empty() {
            tracing::warn!(user_id = %created_by, "Attempted to create room with empty name");
            return Err(Error::InvalidInput("Room name cannot be empty".to_string()));
        }
        if name.len() > 255 {
            tracing::warn!(user_id = %created_by, name_len = name.len(), "Attempted to create room with name too long");
            return Err(Error::InvalidInput("Room name too long".to_string()));
        }

        // Validate description length
        if description.len() > 500 {
            tracing::warn!(user_id = %created_by, desc_len = description.len(), "Attempted to create room with description too long");
            return Err(Error::InvalidInput("Room description too long (max 500 characters)".to_string()));
        }

        // Build settings
        let mut room_settings = settings.unwrap_or_default();
        room_settings.require_password = crate::models::room_settings::RequirePassword(password.is_some());

        // Create room
        let room = Room::new_with_description(name, description, created_by.clone());
        let created_room = self.room_repo.create(&room).await?;

        tracing::info!(
            room_id = %created_room.id,
            user_id = %created_by,
            "Room created successfully"
        );

        // Complete room setup — if any step fails, clean up by deleting the room
        match self
            .setup_new_room(&created_room, &created_by, password, room_settings)
            .await
        {
            Ok(member) => {
                tracing::info!(
                    room_id = %created_room.id,
                    user_id = %created_by,
                    "Room creation completed"
                );
                Ok((created_room, member))
            }
            Err(e) => {
                tracing::error!(
                    room_id = %created_room.id,
                    error = %e,
                    "Room setup failed, cleaning up"
                );
                if let Err(cleanup_err) = self.room_repo.delete(&created_room.id).await {
                    tracing::error!(
                        room_id = %created_room.id,
                        error = %cleanup_err,
                        "Failed to clean up partially created room"
                    );
                }
                Err(e)
            }
        }
    }

    /// Internal helper: set up a newly created room (settings, member, playlist, playback).
    /// Returns the creator's `RoomMember` on success.
    async fn setup_new_room(
        &self,
        room: &Room,
        created_by: &UserId,
        password: Option<String>,
        room_settings: RoomSettings,
    ) -> Result<RoomMember> {
        // Hash password if provided and store in room_settings
        if let Some(pwd) = password {
            let pwd_hash = hash_password(&pwd).await?;
            self.room_settings_repo
                .set(&room.id, "password", &pwd_hash)
                .await?;
            tracing::debug!(room_id = %room.id, "Room password set");
        }

        self.room_settings_repo
            .set_settings(&room.id, &room_settings)
            .await?;

        // Add creator as member with full permissions
        let created_member = self
            .member_service
            .add_member(room.id.clone(), created_by.clone(), RoomRole::Creator)
            .await?;

        // Create root playlist for the room
        self.playlist_service
            .create_root_playlist(&room.id, created_by)
            .await?;

        // Initialize playback state
        self.playback_repo.create_or_get(&room.id).await?;

        Ok(created_member)
    }

    /// Join a room
    pub async fn join_room(
        &self,
        room_id: RoomId,
        user_id: UserId,
        password: Option<String>,
    ) -> Result<(Room, RoomMember, Vec<crate::models::RoomMemberWithUser>)> {
        tracing::info!(
            room_id = %room_id,
            user_id = %user_id,
            has_password = password.is_some(),
            "User attempting to join room"
        );

        // Get room
        let room = self
            .room_repo
            .get_by_id(&room_id)
            .await?
            .ok_or_else(|| {
                tracing::warn!(room_id = %room_id, user_id = %user_id, "Room not found");
                Error::NotFound("Room not found".to_string())
            })?;

        // Check if room is active
        if room.status != RoomStatus::Active {
            tracing::warn!(room_id = %room_id, user_id = %user_id, status = ?room.status, "Attempted to join inactive room");
            return Err(Error::InvalidInput("Room is closed".to_string()));
        }

        // Check if user is banned from this room
        if self.member_service.is_banned(&room_id, &user_id).await? {
            tracing::warn!(room_id = %room_id, user_id = %user_id, "Banned user attempted to join room");
            return Err(Error::Authorization("You are banned from this room".to_string()));
        }

        // Check password if required
        // Load settings to check if password is required
        let room_settings = self.room_settings_repo.get(&room_id).await?;
        if room_settings.require_password.0 {
            // Load password hash from room_settings table
            let password_hash = self.room_settings_repo.get_password_hash(&room_id).await?;

            if let Some(hash) = password_hash {
                let provided_password = password.ok_or_else(|| {
                    tracing::warn!(room_id = %room_id, user_id = %user_id, "Password required but not provided");
                    Error::Authorization("Password required".to_string())
                })?;

                // Verify password using Argon2id
                let is_valid = verify_password(&provided_password, &hash).await?;
                if !is_valid {
                    tracing::warn!(room_id = %room_id, user_id = %user_id, "Invalid password provided");
                    return Err(Error::Authorization("Invalid password".to_string()));
                }
                tracing::debug!(room_id = %room_id, user_id = %user_id, "Password verified successfully");
            } else {
                tracing::error!(room_id = %room_id, "Password required but not set in database");
                return Err(Error::Authorization("Password required but not set".to_string()));
            }
        }

        // Add member (will check if already member and max members)
        let created_member = self.member_service.add_member(room_id.clone(), user_id.clone(), RoomRole::Member).await?;

        // Get all members
        let members = self.member_service.list_members(&room_id).await?;

        // Notify room members with username
        let username = self.user_service.get_username(&user_id).await?.unwrap_or_else(|| "Unknown".to_string());
        let _ = self.notification_service.notify_user_joined(&room_id, &user_id, &username).await;

        tracing::info!(
            room_id = %room_id,
            user_id = %user_id,
            username = %username,
            member_count = members.len(),
            "User joined room successfully"
        );

        Ok((room, created_member, members))
    }

    /// Leave a room
    pub async fn leave_room(&self, room_id: RoomId, user_id: UserId) -> Result<()> {
        tracing::info!(room_id = %room_id, user_id = %user_id, "User leaving room");

        self.member_service.remove_member(room_id.clone(), user_id.clone()).await?;

        // Notify room members with username
        let username = self.user_service.get_username(&user_id).await?.unwrap_or_else(|| "Unknown".to_string());
        let _ = self.notification_service.notify_user_left(&room_id, &user_id, &username).await;

        tracing::info!(room_id = %room_id, user_id = %user_id, username = %username, "User left room");

        Ok(())
    }

    /// Check if guests are allowed to access a room
    ///
    /// Validates guest access based on:
    /// 1. Global `enable_guest` setting
    /// 2. Room `allow_guest_join` setting
    /// 3. Room password requirement (guests blocked if password required)
    ///
    /// # Arguments
    /// * `room_id` - Room ID to check
    /// * `settings_registry` - Optional global settings registry (if None, guest mode is allowed)
    ///
    /// # Returns
    /// * `Ok(())` if guests are allowed
    /// * `Err` with appropriate error message if guests are not allowed
    pub async fn check_guest_allowed(
        &self,
        room_id: &RoomId,
        settings_registry: Option<&crate::service::SettingsRegistry>,
    ) -> Result<()> {
        // Check global enable_guest setting
        if let Some(registry) = settings_registry {
            let enable_guest = registry.enable_guest.get().unwrap_or(true);
            if !enable_guest {
                tracing::debug!(room_id = %room_id, "Guest access denied: global guest mode disabled");
                return Err(Error::Authorization(
                    "Guest mode is disabled globally".to_string(),
                ));
            }
        }

        // Get room settings
        let room_settings = self.room_settings_repo.get(room_id).await?;

        // Check room-level allow_guest_join setting
        if !room_settings.allow_guest_join.0 {
            tracing::debug!(room_id = %room_id, "Guest access denied: room guest mode disabled");
            return Err(Error::Authorization(
                "Guest access is not allowed in this room".to_string(),
            ));
        }

        // Check if room has password (guests cannot join password-protected rooms)
        if room_settings.require_password.0 {
            tracing::debug!(room_id = %room_id, "Guest access denied: room has password");
            return Err(Error::Authorization(
                "Guests cannot join password-protected rooms. Please create an account and join as a member.".to_string(),
            ));
        }

        tracing::debug!(room_id = %room_id, "Guest access allowed");
        Ok(())
    }

    /// Delete a room (creator only)
    pub async fn delete_room(&self, room_id: RoomId, user_id: UserId) -> Result<()> {
        tracing::info!(room_id = %room_id, user_id = %user_id, "Deleting room");

        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::DELETE_ROOM)
            .await?;

        // Notify before deletion
        let _ = self.notification_service.notify_room_deleted(&room_id).await;

        // Delete room
        self.room_repo.delete(&room_id).await?;

        tracing::info!(room_id = %room_id, user_id = %user_id, "Room deleted successfully");

        Ok(())
    }

    /// Set room settings
    pub async fn set_settings(
        &self,
        room_id: RoomId,
        user_id: UserId,
        settings: RoomSettings,
    ) -> Result<Room> {
        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::UPDATE_ROOM_SETTINGS)
            .await?;

        // Verify room exists
        let room = self
            .room_repo
            .get_by_id(&room_id)
            .await?
            .ok_or_else(|| Error::NotFound("Room not found".to_string()))?;

        // Save settings to room_settings table
        self.room_settings_repo.set_settings(&room_id, &settings).await?;

        // Invalidate permission cache for all room members (room-level permission
        // settings like admin/member/guest added/removed affect everyone)
        self.permission_service.invalidate_room_cache(&room_id).await;

        // Notify room members
        let settings_json = serde_json::to_value(&settings)?;
        let _ = self.notification_service.notify_settings_updated(&room_id, settings_json).await;

        Ok(room)
    }

    // ========== Query Operations ==========

    /// Get room with details
    pub async fn get_room(&self, room_id: &RoomId) -> Result<Room> {
        self.room_repo
            .get_by_id(room_id)
            .await?
            .ok_or_else(|| Error::NotFound("Room not found".to_string()))
    }

    /// Get room with settings
    pub async fn get_room_with_settings(&self, room_id: &RoomId) -> Result<(Room, RoomSettings)> {
        let room = self.get_room(room_id).await?;
        let settings = self.room_settings_repo.get(room_id).await?;
        Ok((room, settings))
    }

    /// Get room settings
    pub async fn get_room_settings(&self, room_id: &RoomId) -> Result<RoomSettings> {
        self.room_settings_repo.get(room_id).await
    }

    /// Get settings for multiple rooms in a single query (avoids N+1)
    pub async fn get_room_settings_batch(&self, room_ids: &[&str]) -> Result<std::collections::HashMap<String, RoomSettings>> {
        self.room_settings_repo.get_batch(room_ids).await
    }

    /// Set room settings (replace entire settings object)
    pub async fn set_room_settings(&self, room_id: &RoomId, settings: &RoomSettings) -> Result<RoomSettings> {
        self.room_settings_repo.set_settings(room_id, settings).await?;
        // Return the updated settings
        self.room_settings_repo.get(room_id).await
    }

    /// Update single room setting (requires `UPDATE_ROOM_SETTINGS` permission)
    pub async fn update_room_setting(&self, room_id: &RoomId, user_id: &UserId, key: &str, value: &serde_json::Value) -> Result<String> {
        // Check permission (defense-in-depth, same as set_settings)
        self.permission_service
            .check_permission(room_id, user_id, PermissionBits::UPDATE_ROOM_SETTINGS)
            .await?;
        use crate::models::{AutoPlaySettings, PlayMode, room_settings::{ChatEnabled, DanmakuEnabled, AutoPlay, AllowGuestJoin, RequirePassword, MaxMembers, AutoPlayNext, LoopPlaylist, ShufflePlaylist}};
        use crate::service::notification::GuestKickReason;

        let mut settings = self.room_settings_repo.get(room_id).await?;
        let mut should_kick_guests = false;
        let mut kick_reason = GuestKickReason::RoomGuestModeDisabled;

        // Update the specific setting based on key
        match key {
            "chat_enabled" => {
                if let Some(bool_val) = value.as_bool() {
                    settings.chat_enabled = ChatEnabled(bool_val);
                }
            }
            "danmaku_enabled" => {
                if let Some(bool_val) = value.as_bool() {
                    settings.danmaku_enabled = DanmakuEnabled(bool_val);
                }
            }
            "auto_play" => {
                if let Some(bool_val) = value.as_bool() {
                    settings.auto_play = AutoPlay::new(AutoPlaySettings {
                        enabled: bool_val,
                        mode: PlayMode::default(),
                        delay: 0,
                    });
                }
            }
            "allow_guest_join" => {
                if let Some(bool_val) = value.as_bool() {
                    settings.allow_guest_join = AllowGuestJoin(bool_val);
                    // If guest mode is disabled, kick all guests
                    if !bool_val {
                        should_kick_guests = true;
                        kick_reason = GuestKickReason::RoomGuestModeDisabled;
                    }
                }
            }
            "require_password" => {
                if let Some(bool_val) = value.as_bool() {
                    // Prevent enabling require_password when no password is set
                    if bool_val {
                        let has_password = self.room_settings_repo.get_password_hash(room_id).await?.is_some();
                        if !has_password {
                            return Err(Error::InvalidInput(
                                "Cannot require password when no password is set. Set a password first.".to_string()
                            ));
                        }
                        should_kick_guests = true;
                        kick_reason = GuestKickReason::RoomPasswordAdded;
                    }
                    settings.require_password = RequirePassword(bool_val);
                }
            }
            "max_members" => {
                if let Some(num_val) = value.as_u64() {
                    settings.max_members = MaxMembers(num_val);
                }
            }
            "auto_play_next" => {
                if let Some(bool_val) = value.as_bool() {
                    settings.auto_play_next = AutoPlayNext(bool_val);
                }
            }
            "loop_playlist" => {
                if let Some(bool_val) = value.as_bool() {
                    settings.loop_playlist = LoopPlaylist(bool_val);
                }
            }
            "shuffle_playlist" => {
                if let Some(bool_val) = value.as_bool() {
                    settings.shuffle_playlist = ShufflePlaylist(bool_val);
                }
            }
            _ => {
                return Err(Error::InvalidInput(format!("Unknown setting key: {key}")));
            }
        }

        // Save the updated settings
        self.room_settings_repo.set_settings(room_id, &settings).await?;

        // Invalidate permission cache for all room members
        self.permission_service.invalidate_room_cache(room_id).await;

        // Kick guests if needed
        if should_kick_guests {
            if let Err(e) = self.notification_service.kick_all_guests(room_id, kick_reason).await {
                tracing::warn!("Failed to kick guests after settings change: {}", e);
            }
        }

        // Return updated settings as JSON string
        serde_json::to_string(&settings)
            .map_err(|e| Error::Internal(format!("Failed to serialize settings: {e}")))
    }

    /// Reset room settings to default values
    pub async fn reset_room_settings(&self, room_id: &RoomId) -> Result<String> {
        let default_settings = RoomSettings::default();
        self.room_settings_repo.set_settings(room_id, &default_settings).await?;

        // Return default settings as JSON string
        serde_json::to_string(&default_settings)
            .map_err(|e| Error::Internal(format!("Failed to serialize settings: {e}")))
    }

    /// Check room password
    pub async fn check_room_password(&self, room_id: &RoomId, password: &str) -> Result<bool> {
        let password_hash = self.room_settings_repo.get_password_hash(room_id).await?;

        match password_hash {
            Some(stored) => {
                verify_password(password, &stored).await
                    .map_err(|e| Error::Internal(format!("Password verification failed: {e}")))
            }
            None => Ok(false),
        }
    }

    /// Update room password
    pub async fn update_room_password(&self, room_id: &RoomId, password_hash: Option<String>) -> Result<()> {
        use crate::service::notification::GuestKickReason;

        if let Some(pwd_hash) = password_hash {
            self.room_settings_repo.set(room_id, "password", &pwd_hash).await?;

            // Sync require_password setting to true when password is set
            let mut settings = self.get_room_settings(room_id).await?;
            if !settings.require_password.0 {
                settings.require_password = crate::models::room_settings::RequirePassword(true);
                self.room_settings_repo.set_settings(room_id, &settings).await?;
            }

            // Kick all guests when password is added (guests cannot access password-protected rooms)
            if let Err(e) = self.notification_service.kick_all_guests(
                room_id,
                GuestKickReason::RoomPasswordAdded
            ).await {
                tracing::warn!("Failed to kick guests after password was added: {}", e);
            }
        } else {
            self.room_settings_repo.delete(room_id, "password").await?;

            // Sync require_password setting to false when password is removed
            let mut settings = self.get_room_settings(room_id).await?;
            if settings.require_password.0 {
                settings.require_password = crate::models::room_settings::RequirePassword(false);
                self.room_settings_repo.set_settings(room_id, &settings).await?;
            }
        }
        Ok(())
    }

    /// Update room description
    pub async fn update_room_description(&self, room_id: &RoomId, description: String) -> Result<Room> {
        if description.len() > 500 {
            return Err(Error::InvalidInput("Room description too long (max 500 characters)".to_string()));
        }
        self.room_repo.update_description(room_id, &description).await
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
    ) -> Result<(Vec<(Room, RoomRole, MemberStatus, i32)>, i64)> {
        self.member_service.list_user_rooms_with_details(user_id, page, page_size).await
    }

    // ========== Member Operations (delegated) ==========

    /// Grant permission to user
    pub async fn grant_permission(
        &self,
        room_id: RoomId,
        granter_id: UserId,
        target_user_id: UserId,
        permission: u64,
    ) -> Result<crate::models::RoomMember> {
        self.member_service.grant_permission(room_id, granter_id, target_user_id, permission).await
    }

    /// Update member permissions (Allow/Deny pattern)
    ///
    /// This method sets both `added_permissions` and `removed_permissions`.
    /// To reset to role default, pass 0 for both.
    pub async fn set_member_permission(
        &self,
        room_id: RoomId,
        granter_id: UserId,
        target_user_id: UserId,
        added_permissions: u64,
        removed_permissions: u64,
    ) -> Result<crate::models::RoomMember> {
        self.member_service.set_member_permissions(room_id, granter_id, target_user_id, added_permissions, removed_permissions).await
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

    /// Add media to playlist (convenience method)
    ///
    /// This is a convenience method that:
    /// 1. Gets the root playlist for the room
    /// 2. Calls `MediaService::add_media` with the provided `source_config`
    ///
    /// Note: Clients should typically call the parse endpoint first to get
    /// `source_config`, then call this method with `provider_instance_name`.
    ///
    /// Uses provider registry pattern - no enum switching in service layer.
    pub async fn add_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        provider_instance_name: String,
        source_config: serde_json::Value,
        title: String,
    ) -> Result<Media> {
        use crate::service::media::AddMediaRequest;

        // Get room's root playlist
        let root_playlist = self.playlist_service.get_root_playlist(&room_id).await?;

        // Create request with provider_instance_name
        let request = AddMediaRequest {
            playlist_id: root_playlist.id.clone(),
            name: title,
            provider_instance_name,
            source_config,
        };

        self.media_service.add_media(room_id, user_id, request).await
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

    /// Get playlist (all media in room's root playlist)
    pub async fn get_playlist(&self, room_id: &RoomId) -> Result<Vec<Media>> {
        let root_playlist = self.playlist_service.get_root_playlist(room_id).await?;
        self.media_service.get_playlist_media(&root_playlist.id).await
    }

    /// Get playlist paginated
    pub async fn get_playlist_paginated(
        &self,
        room_id: &RoomId,
        page: i32,
        page_size: i32,
    ) -> Result<(Vec<Media>, i64)> {
        let root_playlist = self.playlist_service.get_root_playlist(room_id).await?;
        self.media_service.get_playlist_media_paginated(&root_playlist.id, page, page_size).await
    }

    /// Get current playing media for a room
    pub async fn get_playing_media(&self, room_id: &RoomId) -> Result<Option<Media>> {
        let state = self.playback_service.get_state(room_id).await?;
        if let Some(media_id) = state.playing_media_id {
            Ok(self.media_service.get_media(&media_id).await?)
        } else {
            Ok(None)
        }
    }

    /// Edit media item
    pub async fn edit_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        media_id: MediaId,
        name: Option<String>,
    ) -> Result<Media> {
        use crate::service::media::EditMediaRequest;
        let request = EditMediaRequest {
            media_id,
            name,
            position: None,
        };
        self.media_service.edit_media(room_id, user_id, request).await
    }

    /// Clear all media from room's root playlist
    pub async fn clear_playlist(&self, room_id: RoomId, user_id: UserId) -> Result<i64> {
        let root_playlist = self.playlist_service.get_root_playlist(&room_id).await?;
        let playlist_id = root_playlist.id.clone();

        // Get all media in playlist
        let media_list = self.media_service.get_playlist_media(&playlist_id).await?;

        if media_list.is_empty() {
            return Ok(0);
        }

        // Batch delete for atomicity
        let media_ids: Vec<_> = media_list.into_iter().map(|m| m.id).collect();
        let count = self.media_service
            .remove_media_batch(room_id, user_id, media_ids)
            .await? as i64;

        Ok(count)
    }

    /// Set current playing media for a room
    pub async fn set_playing_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        media_id: MediaId,
    ) -> Result<RoomPlaybackState> {
        self.playback_service.switch_media(room_id, user_id, media_id).await
    }

    /// Swap positions of two media items in playlist
    pub async fn swap_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        media_id1: MediaId,
        media_id2: MediaId,
    ) -> Result<()> {
        self.media_service.swap_media_positions(room_id, user_id, media_id1, media_id2).await
    }

    // ========== Playback Operations (delegated) ==========

    /// Update playback state (play/pause/seek/etc)
    pub async fn update_playback(
        &self,
        room_id: RoomId,
        user_id: UserId,
        update_fn: impl FnOnce(&mut RoomPlaybackState),
        required_permission: u64,
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
        };
        self.chat_repo.create(&message).await
    }

    // ========== Permission Operations (delegated) ==========

    /// Check if user has permission in room
    pub async fn check_permission(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        permission: u64,
    ) -> Result<()> {
        self.permission_service.check_permission(room_id, user_id, permission).await
    }

    // ========== gRPC-typed Methods (New Architecture) ==========

    /// Get room members using gRPC types
    ///
    /// This method demonstrates the new architecture where service layer
    /// uses gRPC-generated types, allowing both HTTP and gRPC layers to be
    /// lightweight wrappers.
    ///
    /// # Arguments
    /// * `request` - `GetRoomMembersRequest` containing `room_id`
    /// * `requesting_user_id` - The user making the request (for permission checking)
    ///
    /// # Returns
    /// `GetRoomMembersResponse` containing list of room members
    pub async fn get_room_members_grpc(
        &self,
        request: synctv_proto::admin::GetRoomMembersRequest,
        requesting_user_id: &UserId,
    ) -> Result<synctv_proto::admin::GetRoomMembersResponse> {
        // Extract room_id
        let room_id = RoomId::from_string(request.room_id.clone());

        // Check permission - user must be a member of the room to see members
        self.permission_service
            .check_permission(&room_id, requesting_user_id, PermissionBits::SEND_CHAT)
            .await?;

        // Get members from repository
        let members_with_users = self.member_service.list_members(&room_id).await?;

        // Load room settings to calculate effective permissions
        let room_settings = self.room_settings_repo.get(&room_id).await?;

        // Convert to gRPC RoomMember type
        let proto_members: Vec<synctv_proto::admin::RoomMember> = members_with_users
            .into_iter()
            .map(|m| {
                // Calculate role default permissions (global + room-level overrides)
                let role_default = self.permission_service
                    .calculate_role_default_permissions(&m.role, &room_settings);

                // Apply member-level overrides
                let effective = m.effective_permissions(role_default);

                synctv_proto::admin::RoomMember {
                    room_id: m.room_id.as_str().to_string(),
                    user_id: m.user_id.as_str().to_string(),
                    username: m.username,
                    role: m.role.to_string(),
                    permissions: effective.0,
                    added_permissions: m.added_permissions,
                    removed_permissions: m.removed_permissions,
                    admin_added_permissions: m.admin_added_permissions,
                    admin_removed_permissions: m.admin_removed_permissions,
                    joined_at: m.joined_at.timestamp(),
                    is_online: m.is_online,
                }
            })
            .collect();

        Ok(synctv_proto::admin::GetRoomMembersResponse {
            members: proto_members,
        })
    }

    /// Get room by ID using gRPC types
    pub async fn get_room_grpc(
        &self,
        request: GetRoomRequest,
        requesting_user_id: &UserId,
    ) -> Result<GetRoomResponse> {
        let room_id = RoomId::from_string(request.room_id);

        // Check permission - user must be able to view the room
        self.permission_service
            .check_permission(&room_id, requesting_user_id, PermissionBits::SEND_CHAT)
            .await?;

        let room = self.get_room(&room_id).await?;

        // Get member count
        let member_count = self.get_member_count(&room_id).await?;

        // Get creator username
        let creator_username = self
            .user_service
            .get_username(&room.created_by)
            .await?
            .unwrap_or_else(|| "Unknown".to_string());

        // Load room settings
        let room_settings = self.room_settings_repo.get(&room_id).await?;

        // Convert settings to bytes (protobuf serialization)
        let settings_bytes = serde_json::to_vec(&room_settings)
            .unwrap_or_default();

        let admin_room = AdminRoom {
            id: room.id.as_str().to_string(),
            name: room.name.clone(),
            description: room.description,
            creator_id: room.created_by.as_str().to_string(),
            creator_username,
            status: room.status.as_str().to_string(),
            settings: settings_bytes,
            member_count,
            created_at: room.created_at.timestamp(),
            updated_at: room.updated_at.timestamp(),
        };

        Ok(GetRoomResponse {
            room: Some(admin_room),
        })
    }

    /// List rooms using gRPC types
    pub async fn list_rooms_grpc(
        &self,
        request: ListRoomsRequest,
        _requesting_user_id: &UserId,
    ) -> Result<ListRoomsResponse> {
        // Build query from request
        let mut query = RoomListQuery {
            page: request.page,
            page_size: request.page_size,
            ..Default::default()
        };

        if !request.status.is_empty() {
            // Parse status string to RoomStatus enum
            query.status = match request.status.as_str() {
                "active" => Some(RoomStatus::Active),
                "banned" => Some(RoomStatus::Banned),
                "pending" => Some(RoomStatus::Pending),
                _ => None,
            };
        }

        if !request.search.is_empty() {
            query.search = Some(request.search);
        }

        if !request.creator_id.is_empty() {
            let creator_id = UserId::from_string(request.creator_id.clone());
            let (rooms, total) = self.list_rooms_by_creator_with_count(&creator_id, i64::from(query.page), i64::from(query.page_size)).await?;

            // Collect creator IDs for batch lookup
            let creator_ids: Vec<UserId> = rooms.iter().map(|r| r.room.created_by.clone()).collect();
            let usernames_map: std::collections::HashMap<UserId, String> = self.user_service.get_usernames(&creator_ids).await.unwrap_or_default();

            // Batch-load settings for all rooms (single query)
            let room_id_strs: Vec<&str> = rooms.iter().map(|r| r.room.id.as_str()).collect();
            let settings_map = self.room_settings_repo.get_batch(&room_id_strs).await.unwrap_or_default();

            // Convert rooms to AdminRoom format
            let admin_rooms: Vec<AdminRoom> = rooms
                .into_iter()
                .map(|r| {
                    let settings = settings_map.get(r.room.id.as_str())
                        .cloned()
                        .unwrap_or_default();
                    let settings_bytes = serde_json::to_vec(&settings).unwrap_or_default();
                    let creator_username = usernames_map
                        .get(&r.room.created_by)
                        .cloned()
                        .unwrap_or_else(|| "Unknown".to_string());
                    AdminRoom {
                        id: r.room.id.as_str().to_string(),
                        name: r.room.name,
                        description: r.room.description,
                        creator_id: r.room.created_by.as_str().to_string(),
                        creator_username,
                        status: r.room.status.as_str().to_string(),
                        settings: settings_bytes,
                        member_count: r.member_count,
                        created_at: r.room.created_at.timestamp(),
                        updated_at: r.room.updated_at.timestamp(),
                    }
                })
                .collect();

            return Ok(ListRoomsResponse {
                rooms: admin_rooms,
                total: i32::try_from(total).unwrap_or(i32::MAX),
            });
        }

        let (rooms, total) = self.list_rooms_with_count(&query).await?;

        // Collect creator IDs for batch lookup
        let creator_ids: Vec<UserId> = rooms.iter().map(|r| r.room.created_by.clone()).collect();
        let usernames_map: std::collections::HashMap<UserId, String> = self.user_service.get_usernames(&creator_ids).await.unwrap_or_default();

        // Batch-load settings for all rooms (single query)
        let room_id_strs: Vec<&str> = rooms.iter().map(|r| r.room.id.as_str()).collect();
        let settings_map = self.room_settings_repo.get_batch(&room_id_strs).await.unwrap_or_default();

        // Convert rooms to AdminRoom format
        let admin_rooms: Vec<AdminRoom> = rooms
            .into_iter()
            .map(|r| {
                let settings = settings_map.get(r.room.id.as_str())
                    .cloned()
                    .unwrap_or_default();
                let settings_bytes = serde_json::to_vec(&settings).unwrap_or_default();
                let creator_username = usernames_map
                    .get(&r.room.created_by)
                    .cloned()
                    .unwrap_or_else(|| "Unknown".to_string());
                AdminRoom {
                    id: r.room.id.as_str().to_string(),
                    name: r.room.name,
                    description: r.room.description,
                    creator_id: r.room.created_by.as_str().to_string(),
                    creator_username,
                    status: r.room.status.as_str().to_string(),
                    settings: settings_bytes,
                    member_count: r.member_count,
                    created_at: r.room.created_at.timestamp(),
                    updated_at: r.room.updated_at.timestamp(),
                }
            })
            .collect();

        Ok(ListRoomsResponse {
            rooms: admin_rooms,
            total: i32::try_from(total).unwrap_or(i32::MAX),
        })
    }

    /// Delete room using gRPC types
    pub async fn delete_room_grpc(
        &self,
        request: DeleteRoomRequest,
        requesting_user_id: &UserId,
    ) -> Result<DeleteRoomResponse> {
        let room_id = RoomId::from_string(request.room_id);

        self.delete_room(room_id, requesting_user_id.clone()).await?;

        Ok(DeleteRoomResponse { success: true })
    }

    /// Set room password using gRPC types
    pub async fn set_room_password(
        &self,
        request: UpdateRoomPasswordRequest,
        requesting_user_id: &UserId,
    ) -> Result<UpdateRoomPasswordResponse> {
        let room_id = RoomId::from_string(request.room_id);

        // Check permission
        self.permission_service
            .check_permission(&room_id, requesting_user_id, PermissionBits::UPDATE_ROOM_SETTINGS)
            .await?;

        // Verify room exists
        let _room = self
            .room_repo
            .get_by_id(&room_id)
            .await?
            .ok_or_else(|| Error::NotFound("Room not found".to_string()))?;

        // Hash new password if provided
        let hashed_password = if request.new_password.is_empty() {
            None
        } else {
            Some(hash_password(&request.new_password).await?)
        };

        // Load current settings
        let mut settings = self.room_settings_repo.get(&room_id).await?;

        settings.require_password = crate::models::room_settings::RequirePassword(hashed_password.is_some());

        // Update password in room_settings table
        if let Some(pwd_hash) = &hashed_password {
            self.room_settings_repo.set(&room_id, "password", pwd_hash).await?;
        } else {
            // Remove password if clearing
            self.room_settings_repo.delete(&room_id, "password").await?;
        }

        // Update settings
        self.room_settings_repo.set_settings(&room_id, &settings).await?;

        Ok(UpdateRoomPasswordResponse { success: true })
    }

    // ========== Admin Operations ==========

    /// Update room status (admin use, bypasses permission checks)
    pub async fn update_room_status(&self, room_id: &RoomId, status: crate::models::RoomStatus) -> Result<Room> {
        self.room_repo.update_status(room_id, status).await
    }

    /// Update room directly (admin use, bypasses permission checks)
    pub async fn admin_update_room(&self, room: &Room) -> Result<Room> {
        self.room_repo.update(room).await
    }

    /// Delete room (admin use, bypasses permission checks)
    pub async fn admin_delete_room(&self, room_id: &RoomId) -> Result<()> {
        let _ = self.notification_service.notify_room_deleted(room_id).await;
        self.room_repo.delete(room_id).await?;
        Ok(())
    }

    /// Set room password (admin use, bypasses permission checks)
    pub async fn admin_set_room_password(
        &self,
        request: UpdateRoomPasswordRequest,
    ) -> Result<UpdateRoomPasswordResponse> {
        let room_id = RoomId::from_string(request.room_id);

        // Verify room exists
        let _room = self
            .room_repo
            .get_by_id(&room_id)
            .await?
            .ok_or_else(|| Error::NotFound("Room not found".to_string()))?;

        // Hash new password if provided
        let hashed_password = if request.new_password.is_empty() {
            None
        } else {
            Some(hash_password(&request.new_password).await?)
        };

        // Load current settings
        let mut settings = self.room_settings_repo.get(&room_id).await?;
        settings.require_password = crate::models::room_settings::RequirePassword(hashed_password.is_some());

        if let Some(pwd_hash) = &hashed_password {
            self.room_settings_repo.set(&room_id, "password", pwd_hash).await?;
        } else {
            self.room_settings_repo.delete(&room_id, "password").await?;
        }

        self.room_settings_repo.set_settings(&room_id, &settings).await?;

        Ok(UpdateRoomPasswordResponse { success: true })
    }

    // ========== Service Accessors ==========

    /// Get reference to media service
    #[must_use] 
    pub const fn media_service(&self) -> &MediaService {
        &self.media_service
    }

    // ========== Room Management ==========

    /// Approve a pending room
    ///
    /// Changes room status from pending to active.
    /// Only admins can approve rooms.
    pub async fn approve_room(&self, room_id: &RoomId) -> Result<Room> {
        let room = self.room_repo.get_by_id(room_id).await?
            .ok_or_else(|| Error::NotFound("Room not found".to_string()))?;

        if !room.status.is_pending() {
            return Err(Error::InvalidInput("Room is not pending approval".to_string()));
        }

        let updated_room = self.room_repo.update_status(room_id, RoomStatus::Active).await?;

        Ok(updated_room)
    }

    /// Ban a room
    ///
    /// Changes room status to banned.
    /// Only admins can ban rooms.
    pub async fn ban_room(&self, room_id: &RoomId) -> Result<Room> {
        let room = self.room_repo.get_by_id(room_id).await?
            .ok_or_else(|| Error::NotFound("Room not found".to_string()))?;

        if room.status.is_banned() {
            return Err(Error::InvalidInput("Room is already banned".to_string()));
        }

        let updated_room = self.room_repo.update_status(room_id, RoomStatus::Banned).await?;

        Ok(updated_room)
    }

    /// Get reference to playback service
    #[must_use] 
    pub const fn playback_service(&self) -> &PlaybackService {
        &self.playback_service
    }

    /// Get reference to member service
    #[must_use] 
    pub const fn member_service(&self) -> &MemberService {
        &self.member_service
    }

    /// Get reference to notification service
    #[must_use] 
    pub const fn notification_service(&self) -> &NotificationService {
        &self.notification_service
    }
}

#[cfg(test)]
mod tests {

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
