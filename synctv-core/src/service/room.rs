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
    repository::{RoomRepository, RoomMemberRepository, MediaRepository, PlaylistRepository, RoomPlaybackStateRepository, ChatRepository},
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
    providers_manager: Arc<ProvidersManager>,
}

impl std::fmt::Debug for RoomService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoomService").finish()
    }
}

impl RoomService {
    pub fn new(pool: PgPool, user_service: UserService) -> Self {
        // Initialize repositories
        let room_repo = RoomRepository::new(pool.clone());
        let member_repo = RoomMemberRepository::new(pool.clone());
        let media_repo = MediaRepository::new(pool.clone());
        let playlist_repo = PlaylistRepository::new(pool.clone());
        let playback_repo = RoomPlaybackStateRepository::new(pool.clone());
        let provider_instance_repo = Arc::new(crate::repository::ProviderInstanceRepository::new(pool.clone()));
        let chat_repo = ChatRepository::new(pool);

        // Initialize permission service with caching
        let permission_service = PermissionService::new(member_repo.clone(), 10000, 300);

        // Initialize provider instance manager and providers manager
        let provider_instance_manager = Arc::new(crate::service::ProviderInstanceManager::new(provider_instance_repo.clone()));
        let providers_manager = Arc::new(ProvidersManager::new(provider_instance_manager));

        // Initialize domain services
        let member_service = MemberService::new(member_repo.clone(), room_repo.clone(), permission_service.clone());
        let playlist_service = PlaylistService::new(playlist_repo.clone(), permission_service.clone());
        let media_service = MediaService::new(
            media_repo.clone(),
            playlist_repo.clone(),
            permission_service.clone(),
            providers_manager.clone(),
        );
        let playback_service = PlaybackService::new(playback_repo.clone(), permission_service.clone(), media_service.clone(), media_repo.clone());
        let notification_service = NotificationService::new();

        Self {
            room_repo,
            playback_repo,
            chat_repo,
            member_service,
            permission_service,
            playlist_service,
            media_service,
            playback_service,
            notification_service,
            user_service,
            providers_manager,
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

        // Notify room members with username
        let username = self.user_service.get_username(&user_id).await?.unwrap_or_else(|| "Unknown".to_string());
        let _ = self.notification_service.notify_user_joined(&room_id, &user_id, &username).await;

        Ok((room, created_member, members))
    }

    /// Leave a room
    pub async fn leave_room(&self, room_id: RoomId, user_id: UserId) -> Result<()> {
        self.member_service.remove_member(room_id.clone(), user_id.clone()).await?;

        // Notify room members with username
        let username = self.user_service.get_username(&user_id).await?.unwrap_or_else(|| "Unknown".to_string());
        let _ = self.notification_service.notify_user_left(&room_id, &user_id, &username).await;

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

    /// Add media to playlist (convenience method)
    ///
    /// This is a convenience method that:
    /// 1. Gets the root playlist for the room
    /// 2. Calls MediaService::add_media with the provided source_config
    ///
    /// Note: Clients should typically call the parse endpoint first to get
    /// source_config, then call this method with provider_instance_name.
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
            metadata: None,
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
        // TODO: Get room's root playlist and fetch media
        // For now, return empty vec as this needs to be implemented
        // let root_playlist = self.playlist_repo.get_root_playlist(room_id).await?;
        // self.media_service.get_playlist_media(&root_playlist.id).await
        Ok(Vec::new())
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

    // ========== gRPC-typed Methods (New Architecture) ==========

    /// Get room members using gRPC types
    ///
    /// This method demonstrates the new architecture where service layer
    /// uses gRPC-generated types, allowing both HTTP and gRPC layers to be
    /// lightweight wrappers.
    ///
    /// # Arguments
    /// * `request` - GetRoomMembersRequest containing room_id
    /// * `requesting_user_id` - The user making the request (for permission checking)
    ///
    /// # Returns
    /// GetRoomMembersResponse containing list of room members
    pub async fn get_room_members_grpc(
        &self,
        request: synctv_proto::GetRoomMembersRequest,
        requesting_user_id: &UserId,
    ) -> Result<synctv_proto::GetRoomMembersResponse> {
        // Extract room_id
        let room_id = RoomId::from_string(request.room_id.clone());

        // Check permission - user must be a member of the room to see members
        self.permission_service
            .check_permission(&room_id, requesting_user_id, PermissionBits::SEND_CHAT)
            .await?;

        // Get members from repository
        let members_with_users = self.member_service.list_members(&room_id).await?;

        // Convert to gRPC RoomMember type
        let proto_members: Vec<synctv_proto::RoomMember> = members_with_users
            .into_iter()
            .map(|m| synctv_proto::RoomMember {
                room_id: m.room_id.as_str().to_string(),
                user_id: m.user_id.as_str().to_string(),
                username: m.username,
                permissions: m.permissions.0,
                joined_at: m.joined_at.timestamp(),
                is_online: m.is_online,
            })
            .collect();

        Ok(synctv_proto::GetRoomMembersResponse {
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

        // Convert settings to bytes (protobuf serialization)
        let settings_bytes = serde_json::to_vec(&room.settings)
            .unwrap_or_default();

        let admin_room = AdminRoom {
            id: room.id.as_str().to_string(),
            name: room.name,
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
                "closed" => Some(RoomStatus::Closed),
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
            let (rooms, total) = self.list_rooms_by_creator_with_count(&creator_id, query.page as i64, query.page_size as i64).await?;

            // Collect creator IDs for batch lookup
            let creator_ids: Vec<UserId> = rooms.iter().map(|r| r.room.created_by.clone()).collect();
            let usernames_map: std::collections::HashMap<UserId, String> = self.user_service.get_usernames(&creator_ids).await.unwrap_or_default();

            // Convert rooms to AdminRoom format
            let admin_rooms: Vec<AdminRoom> = rooms
                .into_iter()
                .map(|r| {
                    let settings_bytes = serde_json::to_vec(&r.room.settings).unwrap_or_default();
                    let creator_username = usernames_map
                        .get(&r.room.created_by)
                        .cloned()
                        .unwrap_or_else(|| "Unknown".to_string());
                    AdminRoom {
                        id: r.room.id.as_str().to_string(),
                        name: r.room.name,
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
                total: total as i32,
            });
        }

        let (rooms, total) = self.list_rooms_with_count(&query).await?;

        // Collect creator IDs for batch lookup
        let creator_ids: Vec<UserId> = rooms.iter().map(|r| r.room.created_by.clone()).collect();
        let usernames_map: std::collections::HashMap<UserId, String> = self.user_service.get_usernames(&creator_ids).await.unwrap_or_default();

        // Convert rooms to AdminRoom format
        let admin_rooms: Vec<AdminRoom> = rooms
            .into_iter()
            .map(|r| {
                let settings_bytes = serde_json::to_vec(&r.room.settings).unwrap_or_default();
                let creator_username = usernames_map
                    .get(&r.room.created_by)
                    .cloned()
                    .unwrap_or_else(|| "Unknown".to_string());
                AdminRoom {
                    id: r.room.id.as_str().to_string(),
                    name: r.room.name,
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
            total: total as i32,
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

    /// Update room password using gRPC types
    pub async fn update_room_password_grpc(
        &self,
        request: UpdateRoomPasswordRequest,
        requesting_user_id: &UserId,
    ) -> Result<UpdateRoomPasswordResponse> {
        let room_id = RoomId::from_string(request.room_id);

        // Check permission
        self.permission_service
            .check_permission(&room_id, requesting_user_id, PermissionBits::UPDATE_ROOM_SETTINGS)
            .await?;

        // Get room
        let mut room = self
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

        // Update room settings with new password
        let mut settings = serde_json::from_value::<RoomSettings>(room.settings.clone())
            .unwrap_or_default();

        settings.require_password = hashed_password.is_some();

        room.settings = json!({
            "require_password": settings.require_password,
            "password": hashed_password,
            "auto_play_next": settings.auto_play_next,
            "loop_playlist": settings.loop_playlist,
            "shuffle_playlist": settings.shuffle_playlist,
            "allow_guest_join": settings.allow_guest_join,
            "max_members": settings.max_members,
            "chat_enabled": settings.chat_enabled,
            "danmaku_enabled": settings.danmaku_enabled,
        });

        self.room_repo.update(&room).await?;

        Ok(UpdateRoomPasswordResponse { success: true })
    }

    // ========== Admin Operations ==========

    /// Update room directly (admin use, bypasses permission checks)
    pub async fn admin_update_room(&self, room: &Room) -> Result<Room> {
        self.room_repo.update(room).await
    }

    // ========== Service Accessors ==========

    /// Get reference to media service
    pub fn media_service(&self) -> &MediaService {
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
    pub fn playback_service(&self) -> &PlaybackService {
        &self.playback_service
    }

    /// Get reference to member service
    pub fn member_service(&self) -> &MemberService {
        &self.member_service
    }

    /// Get reference to notification service
    pub fn notification_service(&self) -> &NotificationService {
        &self.notification_service
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
