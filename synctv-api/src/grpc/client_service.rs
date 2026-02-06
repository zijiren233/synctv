use std::collections::HashMap;
use std::sync::Arc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};

use synctv_cluster::sync::{ClusterEvent, ClusterManager, ConnectionManager};
use crate::impls::messaging::{StreamMessageHandler, MessageSender};
use synctv_core::models::{
    MediaId, PermissionBits, RoomId, RoomListQuery, RoomSettings, RoomStatus, UserId,
};
use synctv_core::service::{
    ContentFilter, RateLimitConfig, RateLimiter, RoomService as CoreRoomService,
    UserService as CoreUserService,
};

// Use synctv_proto for all gRPC traits and types
use crate::proto::client::{
    auth_service_server::AuthService, email_service_server::EmailService,
    media_service_server::MediaService, public_service_server::PublicService,
    room_service_server::RoomService, user_service_server::UserService, ServerMessage, server_message, ChatMessageReceive, UserJoinedRoom, RoomMember, UserLeftRoom, PlaybackStateChanged, PlaybackState, RoomSettingsChanged, RegisterRequest, RegisterResponse, User, LoginRequest, LoginResponse, RefreshTokenRequest, RefreshTokenResponse, LogoutRequest, LogoutResponse, GetProfileRequest, GetProfileResponse, SetUsernameRequest, SetUsernameResponse, SetPasswordRequest, SetPasswordResponse, ListCreatedRoomsRequest, ListCreatedRoomsResponse, Room, ListParticipatedRoomsRequest, ListParticipatedRoomsResponse, RoomWithRole, CreateRoomRequest, CreateRoomResponse, GetRoomRequest, GetRoomResponse, JoinRoomRequest, JoinRoomResponse, LeaveRoomRequest, LeaveRoomResponse, DeleteRoomRequest, DeleteRoomResponse, SetRoomSettingsRequest, SetRoomSettingsResponse, GetRoomMembersRequest, GetRoomMembersResponse, SetMemberPermissionRequest, SetMemberPermissionResponse, KickMemberRequest, KickMemberResponse, BanMemberRequest, BanMemberResponse, UnbanMemberRequest, UnbanMemberResponse, GetRoomSettingsRequest, GetRoomSettingsResponse, UpdateRoomSettingRequest, UpdateRoomSettingResponse, ResetRoomSettingsRequest, ResetRoomSettingsResponse, ClientMessage, GetChatHistoryRequest, GetChatHistoryResponse, AddMediaRequest, AddMediaResponse, Media, RemoveMediaRequest, RemoveMediaResponse, ListPlaylistRequest, ListPlaylistResponse, ListPlaylistItemsRequest, ListPlaylistItemsResponse, Playlist, SwapMediaRequest, SwapMediaResponse, PlayRequest, PlayResponse, PauseRequest, PauseResponse, SeekRequest, SeekResponse, ChangeSpeedRequest, ChangeSpeedResponse, SwitchMediaRequest, SwitchMediaResponse, GetPlaybackStateRequest, GetPlaybackStateResponse, NewPublishKeyRequest, NewPublishKeyResponse, CreatePlaylistRequest, CreatePlaylistResponse, SetPlaylistRequest, SetPlaylistResponse, DeletePlaylistRequest, DeletePlaylistResponse, ListPlaylistsRequest, ListPlaylistsResponse, SetPlayingRequest, SetPlayingResponse, CheckRoomRequest, CheckRoomResponse, ListRoomsRequest, ListRoomsResponse, GetHotRoomsRequest, GetHotRoomsResponse, RoomWithStats, GetPublicSettingsRequest, GetPublicSettingsResponse, SendVerificationEmailRequest, SendVerificationEmailResponse, ConfirmEmailRequest, ConfirmEmailResponse, RequestPasswordResetRequest, RequestPasswordResetResponse, ConfirmPasswordResetRequest, ConfirmPasswordResetResponse, GetIceServersRequest, GetIceServersResponse, IceServer, GetNetworkQualityRequest, GetNetworkQualityResponse,
    GetMovieInfoRequest, GetMovieInfoResponse,
};

/// Configuration for `ClientService`
#[derive(Clone)]
pub struct ClientServiceConfig {
    pub user_service: CoreUserService,
    pub room_service: CoreRoomService,
    pub cluster_manager: Arc<ClusterManager>,
    pub rate_limiter: RateLimiter,
    pub rate_limit_config: RateLimitConfig,
    pub content_filter: ContentFilter,
    pub connection_manager: ConnectionManager,
    pub email_service: Option<Arc<synctv_core::service::EmailService>>,
    pub email_token_service: Option<Arc<synctv_core::service::EmailTokenService>>,
    pub settings_registry: Option<Arc<synctv_core::service::SettingsRegistry>>,
    pub config: Arc<synctv_core::Config>,
}

/// `ClientService` implementation
#[derive(Clone)]
pub struct ClientServiceImpl {
    user_service: Arc<CoreUserService>,
    room_service: Arc<CoreRoomService>,
    cluster_manager: Arc<ClusterManager>,
    rate_limiter: Arc<RateLimiter>,
    rate_limit_config: Arc<RateLimitConfig>,
    content_filter: Arc<ContentFilter>,
    connection_manager: Arc<ConnectionManager>,
    email_service: Option<Arc<synctv_core::service::EmailService>>,
    email_token_service: Option<Arc<synctv_core::service::EmailTokenService>>,
    settings_registry: Option<Arc<synctv_core::service::SettingsRegistry>>,
    config: Arc<synctv_core::Config>,
}

impl ClientServiceImpl {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        user_service: CoreUserService,
        room_service: CoreRoomService,
        cluster_manager: Arc<ClusterManager>,
        rate_limiter: RateLimiter,
        rate_limit_config: RateLimitConfig,
        content_filter: ContentFilter,
        connection_manager: ConnectionManager,
        email_service: Option<Arc<synctv_core::service::EmailService>>,
        email_token_service: Option<Arc<synctv_core::service::EmailTokenService>>,
        settings_registry: Option<Arc<synctv_core::service::SettingsRegistry>>,
        config: Arc<synctv_core::Config>,
    ) -> Self {
        Self {
            user_service: Arc::new(user_service),
            room_service: Arc::new(room_service),
            cluster_manager,
            rate_limiter: Arc::new(rate_limiter),
            rate_limit_config: Arc::new(rate_limit_config),
            content_filter: Arc::new(content_filter),
            connection_manager: Arc::new(connection_manager),
            email_service,
            email_token_service,
            settings_registry,
            config,
        }
    }

    /// Create `ClientService` from configuration struct
    #[must_use]
    pub fn from_config(config: ClientServiceConfig) -> Self {
        Self {
            user_service: Arc::new(config.user_service),
            room_service: Arc::new(config.room_service),
            cluster_manager: config.cluster_manager,
            rate_limiter: Arc::new(config.rate_limiter),
            rate_limit_config: Arc::new(config.rate_limit_config),
            content_filter: Arc::new(config.content_filter),
            connection_manager: Arc::new(config.connection_manager),
            email_service: config.email_service,
            email_token_service: config.email_token_service,
            settings_registry: config.settings_registry,
            config: config.config,
        }
    }

    /// Extract `user_id` from `UserContext` (injected by `inject_user` interceptor)
    #[allow(clippy::result_large_err)]
    fn get_user_id(&self, request: &Request<impl std::fmt::Debug>) -> Result<UserId, Status> {
        let user_context = request
            .extensions()
            .get::<super::interceptors::UserContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;

        Ok(UserId::from_string(user_context.user_id.clone()))
    }

    /// Extract `RoomContext` (injected by `inject_room` interceptor)
    #[allow(clippy::result_large_err)]
    fn get_room_context(
        &self,
        request: &Request<impl std::fmt::Debug>,
    ) -> Result<super::interceptors::RoomContext, Status> {
        let room_context = request
            .extensions()
            .get::<super::interceptors::RoomContext>()
            .ok_or_else(|| Status::unauthenticated("Room context required"))?;

        Ok(room_context.clone())
    }

    /// Extract `room_id` from `RoomContext`
    #[allow(clippy::result_large_err)]
    fn get_room_id(&self, request: &Request<impl std::fmt::Debug>) -> Result<RoomId, Status> {
        let room_context = self.get_room_context(request)?;
        Ok(RoomId::from_string(room_context.room_id))
    }

    /// Convert a core Playlist model to the proto Playlist message
    fn playlist_to_proto(&self, playlist: &synctv_core::models::Playlist, item_count: i32) -> Playlist {
        Playlist {
            id: playlist.id.as_str().to_string(),
            room_id: playlist.room_id.as_str().to_string(),
            name: playlist.name.clone(),
            parent_id: playlist.parent_id.as_ref().map(|id| id.as_str().to_string()).unwrap_or_default(),
            position: playlist.position,
            is_folder: playlist.parent_id.is_none() || playlist.source_provider.is_some(),
            is_dynamic: playlist.is_dynamic(),
            item_count,
            created_at: playlist.created_at.timestamp(),
            updated_at: playlist.updated_at.timestamp(),
        }
    }

    /// Handle incoming client message from bidirectional stream
    #[allow(dead_code)]
    fn convert_event_to_server_message(event: ClusterEvent) -> Option<ServerMessage> {
        match event {
            ClusterEvent::ChatMessage {
                room_id,
                user_id,
                username,
                message,
                timestamp,
                position,
                color,
            } => Some(ServerMessage {
                message: Some(server_message::Message::Chat(ChatMessageReceive {
                    id: nanoid::nanoid!(12),
                    room_id: room_id.as_str().to_string(),
                    user_id: user_id.as_str().to_string(),
                    username,
                    content: message,
                    timestamp: timestamp.timestamp(),
                    position,
                    color,
                })),
            }),

            ClusterEvent::UserJoined {
                room_id,
                user_id,
                username,
                permissions,
                ..
            } => Some(ServerMessage {
                message: Some(server_message::Message::UserJoined(UserJoinedRoom {
                    room_id: room_id.as_str().to_string(),
                    member: Some(RoomMember {
                        room_id: room_id.as_str().to_string(),
                        user_id: user_id.as_str().to_string(),
                        username,
                        role: "member".to_string(),
                        permissions: permissions.0,
                        added_permissions: 0,
                        removed_permissions: 0,
                        admin_added_permissions: 0,
                        admin_removed_permissions: 0,
                        joined_at: chrono::Utc::now().timestamp(),
                        is_online: true,
                    }),
                })),
            }),

            ClusterEvent::UserLeft {
                room_id, user_id, ..
            } => Some(ServerMessage {
                message: Some(server_message::Message::UserLeft(UserLeftRoom {
                    room_id: room_id.as_str().to_string(),
                    user_id: user_id.as_str().to_string(),
                })),
            }),

            ClusterEvent::PlaybackStateChanged { room_id, state, .. } => Some(ServerMessage {
                message: Some(server_message::Message::PlaybackState(
                    PlaybackStateChanged {
                        room_id: room_id.as_str().to_string(),
                        state: Some(PlaybackState {
                            room_id: room_id.as_str().to_string(),
                            playing_media_id: state
                                .playing_media_id
                                .map(|id| id.as_str().to_string())
                                .unwrap_or_default(),
                            position: state.position,
                            speed: state.speed,
                            is_playing: state.is_playing,
                            updated_at: state.updated_at.timestamp(),
                            version: state.version,
                        }),
                    },
                )),
            }),

            ClusterEvent::RoomSettingsChanged { room_id, .. } => Some(ServerMessage {
                message: Some(server_message::Message::RoomSettings(RoomSettingsChanged {
                    room_id: room_id.as_str().to_string(),
                    // Settings are embedded in the room object, client should fetch room details
                    // to get updated settings. This event is just a notification.
                    settings: vec![],
                })),
            }),

            _ => None,
        }
    }
}

// ==================== AuthService Implementation ====================
#[tonic::async_trait]
impl AuthService for ClientServiceImpl {
    async fn register(
        &self,
        request: Request<RegisterRequest>,
    ) -> Result<Response<RegisterResponse>, Status> {
        let req = request.into_inner();

        // Validate input
        if req.username.is_empty() {
            return Err(Status::invalid_argument("Username is required"));
        }
        if req.email.is_empty() {
            return Err(Status::invalid_argument("Email is required"));
        }
        if req.password.is_empty() {
            return Err(Status::invalid_argument("Password is required"));
        }

        // Register user
        let (user, access_token, refresh_token) = self
            .user_service
            .register(req.username, Some(req.email), req.password)
            .await
            .map_err(|e| match e {
                synctv_core::Error::InvalidInput(msg) => Status::invalid_argument(msg),
                synctv_core::Error::Database(db_err) => {
                    tracing::error!("Database error during registration: {}", db_err);
                    Status::internal("Failed to register user")
                }
                _ => Status::internal("Registration failed"),
            })?;

        // Convert to proto User
        let role_str = match user.role {
            synctv_core::models::UserRole::Root => "root",
            synctv_core::models::UserRole::Admin => "admin",
            synctv_core::models::UserRole::User => "user",
        };

        let status_str = match user.status {
            synctv_core::models::UserStatus::Active => "active",
            synctv_core::models::UserStatus::Pending => "pending",
            synctv_core::models::UserStatus::Banned => "banned",
        };

        let proto_user = Some(User {
            id: user.id.as_str().to_string(),
            username: user.username,
            email: user.email.unwrap_or_default(),
            role: role_str.to_string(),
            status: status_str.to_string(),
            created_at: user.created_at.timestamp(),
        });

        Ok(Response::new(RegisterResponse {
            user: proto_user,
            access_token,
            refresh_token,
        }))
    }

    async fn login(
        &self,
        request: Request<LoginRequest>,
    ) -> Result<Response<LoginResponse>, Status> {
        let req = request.into_inner();

        // Validate input
        if req.username.is_empty() {
            return Err(Status::invalid_argument("Username is required"));
        }
        if req.password.is_empty() {
            return Err(Status::invalid_argument("Password is required"));
        }

        // Login user
        let (user, access_token, refresh_token) = self
            .user_service
            .login(req.username, req.password)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authentication(msg) => Status::unauthenticated(msg),
                synctv_core::Error::Database(db_err) => {
                    tracing::error!("Database error during login: {}", db_err);
                    Status::internal("Login failed")
                }
                _ => Status::internal("Login failed"),
            })?;

        // Convert to proto User
        let role_str = match user.role {
            synctv_core::models::UserRole::Root => "root",
            synctv_core::models::UserRole::Admin => "admin",
            synctv_core::models::UserRole::User => "user",
        };

        let status_str = match user.status {
            synctv_core::models::UserStatus::Active => "active",
            synctv_core::models::UserStatus::Pending => "pending",
            synctv_core::models::UserStatus::Banned => "banned",
        };

        let proto_user = Some(User {
            id: user.id.as_str().to_string(),
            username: user.username,
            email: user.email.unwrap_or_default(),
            role: role_str.to_string(),
            status: status_str.to_string(),
            created_at: user.created_at.timestamp(),
        });

        Ok(Response::new(LoginResponse {
            user: proto_user,
            access_token,
            refresh_token,
        }))
    }

    async fn refresh_token(
        &self,
        request: Request<RefreshTokenRequest>,
    ) -> Result<Response<RefreshTokenResponse>, Status> {
        let req = request.into_inner();

        // Validate input
        if req.refresh_token.is_empty() {
            return Err(Status::invalid_argument("Refresh token is required"));
        }

        // Refresh token
        let (access_token, refresh_token) = self
            .user_service
            .refresh_token(req.refresh_token)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authentication(msg) => Status::unauthenticated(msg),
                _ => Status::internal("Token refresh failed"),
            })?;

        Ok(Response::new(RefreshTokenResponse {
            access_token,
            refresh_token,
        }))
    }
}

// ==================== UserService Implementation ====================
#[tonic::async_trait]
impl UserService for ClientServiceImpl {
    async fn logout(
        &self,
        request: Request<LogoutRequest>,
    ) -> Result<Response<LogoutResponse>, Status> {
        // Extract token from Authorization header in metadata
        let token = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| {
                if s.starts_with("Bearer ") || s.starts_with("bearer ") {
                    Some(&s[7..])
                } else {
                    None
                }
            })
            .ok_or_else(|| Status::unauthenticated("Missing or invalid authorization header"))?;

        // Blacklist the token
        self.user_service
            .logout(token)
            .await
            .map_err(|e| Status::internal(format!("Failed to logout: {e}")))?;

        Ok(Response::new(LogoutResponse { success: true }))
    }

    async fn get_profile(
        &self,
        request: Request<GetProfileRequest>,
    ) -> Result<Response<GetProfileResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;

        // Get user from service
        let user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get user: {e}")))?;

        Ok(Response::new(GetProfileResponse {
            user: Some(User {
                id: user.id.to_string(),
                username: user.username,
                email: user.email.unwrap_or_default(),
                role: user.role.to_string(),
                status: user.status.as_str().to_string(),
                created_at: user.created_at.timestamp(),
            }),
        }))
    }

    async fn set_username(
        &self,
        request: Request<SetUsernameRequest>,
    ) -> Result<Response<SetUsernameResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        // Validate new username
        if req.new_username.is_empty() {
            return Err(Status::invalid_argument("Username cannot be empty"));
        }

        if req.new_username.len() < 3 || req.new_username.len() > 32 {
            return Err(Status::invalid_argument(
                "Username must be between 3 and 32 characters",
            ));
        }

        // Get current user
        let mut user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get user: {e}")))?;

        // Update username
        user.username = req.new_username;

        // Save to database
        let updated_user = self
            .user_service
            .update_user(&user)
            .await
            .map_err(|e| Status::internal(format!("Failed to update username: {e}")))?;

        // Convert to proto format
        let proto_user = User {
            id: updated_user.id.to_string(),
            username: updated_user.username,
            email: updated_user.email.unwrap_or_default(),
            role: updated_user.role.to_string(),
            status: updated_user.status.as_str().to_string(),
            created_at: updated_user.created_at.timestamp(),
        };

        Ok(Response::new(SetUsernameResponse {
            user: Some(proto_user),
        }))
    }

    async fn set_password(
        &self,
        request: Request<SetPasswordRequest>,
    ) -> Result<Response<SetPasswordResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        // Validate new password
        if req.new_password.is_empty() {
            return Err(Status::invalid_argument("New password cannot be empty"));
        }

        if req.new_password.len() < 6 {
            return Err(Status::invalid_argument(
                "Password must be at least 6 characters",
            ));
        }

        // Get current user
        let mut user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get user: {e}")))?;

        // Verify old password
        if !synctv_core::service::auth::password::verify_password(
            &req.old_password,
            &user.password_hash,
        )
        .await
        .map_err(|e| Status::internal(format!("Failed to verify password: {e}")))?
        {
            return Err(Status::permission_denied("Invalid old password"));
        }

        // Hash new password
        let new_hash = synctv_core::service::auth::password::hash_password(&req.new_password)
            .await
            .map_err(|e| Status::internal(format!("Failed to hash password: {e}")))?;

        // Update password
        user.password_hash = new_hash;

        // Save to database
        self.user_service
            .update_user(&user)
            .await
            .map_err(|e| Status::internal(format!("Failed to update password: {e}")))?;

        Ok(Response::new(SetPasswordResponse { success: true }))
    }

    async fn list_created_rooms(
        &self,
        request: Request<ListCreatedRoomsRequest>,
    ) -> Result<Response<ListCreatedRoomsResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        let page = if req.page == 0 { 1 } else { i64::from(req.page) };
        let page_size = if req.page_size == 0 || req.page_size > 50 {
            10
        } else {
            i64::from(req.page_size)
        };

        // Get rooms created by user with member count (optimized single query)
        let (rooms_with_count, total) = self
            .room_service
            .list_rooms_by_creator_with_count(&user_id, page, page_size)
            .await
            .map_err(|e| Status::internal(format!("Failed to get rooms: {e}")))?;

        // Convert to proto format
        let mut room_protos: Vec<Room> = Vec::new();
        for rwc in rooms_with_count {
            // Load room settings
            let settings = self.room_service
                .get_room_settings(&rwc.room.id)
                .await
                .unwrap_or_default();

            room_protos.push(Room {
                id: rwc.room.id.to_string(),
                name: rwc.room.name,
                created_by: rwc.room.created_by.to_string(),
                status: match rwc.room.status {
                    RoomStatus::Pending => "pending".to_string(),

                    RoomStatus::Active => "active".to_string(),
                    RoomStatus::Banned => "banned".to_string(),
                },
                settings: serde_json::to_vec(&settings).unwrap_or_default(),
                created_at: rwc.room.created_at.timestamp(),
                member_count: rwc.member_count,
            });
        }

        Ok(Response::new(ListCreatedRoomsResponse {
            rooms: room_protos,
            total: total as i32,
        }))
    }

    async fn list_participated_rooms(
        &self,
        request: Request<ListParticipatedRoomsRequest>,
    ) -> Result<Response<ListParticipatedRoomsResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        let page = if req.page == 0 { 1 } else { i64::from(req.page) };
        let page_size = if req.page_size == 0 || req.page_size > 50 {
            10
        } else {
            i64::from(req.page_size)
        };

        // Get rooms where user is a member with full details (optimized single query)
        let (rooms_with_details, total) = self
            .room_service
            .list_joined_rooms_with_details(&user_id, page, page_size)
            .await
            .map_err(|e| Status::internal(format!("Failed to get joined rooms: {e}")))?;

        // Convert to proto format
        let mut room_with_roles: Vec<RoomWithRole> = Vec::new();
        for (room, role, _status, member_count) in rooms_with_details {
            // Load room settings
            let settings = self.room_service
                .get_room_settings(&room.id)
                .await
                .unwrap_or_default();

            // Convert RoomRole to string
            let role_str = match role {
                synctv_core::models::RoomRole::Creator => "creator",
                synctv_core::models::RoomRole::Admin => "admin",
                synctv_core::models::RoomRole::Member => "member",
                synctv_core::models::RoomRole::Guest => "guest",
            };

            let room_proto = Room {
                id: room.id.to_string(),
                name: room.name,
                created_by: room.created_by.to_string(),
                status: match room.status {
                    RoomStatus::Pending => "pending".to_string(),

                    RoomStatus::Active => "active".to_string(),
                    RoomStatus::Banned => "banned".to_string(),
                },
                settings: serde_json::to_vec(&settings).unwrap_or_default(),
                created_at: room.created_at.timestamp(),
                member_count,
            };

            room_with_roles.push(RoomWithRole {
                room: Some(room_proto),
                permissions: role.permissions().0,
                role: role_str.to_string(),
            });
        }

        Ok(Response::new(ListParticipatedRoomsResponse {
            rooms: room_with_roles,
            total: total as i32,
        }))
    }
}

// ==================== RoomService Implementation ====================
#[tonic::async_trait]
impl RoomService for ClientServiceImpl {
    async fn create_room(
        &self,
        request: Request<CreateRoomRequest>,
    ) -> Result<Response<CreateRoomResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        // Validate input
        if req.name.is_empty() {
            return Err(Status::invalid_argument("Room name is required"));
        }

        // Parse settings
        let settings = if req.settings.is_empty() {
            None
        } else {
            Some(
                serde_json::from_slice(&req.settings)
                    .map_err(|e| Status::invalid_argument(format!("Invalid settings: {e}")))?,
            )
        };

        // Parse password
        let password = if req.password.is_empty() {
            None
        } else {
            Some(req.password)
        };

        // Create room
        let (room, _member) = self
            .room_service
            .create_room(req.name, user_id, password, settings)
            .await
            .map_err(|e| match e {
                synctv_core::Error::InvalidInput(msg) => Status::invalid_argument(msg),
                _ => Status::internal("Failed to create room"),
            })?;

        // Load room settings
        let settings = self.room_service
            .get_room_settings(&room.id)
            .await
            .unwrap_or_default();

        // Convert to proto Room
        let proto_room = Some(Room {
            id: room.id.as_str().to_string(),
            name: room.name,
            created_by: room.created_by.as_str().to_string(),
            status: room.status.as_str().to_string(),
            settings: serde_json::to_vec(&settings).unwrap_or_default(),
            created_at: room.created_at.timestamp(),
            member_count: 1,
        });

        Ok(Response::new(CreateRoomResponse { room: proto_room }))
    }

    async fn get_room(
        &self,
        request: Request<GetRoomRequest>,
    ) -> Result<Response<GetRoomResponse>, Status> {
        let req = request.into_inner();

        let room_id = RoomId::from_string(req.room_id);

        // Get room
        let room = self
            .room_service
            .get_room(&room_id)
            .await
            .map_err(|e| match e {
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                _ => Status::internal("Failed to get room"),
            })?;

        // Get playback state
        let playback_state = self
            .room_service
            .get_playback_state(&room_id)
            .await
            .map_err(|_| Status::internal("Failed to get playback state"))?;

        // Get member count
        let member_count = self
            .room_service
            .get_member_count(&room_id)
            .await
            .unwrap_or(0);

        // Load room settings
        let settings = self.room_service
            .get_room_settings(&room_id)
            .await
            .unwrap_or_default();

        // Convert to proto
        let proto_room = Some(Room {
            id: room.id.as_str().to_string(),
            name: room.name,
            created_by: room.created_by.as_str().to_string(),
            status: room.status.as_str().to_string(),
            settings: serde_json::to_vec(&settings).unwrap_or_default(),
            created_at: room.created_at.timestamp(),
            member_count,
        });

        let proto_playback = Some(PlaybackState {
            room_id: playback_state.room_id.as_str().to_string(),
            playing_media_id: playback_state
                .playing_media_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            position: playback_state.position,
            speed: playback_state.speed,
            is_playing: playback_state.is_playing,
            updated_at: playback_state.updated_at.timestamp(),
            version: playback_state.version,
        });

        Ok(Response::new(GetRoomResponse {
            room: proto_room,
            playback_state: proto_playback,
        }))
    }

    async fn join_room(
        &self,
        request: Request<JoinRoomRequest>,
    ) -> Result<Response<JoinRoomResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Join room
        let password = if req.password.is_empty() {
            None
        } else {
            Some(req.password)
        };

        let (room, _member, members) = self
            .room_service
            .join_room(room_id.clone(), user_id, password)
            .await
            .map_err(|e| match e {
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                synctv_core::Error::InvalidInput(msg) => Status::invalid_argument(msg),
                _ => Status::internal("Failed to join room"),
            })?;

        // Get playback state
        let playback_state = self
            .room_service
            .get_playback_state(&room_id)
            .await
            .map_err(|_| Status::internal("Failed to get playback state"))?;

        // Load room settings
        let settings = self.room_service
            .get_room_settings(&room_id)
            .await
            .unwrap_or_default();

        // Convert to proto
        let proto_room = Some(Room {
            id: room.id.as_str().to_string(),
            name: room.name,
            created_by: room.created_by.as_str().to_string(),
            status: room.status.as_str().to_string(),
            settings: serde_json::to_vec(&settings).unwrap_or_default(),
            created_at: room.created_at.timestamp(),
            member_count: members.len() as i32,
        });

        let proto_playback = Some(PlaybackState {
            room_id: playback_state.room_id.as_str().to_string(),
            playing_media_id: playback_state
                .playing_media_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            position: playback_state.position,
            speed: playback_state.speed,
            is_playing: playback_state.is_playing,
            updated_at: playback_state.updated_at.timestamp(),
            version: playback_state.version,
        });

        let proto_members: Vec<RoomMember> = members
            .into_iter()
            .map(|m| {
                let role_str = match m.role {
                    synctv_core::models::RoomRole::Creator => "creator",
                    synctv_core::models::RoomRole::Admin => "admin",
                    synctv_core::models::RoomRole::Member => "member",
                    synctv_core::models::RoomRole::Guest => "guest",
                };
                RoomMember {
                    room_id: m.room_id.as_str().to_string(),
                    user_id: m.user_id.as_str().to_string(),
                    username: m.username.clone(),
                    role: role_str.to_string(),
                    permissions: m.effective_permissions(synctv_core::models::PermissionBits::empty()).0,
                    added_permissions: m.added_permissions,
                    removed_permissions: m.removed_permissions,
                    admin_added_permissions: m.admin_added_permissions,
                    admin_removed_permissions: m.admin_removed_permissions,
                    joined_at: m.joined_at.timestamp(),
                    is_online: m.is_online,
                }
            })
            .collect();

        Ok(Response::new(JoinRoomResponse {
            room: proto_room,
            playback_state: proto_playback,
            members: proto_members,
        }))
    }

    async fn leave_room(
        &self,
        request: Request<LeaveRoomRequest>,
    ) -> Result<Response<LeaveRoomResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Leave room
        self.room_service
            .leave_room(room_id, user_id)
            .await
            .map_err(|e| match e {
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                _ => Status::internal("Failed to leave room"),
            })?;

        Ok(Response::new(LeaveRoomResponse { success: true }))
    }

    async fn delete_room(
        &self,
        request: Request<DeleteRoomRequest>,
    ) -> Result<Response<DeleteRoomResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Delete room
        self.room_service
            .delete_room(room_id, user_id)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                _ => Status::internal("Failed to delete room"),
            })?;

        Ok(Response::new(DeleteRoomResponse { success: true }))
    }

    async fn set_room_settings(
        &self,
        request: Request<SetRoomSettingsRequest>,
    ) -> Result<Response<SetRoomSettingsResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Parse settings from JSON bytes
        let settings = if req.settings.is_empty() {
            RoomSettings::default()
        } else {
            serde_json::from_slice(&req.settings)
                .map_err(|e| Status::invalid_argument(format!("Invalid settings: {e}")))?
        };

        // Set settings
        let updated_room = self
            .room_service
            .set_settings(room_id.clone(), user_id, settings)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                _ => Status::internal("Failed to set room settings"),
            })?;

        // Get member count
        let member_count = self
            .room_service
            .get_member_count(&room_id)
            .await
            .unwrap_or(0);

        // Load updated settings
        let room_settings = self.room_service
            .get_room_settings(&room_id)
            .await
            .unwrap_or_default();

        Ok(Response::new(SetRoomSettingsResponse {
            room: Some(Room {
                id: updated_room.id.to_string(),
                name: updated_room.name,
                created_by: updated_room.created_by.to_string(),
                status: match updated_room.status {
                    RoomStatus::Pending => "pending".to_string(),

                    RoomStatus::Active => "active".to_string(),
                    RoomStatus::Banned => "banned".to_string(),
                },
                settings: serde_json::to_vec(&room_settings).unwrap_or_default(),
                created_at: updated_room.created_at.timestamp(),
                member_count,
            }),
        }))
    }

//     async fn get_room_settings(
//         &self,
//         request: Request<GetRoomSettingsRequest>,
//     ) -> Result<Response<GetRoomSettingsResponse>, Status> {
//         let req = request.into_inner();
//         let room_id = RoomId::from_string(req.room_id);
// 
//         // Get settings (with caching)
//         let settings = self
//             .room_service
//             .get_room_settings(&room_id)
//             .await
//             .map_err(|e| Status::internal(format!("Failed to get settings: {}", e)))?;
// 
//         let settings_bytes = serde_json::to_vec(&settings)
//             .map_err(|e| Status::internal(format!("Failed to serialize settings: {}", e)))?;
// 
//         Ok(Response::new(GetRoomSettingsResponse {
//             settings: settings_bytes,
//         }))
//     }
// 
//     async fn update_room_setting(
//         &self,
//         request: Request<UpdateRoomSettingRequest>,
//     ) -> Result<Response<UpdateRoomSettingResponse>, Status> {
//         // Extract user_id from JWT token
//         let user_id = self.get_user_id(&request)?;
//         let req = request.into_inner();
//         let room_id = RoomId::from_string(req.room_id);
// 
//         // Get current settings
//         let mut settings = self
//             .room_service
//             .get_room_settings(&room_id)
//             .await
//             .map_err(|e| Status::internal(format!("Failed to get settings: {}", e)))?;
// 
//         // Update specific field based on key
//         match req.key.as_str() {
//             "require_password" => {
//                 if let Ok(value) = serde_json::from_slice::<bool>(&req.value) {
//                     settings.require_password = value;
//                 }
//             }
//             "auto_play_next" => {
//                 if let Ok(value) = serde_json::from_slice::<bool>(&req.value) {
//                     settings.auto_play_next = value;
//                 }
//             }
//             "auto_play" => {
//                 if let Ok(value) = serde_json::from_slice::<synctv_core::models::AutoPlaySettings>(&req.value) {
//                     settings.auto_play = value;
//                 }
//             }
//             "loop_playlist" => {
//                 if let Ok(value) = serde_json::from_slice::<bool>(&req.value) {
//                     settings.loop_playlist = value;
//                 }
//             }
//             "shuffle_playlist" => {
//                 if let Ok(value) = serde_json::from_slice::<bool>(&req.value) {
//                     settings.shuffle_playlist = value;
//                 }
//             }
//             "allow_guest_join" => {
//                 if let Ok(value) = serde_json::from_slice::<bool>(&req.value) {
//                     settings.allow_guest_join = value;
//                 }
//             }
//             "max_members" => {
//                 if let Ok(value) = serde_json::from_slice::<i32>(&req.value) {
//                     settings.max_members = Some(value);
//                 }
//             }
//             "chat_enabled" => {
//                 if let Ok(value) = serde_json::from_slice::<bool>(&req.value) {
//                     settings.chat_enabled = value;
//                 }
//             }
//             "danmaku_enabled" => {
//                 if let Ok(value) = serde_json::from_slice::<bool>(&req.value) {
//                     settings.danmaku_enabled = value;
//                 }
//             }
//             "require_approval" => {
//                 if let Ok(value) = serde_json::from_slice::<bool>(&req.value) {
//                     settings.require_approval = value;
//                 }
//             }
//             "allow_auto_join" => {
//                 if let Ok(value) = serde_json::from_slice::<bool>(&req.value) {
//                     settings.allow_auto_join = value;
//                 }
//             }
//             _ => {
//                 return Err(Status::invalid_argument(format!("Unknown setting key: {}", req.key)));
//             }
//         }
// 
//         // Save updated settings
//         self.room_service
//             .set_settings(room_id.clone(), user_id, settings)
//             .await
//             .map_err(|e| match e {
//                 synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
//                 synctv_core::Error::NotFound(msg) => Status::not_found(msg),
//                 _ => Status::internal("Failed to update setting"),
//             })?;
// 
//         // Load updated settings
//         let updated_settings = self
//             .room_service
//             .get_room_settings(&room_id)
//             .await
//             .map_err(|e| Status::internal(format!("Failed to get updated settings: {}", e)))?;
// 
//         let settings_bytes = serde_json::to_vec(&updated_settings)
//             .map_err(|e| Status::internal(format!("Failed to serialize settings: {}", e)))?;
// 
//         Ok(Response::new(UpdateRoomSettingResponse {
//             settings: settings_bytes,
//         }))
//     }
// 
//     async fn reset_room_settings(
//         &self,
//         request: Request<ResetRoomSettingsRequest>,
//     ) -> Result<Response<ResetRoomSettingsResponse>, Status> {
//         // Extract user_id from JWT token
//         let user_id = self.get_user_id(&request)?;
//         let req = request.into_inner();
//         let room_id = RoomId::from_string(req.room_id);
// 
//         // Reset to default
//         let default_settings = synctv_core::models::RoomSettings::default();
// 
//         self.room_service
//             .set_settings(room_id.clone(), user_id, default_settings)
//             .await
//             .map_err(|e| match e {
//                 synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
//                 synctv_core::Error::NotFound(msg) => Status::not_found(msg),
//                 _ => Status::internal("Failed to reset settings"),
//             })?;
// 
//         let settings_bytes = serde_json::to_vec(&default_settings)
//             .map_err(|e| Status::internal(format!("Failed to serialize settings: {}", e)))?;
// 
//         Ok(Response::new(ResetRoomSettingsResponse {
//             settings: settings_bytes,
//         }))
//     }

    async fn get_room_members(
        &self,
        request: Request<GetRoomMembersRequest>,
    ) -> Result<Response<GetRoomMembersResponse>, Status> {
        let room_id = self.get_room_id(&request)?;

        // Get members
        let members = self
            .room_service
            .get_room_members(&room_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get room members: {e}")))?;

        // Convert to response
        let member_list = members
            .into_iter()
            .map(|m| {
                let role_str = match m.role {
                    synctv_core::models::RoomRole::Creator => "creator",
                    synctv_core::models::RoomRole::Admin => "admin",
                    synctv_core::models::RoomRole::Member => "member",
                    synctv_core::models::RoomRole::Guest => "guest",
                };
                RoomMember {
                    room_id: room_id.to_string(),
                    user_id: m.user_id.to_string(),
                    username: m.username.clone(),
                    role: role_str.to_string(),
                    permissions: m.effective_permissions(synctv_core::models::PermissionBits::empty()).0,
                    added_permissions: m.added_permissions,
                    removed_permissions: m.removed_permissions,
                    admin_added_permissions: m.admin_added_permissions,
                    admin_removed_permissions: m.admin_removed_permissions,
                    joined_at: m.joined_at.timestamp(),
                    is_online: m.is_online,
                }
            })
            .collect();

        Ok(Response::new(GetRoomMembersResponse {
            members: member_list,
        }))
    }

    async fn set_member_permission(
        &self,
        request: Request<SetMemberPermissionRequest>,
    ) -> Result<Response<SetMemberPermissionResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();
        let target_user_id = UserId::from_string(req.user_id);

        // Determine which permissions to set based on the request
        // Priority: admin_* fields take precedence over regular fields
        let use_admin = req.admin_added_permissions > 0 || req.admin_removed_permissions > 0;

        let added = if use_admin {
            req.admin_added_permissions
        } else {
            req.added_permissions
        };

        let removed = if use_admin {
            req.admin_removed_permissions
        } else {
            req.removed_permissions
        };

        // Set member permissions
        let member = self
            .room_service
            .set_member_permission(
                room_id.clone(),
                user_id,
                target_user_id.clone(),
                added,
                removed,
            )
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                _ => Status::internal("Failed to set member permission"),
            })?;

        // Get username
        let username = self
            .user_service
            .get_user(&target_user_id)
            .await
            .map(|u| u.username)
            .unwrap_or_default();

        // Convert role to string
        let role_str = match member.role {
            synctv_core::models::RoomRole::Creator => "creator",
            synctv_core::models::RoomRole::Admin => "admin",
            synctv_core::models::RoomRole::Member => "member",
            synctv_core::models::RoomRole::Guest => "guest",
        };

        Ok(Response::new(SetMemberPermissionResponse {
            member: Some(RoomMember {
                room_id: room_id.to_string(),
                user_id: member.user_id.to_string(),
                username,
                role: role_str.to_string(),
                permissions: member.effective_permissions(synctv_core::models::PermissionBits::empty()).0,
                added_permissions: member.added_permissions,
                removed_permissions: member.removed_permissions,
                admin_added_permissions: member.admin_added_permissions,
                admin_removed_permissions: member.admin_removed_permissions,
                joined_at: member.joined_at.timestamp(),
                is_online: false,
            }),
        }))
    }

    async fn kick_member(
        &self,
        request: Request<KickMemberRequest>,
    ) -> Result<Response<KickMemberResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();
        let target_user_id = UserId::from_string(req.user_id);

        // Kick member
        self.room_service
            .kick_member(room_id, user_id, target_user_id)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                synctv_core::Error::InvalidInput(msg) => Status::invalid_argument(msg),
                _ => Status::internal("Failed to kick member"),
            })?;

        Ok(Response::new(KickMemberResponse { success: true }))
    }

    async fn ban_member(
        &self,
        request: Request<BanMemberRequest>,
    ) -> Result<Response<BanMemberResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();
        let target_uid = UserId::from_string(req.user_id);
        let reason = if req.reason.is_empty() { None } else { Some(req.reason) };

        self.room_service
            .member_service()
            .ban_member(room_id, user_id, target_uid, reason)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                _ => Status::internal(format!("Failed to ban member: {e}")),
            })?;

        Ok(Response::new(BanMemberResponse { success: true }))
    }

    async fn unban_member(
        &self,
        request: Request<UnbanMemberRequest>,
    ) -> Result<Response<UnbanMemberResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();
        let target_uid = UserId::from_string(req.user_id);

        self.room_service
            .member_service()
            .unban_member(room_id, user_id, target_uid)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                _ => Status::internal(format!("Failed to unban member: {e}")),
            })?;

        Ok(Response::new(UnbanMemberResponse { success: true }))
    }

    async fn get_room_settings(
        &self,
        request: Request<GetRoomSettingsRequest>,
    ) -> Result<Response<GetRoomSettingsResponse>, Status> {
        let room_id = self.get_room_id(&request)?;

        // Get room settings
        let settings = self
            .room_service
            .get_room_settings(&room_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get room settings: {e}")))?;

        // Serialize settings to JSON bytes
        let settings_json = serde_json::to_vec(&settings)
            .map_err(|e| Status::internal(format!("Failed to serialize settings: {e}")))?;

        Ok(Response::new(GetRoomSettingsResponse {
            settings: settings_json,
        }))
    }

    async fn update_room_setting(
        &self,
        request: Request<UpdateRoomSettingRequest>,
    ) -> Result<Response<UpdateRoomSettingResponse>, Status> {
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();

        // Parse the value as JSON
        let value: serde_json::Value = serde_json::from_slice(&req.value)
            .map_err(|e| Status::invalid_argument(format!("Invalid JSON value: {e}")))?;

        // Update single setting
        let settings_json = self
            .room_service
            .update_room_setting(&room_id, &req.key, &value)
            .await
            .map_err(|e| Status::internal(format!("Failed to update room setting: {e}")))?;

        Ok(Response::new(UpdateRoomSettingResponse {
            settings: settings_json.into_bytes(),
        }))
    }

    async fn reset_room_settings(
        &self,
        request: Request<ResetRoomSettingsRequest>,
    ) -> Result<Response<ResetRoomSettingsResponse>, Status> {
        let room_id = self.get_room_id(&request)?;

        // Reset room settings to default
        let settings_json = self
            .room_service
            .reset_room_settings(&room_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to reset room settings: {e}")))?;

        Ok(Response::new(ResetRoomSettingsResponse {
            settings: settings_json.into_bytes(),
        }))
    }

    type MessageStreamStream = std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<ServerMessage, Status>> + Send + 'static>,
    >;

    async fn message_stream(
        &self,
        request: Request<tonic::Streaming<ClientMessage>>,
    ) -> Result<Response<Self::MessageStreamStream>, Status> {
        use nanoid::nanoid;
        use tokio::sync::mpsc;

        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;

        // Extract room_id from metadata
        let room_id = self.get_room_id(&request)?;

        // Get user details from service
        let user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get user: {e}")))?;
        let username = user.username;

        // Generate unique connection ID
        let connection_id = nanoid!(16);

        tracing::info!(
            user_id = %user_id.as_str(),
            room_id = %room_id.as_str(),
            connection_id = %connection_id,
            "Client establishing MessageStream connection"
        );

        // Register connection with connection manager
        if let Err(e) = self
            .connection_manager
            .register(connection_id.clone(), user_id.clone())
        {
            tracing::warn!(
                user_id = %user_id.as_str(),
                error = %e,
                "Connection rejected by connection manager"
            );
            return Err(Status::resource_exhausted(e));
        }

        // Register with room
        if let Err(e) = self
            .connection_manager
            .join_room(&connection_id, room_id.clone())
        {
            return Err(Status::resource_exhausted(format!(
                "Cannot join room: {e}"
            )));
        }

        let mut client_stream = request.into_inner();

        // Create channel for outgoing messages
        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel::<ServerMessage>();

        // Create gRPC message sender
        let grpc_sender = Arc::new(GrpcMessageSender::new(outgoing_tx.clone()));

        // Create StreamMessageHandler with all configuration
        let stream_handler = StreamMessageHandler::new(
            room_id.clone(),
            user_id.clone(),
            username.clone(),
            self.room_service.clone(),
            self.cluster_manager.clone(),
            (*self.connection_manager).clone(),
            self.rate_limiter.clone(),
            self.rate_limit_config.clone(),
            self.content_filter.clone(),
            grpc_sender,
        );

        // Start the handler and get sender for client messages
        let client_msg_tx = stream_handler.start();

        // Clone for cleanup task
        let connection_id_clone = connection_id;
        let connection_manager_clone = self.connection_manager.clone();
        let cluster_manager_clone = self.cluster_manager.clone();
        let room_id_clone = room_id.clone();
        let user_id_clone = user_id.clone();
        let username_clone = username;
        let _outgoing_tx_clone = outgoing_tx;

        // Spawn task to handle incoming client messages
        tokio::spawn(async move {
            while let Ok(Some(client_msg)) = client_stream.message().await {
                if let Err(e) = client_msg_tx.send(client_msg) {
                    tracing::error!("Failed to send message to handler: {}", e);
                    break;
                }
            }

            // Client disconnected, cleanup
            cluster_manager_clone.unsubscribe(&connection_id_clone);

            // Notify other users that this user left
            let event = ClusterEvent::UserLeft {
                room_id: room_id_clone.clone(),
                user_id: user_id_clone.clone(),
                username: username_clone.clone(),
                timestamp: chrono::Utc::now(),
            };
            cluster_manager_clone.broadcast(event);

            // Unregister connection from connection manager
            connection_manager_clone.unregister(&connection_id_clone);

            tracing::info!(
                user_id = %user_id_clone.as_str(),
                connection_id = %connection_id_clone,
                "Client disconnected from MessageStream"
            );
        });

        // Convert outgoing channel to stream, wrapping items in Ok()
        let output_stream = UnboundedReceiverStream::new(outgoing_rx).map(Ok::<_, Status>);

        Ok(Response::new(
            Box::pin(output_stream) as Self::MessageStreamStream
        ))
    }

    async fn get_chat_history(
        &self,
        request: Request<GetChatHistoryRequest>,
    ) -> Result<Response<GetChatHistoryResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();

        // Check if user has access to the room (is a member)
        self.room_service
            .check_membership(&room_id, &user_id)
            .await
            .map_err(|e| Status::permission_denied(format!("Not a member of the room: {e}")))?;

        // Get chat history from database
        let limit = if req.limit == 0 || req.limit > 100 {
            50
        } else {
            req.limit
        };

        // Parse before timestamp if provided
        let before = if req.before > 0 {
            Some(chrono::DateTime::from_timestamp(req.before, 0).unwrap_or_else(chrono::Utc::now))
        } else {
            None
        };

        let messages = self
            .room_service
            .get_chat_history(&room_id, before, limit)
            .await
            .map_err(|e| Status::internal(format!("Failed to get chat history: {e}")))?;

        // Collect unique user IDs to batch fetch usernames
        let user_ids: Vec<UserId> = messages
            .iter()
            .map(|m| m.user_id.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        // Batch fetch usernames
        let mut username_map = HashMap::new();
        for user_id in user_ids {
            if let Ok(user) = self.user_service.get_user(&user_id).await {
                username_map.insert(user_id.to_string(), user.username);
            }
        }

        // Convert to proto format
        let proto_messages = messages
            .into_iter()
            .map(|m| {
                let username = username_map
                    .get(m.user_id.as_str())
                    .cloned()
                    .unwrap_or_else(|| "Unknown".to_string());

                ChatMessageReceive {
                    id: m.id,
                    room_id: m.room_id.to_string(),
                    user_id: m.user_id.to_string(),
                    username,
                    content: m.content,
                    timestamp: m.created_at.timestamp(),
                    position: None, // History messages don't have danmaku position
                    color: None,    // History messages don't have danmaku color
                }
            })
            .collect();

        Ok(Response::new(GetChatHistoryResponse {
            messages: proto_messages,
        }))
    }

    async fn get_ice_servers(
        &self,
        request: Request<GetIceServersRequest>,
    ) -> Result<Response<GetIceServersResponse>, Status> {
        let _user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;

        // Check if user has access to the room (is a member)
        self.room_service
            .check_membership(&room_id, &_user_id)
            .await
            .map_err(|e| Status::permission_denied(format!("Not a member of the room: {e}")))?;

        // Get WebRTC configuration from settings
        let webrtc_config = &self.config.webrtc;

        let mut servers = Vec::new();

        // Add built-in STUN server if enabled
        if webrtc_config.enable_builtin_stun {
            let stun_url = format!(
                "stun:{}:{}",
                self.config.server.host,
                webrtc_config.builtin_stun_port
            );
            servers.push(IceServer {
                urls: vec![stun_url],
                username: None,
                credential: None,
            });
        }

        // Add external STUN servers
        for url in &webrtc_config.external_stun_servers {
            servers.push(IceServer {
                urls: vec![url.clone()],
                username: None,
                credential: None,
            });
        }

        // Add TURN server based on configured mode
        match webrtc_config.turn_mode {
            synctv_core::config::TurnMode::Builtin => {
                if webrtc_config.enable_builtin_turn {
                    // Use built-in TURN server
                    let turn_url = format!(
                        "turn:{}:{}",
                        self.config.server.host,
                        webrtc_config.builtin_turn_port
                    );

                    // Get static secret for credential generation
                    if let Some(turn_secret) = &webrtc_config.external_turn_static_secret {
                        let turn_config = synctv_core::service::TurnConfig {
                            server_url: turn_url.clone(),
                            static_secret: turn_secret.clone(),
                            credential_ttl: std::time::Duration::from_secs(webrtc_config.turn_credential_ttl),
                            use_tls: false,
                        };
                        let turn_service = synctv_core::service::TurnCredentialService::new(turn_config);

                        // Generate time-limited credentials
                        let credential = turn_service
                            .generate_credential(_user_id.as_str())
                            .map_err(|e| Status::internal(format!("Failed to generate TURN credentials: {e}")))?;

                        servers.push(IceServer {
                            urls: vec![turn_url],
                            username: Some(credential.username),
                            credential: Some(credential.password),
                        });
                    }
                }
            }
            synctv_core::config::TurnMode::External => {
                // Use external TURN server (coturn)
                if let (Some(turn_url), Some(turn_secret)) = (
                    &webrtc_config.external_turn_server_url,
                    &webrtc_config.external_turn_static_secret,
                ) {
                    let turn_config = synctv_core::service::TurnConfig {
                        server_url: turn_url.clone(),
                        static_secret: turn_secret.clone(),
                        credential_ttl: std::time::Duration::from_secs(webrtc_config.turn_credential_ttl),
                        use_tls: false,
                    };
                    let turn_service = synctv_core::service::TurnCredentialService::new(turn_config);

                    // Generate time-limited credentials
                    let credential = turn_service
                        .generate_credential(_user_id.as_str())
                        .map_err(|e| Status::internal(format!("Failed to generate TURN credentials: {e}")))?;

                    // Get all TURN URLs (including TLS variant if enabled)
                    let urls = turn_service.get_urls();

                    servers.push(IceServer {
                        urls,
                        username: Some(credential.username),
                        credential: Some(credential.password),
                    });
                }
            }
            synctv_core::config::TurnMode::Disabled => {
                // TURN disabled - rely on STUN only for NAT traversal
                // This may result in ~85-90% connection success rate instead of ~99%
            }
        }

        Ok(Response::new(GetIceServersResponse { servers }))
    }

    async fn get_network_quality(
        &self,
        request: Request<GetNetworkQualityRequest>,
    ) -> Result<Response<GetNetworkQualityResponse>, Status> {
        let _user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;

        // Check if user has access to the room (is a member)
        self.room_service
            .check_membership(&room_id, &_user_id)
            .await
            .map_err(|e| Status::permission_denied(format!("Not a member of the room: {e}")))?;

        // Return empty stats for now - in production this would be populated
        // from the SFU NetworkQualityMonitor when SFU mode is active
        Ok(Response::new(GetNetworkQualityResponse { peers: vec![] }))
    }
}

/// gRPC message sender for `StreamMessageHandler`
struct GrpcMessageSender {
    sender: tokio::sync::mpsc::UnboundedSender<ServerMessage>,
}

impl GrpcMessageSender {
    const fn new(sender: tokio::sync::mpsc::UnboundedSender<ServerMessage>) -> Self {
        Self { sender }
    }
}

impl MessageSender for GrpcMessageSender {
    fn send(&self, message: ServerMessage) -> Result<(), String> {
        self.sender
            .send(message)
            .map_err(|e| format!("Failed to send message: {e}"))
    }
}

// ==================== MediaService Implementation ====================
#[tonic::async_trait]
impl MediaService for ClientServiceImpl {
    async fn add_media(
        &self,
        request: Request<AddMediaRequest>,
    ) -> Result<Response<AddMediaResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();

        let provider_instance_name = if req.provider.is_empty() {
            String::new()
        } else {
            req.provider.clone()
        };

        let source_config = serde_json::json!({
            "url": req.url.clone()
        });

        let title = if req.title.is_empty() {
            req.url.clone()
        } else {
            req.title.clone()
        };

        let media = self
            .room_service
            .add_media(room_id, user_id, provider_instance_name, source_config, title)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                _ => Status::internal("Failed to add media"),
            })?;

        // Get metadata from PlaybackResult if available (for direct URLs)
        let metadata_bytes = if media.is_direct() {
            media
                .get_playback_result()
                .map(|pb| serde_json::to_vec(&pb.metadata).unwrap_or_default())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let proto_media = Some(Media {
            id: media.id.as_str().to_string(),
            room_id: media.room_id.as_str().to_string(),
            url: String::new(), // Deprecated: Use source_config instead
            provider: media.source_provider.clone(),
            title: media.name.clone(),
            metadata: metadata_bytes,
            position: media.position,
            added_at: media.added_at.timestamp(),
            added_by: media.creator_id.as_str().to_string(),
            provider_instance_name: media.provider_instance_name.unwrap_or_default(),
            source_config: serde_json::to_vec(&media.source_config).unwrap_or_default(),
        });

        Ok(Response::new(AddMediaResponse { media: proto_media }))
    }

    async fn remove_media(
        &self,
        request: Request<RemoveMediaRequest>,
    ) -> Result<Response<RemoveMediaResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();
        let media_id = MediaId::from_string(req.media_id);

        self.room_service
            .remove_media(room_id, user_id, media_id)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                _ => Status::internal("Failed to remove media"),
            })?;

        Ok(Response::new(RemoveMediaResponse { success: true }))
    }

    async fn list_playlist(
        &self,
        request: Request<ListPlaylistRequest>,
    ) -> Result<Response<ListPlaylistResponse>, Status> {
        let room_id = self.get_room_id(&request)?;

        let media_list = self
            .room_service
            .get_playlist(&room_id)
            .await
            .map_err(|_| Status::internal("Failed to get playlist"))?;

        let proto_media: Vec<Media> = media_list
            .into_iter()
            .map(|m| {
                // Get metadata from PlaybackResult if available (for direct URLs)
                let metadata_bytes = if m.is_direct() {
                    m.get_playback_result()
                        .map(|pb| serde_json::to_vec(&pb.metadata).unwrap_or_default())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };

                Media {
                    id: m.id.as_str().to_string(),
                    room_id: m.room_id.as_str().to_string(),
                    url: String::new(), // Deprecated: Use source_config instead
                    provider: m.source_provider.clone(),
                    title: m.name.clone(),
                    metadata: metadata_bytes,
                    position: m.position,
                    added_at: m.added_at.timestamp(),
                    added_by: m.creator_id.as_str().to_string(),
                    provider_instance_name: m.provider_instance_name.unwrap_or_default(),
                    source_config: serde_json::to_vec(&m.source_config).unwrap_or_default(),
                }
            })
            .collect();

        let total = proto_media.len() as i32;

        // Get actual playlist info from service
        let playlist = match self.room_service.playlist_service().get_root_playlist(&room_id).await {
            Ok(pl) => Some(self.playlist_to_proto(&pl, total)),
            Err(_) => Some(Playlist {
                id: String::new(),
                room_id: room_id.as_str().to_string(),
                name: String::new(),
                parent_id: String::new(),
                position: 0,
                is_folder: false,
                is_dynamic: false,
                item_count: total,
                created_at: 0,
                updated_at: 0,
            }),
        };

        Ok(Response::new(ListPlaylistResponse {
            playlist,
            media: proto_media,
            total,
        }))
    }

    async fn list_playlist_items(
        &self,
        request: Request<ListPlaylistItemsRequest>,
    ) -> Result<Response<ListPlaylistItemsResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        let room_id = RoomId::from_string(req.room_id);
        let playlist_id = synctv_core::models::PlaylistId::from_string(req.playlist_id);

        let relative_path = if req.relative_path.is_empty() {
            None
        } else {
            Some(req.relative_path.as_str())
        };

        let page = req.page.max(0) as usize;
        let page_size = req.page_size.clamp(1, 100) as usize;

        let items = self
            .room_service
            .media_service()
            .list_dynamic_playlist_items(room_id, user_id, &playlist_id, relative_path, page, page_size)
            .await
            .map_err(|e| {
                tracing::error!("Failed to list playlist items: {}", e);
                match e {
                    synctv_core::Error::Authorization(msg) | synctv_core::Error::PermissionDenied(msg) => {
                        Status::permission_denied(msg)
                    }
                    synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                    synctv_core::Error::InvalidInput(msg) => Status::invalid_argument(msg),
                    _ => Status::internal("Failed to list playlist items"),
                }
            })?;

        // Convert DirectoryItem to proto DirectoryItem
        let proto_items: Vec<_> = items
            .into_iter()
            .map(|item| {
                use synctv_core::provider::ItemType as CoreItemType;
                let item_type = match item.item_type {
                    CoreItemType::Video => crate::proto::client::ItemType::Video as i32,
                    CoreItemType::Audio => crate::proto::client::ItemType::Audio as i32,
                    CoreItemType::Folder => crate::proto::client::ItemType::Folder as i32,
                    CoreItemType::Live => crate::proto::client::ItemType::Live as i32,
                    CoreItemType::File => crate::proto::client::ItemType::File as i32,
                };

                crate::proto::client::DirectoryItem {
                    name: item.name,
                    item_type,
                    path: item.path,
                    size: item.size.map(|s| s as i64),
                    thumbnail: item.thumbnail,
                    modified_at: item.modified_at,
                }
            })
            .collect();

        let total = proto_items.len() as i32;

        Ok(Response::new(ListPlaylistItemsResponse {
            items: proto_items,
            total,
        }))
    }

    async fn swap_media(
        &self,
        request: Request<SwapMediaRequest>,
    ) -> Result<Response<SwapMediaResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();
        let media_id1 = MediaId::from_string(req.media_id1);
        let media_id2 = MediaId::from_string(req.media_id2);

        self.room_service
            .swap_media(room_id, user_id, media_id1, media_id2)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                _ => Status::internal("Failed to swap media"),
            })?;

        Ok(Response::new(SwapMediaResponse { success: true }))
    }

    async fn play(&self, request: Request<PlayRequest>) -> Result<Response<PlayResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;

        let state = self
            .room_service
            .update_playback(
                room_id,
                user_id,
                synctv_core::models::RoomPlaybackState::play,
                PermissionBits::PLAY_PAUSE,
            )
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                _ => Status::internal("Failed to play"),
            })?;

        let proto_state = Some(PlaybackState {
            room_id: state.room_id.as_str().to_string(),
            playing_media_id: state
                .playing_media_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            position: state.position,
            speed: state.speed,
            is_playing: state.is_playing,
            updated_at: state.updated_at.timestamp(),
            version: state.version,
        });

        Ok(Response::new(PlayResponse {
            playback_state: proto_state,
        }))
    }

    async fn pause(
        &self,
        request: Request<PauseRequest>,
    ) -> Result<Response<PauseResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;

        let state = self
            .room_service
            .update_playback(
                room_id,
                user_id,
                synctv_core::models::RoomPlaybackState::pause,
                PermissionBits::PLAY_PAUSE,
            )
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                _ => Status::internal("Failed to pause"),
            })?;

        let proto_state = Some(PlaybackState {
            room_id: state.room_id.as_str().to_string(),
            playing_media_id: state
                .playing_media_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            position: state.position,
            speed: state.speed,
            is_playing: state.is_playing,
            updated_at: state.updated_at.timestamp(),
            version: state.version,
        });

        Ok(Response::new(PauseResponse {
            playback_state: proto_state,
        }))
    }

    async fn seek(&self, request: Request<SeekRequest>) -> Result<Response<SeekResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();

        let state = self
            .room_service
            .update_playback(
                room_id,
                user_id,
                |state| state.seek(req.position),
                PermissionBits::SEEK,
            )
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                _ => Status::internal("Failed to seek"),
            })?;

        let proto_state = Some(PlaybackState {
            room_id: state.room_id.as_str().to_string(),
            playing_media_id: state
                .playing_media_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            position: state.position,
            speed: state.speed,
            is_playing: state.is_playing,
            updated_at: state.updated_at.timestamp(),
            version: state.version,
        });

        Ok(Response::new(SeekResponse {
            playback_state: proto_state,
        }))
    }

    async fn change_speed(
        &self,
        request: Request<ChangeSpeedRequest>,
    ) -> Result<Response<ChangeSpeedResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();

        let state = self
            .room_service
            .update_playback(
                room_id,
                user_id,
                |state| state.change_speed(req.speed),
                PermissionBits::CHANGE_SPEED,
            )
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                _ => Status::internal("Failed to change speed"),
            })?;

        let proto_state = Some(PlaybackState {
            room_id: state.room_id.as_str().to_string(),
            playing_media_id: state
                .playing_media_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            position: state.position,
            speed: state.speed,
            is_playing: state.is_playing,
            updated_at: state.updated_at.timestamp(),
            version: state.version,
        });

        Ok(Response::new(ChangeSpeedResponse {
            playback_state: proto_state,
        }))
    }

    async fn switch_media(
        &self,
        request: Request<SwitchMediaRequest>,
    ) -> Result<Response<SwitchMediaResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();
        let media_id = MediaId::from_string(req.media_id);

        let state = self
            .room_service
            .update_playback(
                room_id,
                user_id,
                |state| state.switch_media(media_id.clone()),
                PermissionBits::SWITCH_MEDIA,
            )
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                _ => Status::internal("Failed to switch media"),
            })?;

        let proto_state = Some(PlaybackState {
            room_id: state.room_id.as_str().to_string(),
            playing_media_id: state
                .playing_media_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            position: state.position,
            speed: state.speed,
            is_playing: state.is_playing,
            updated_at: state.updated_at.timestamp(),
            version: state.version,
        });

        Ok(Response::new(SwitchMediaResponse {
            playback_state: proto_state,
        }))
    }

    async fn get_playback_state(
        &self,
        request: Request<GetPlaybackStateRequest>,
    ) -> Result<Response<GetPlaybackStateResponse>, Status> {
        let room_id = self.get_room_id(&request)?;

        let state = self
            .room_service
            .get_playback_state(&room_id)
            .await
            .map_err(|_| Status::internal("Failed to get playback state"))?;

        let proto_state = Some(PlaybackState {
            room_id: state.room_id.as_str().to_string(),
            playing_media_id: state
                .playing_media_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            position: state.position,
            speed: state.speed,
            is_playing: state.is_playing,
            updated_at: state.updated_at.timestamp(),
            version: state.version,
        });

        Ok(Response::new(GetPlaybackStateResponse {
            playback_state: proto_state,
        }))
    }

    async fn new_publish_key(
        &self,
        request: Request<NewPublishKeyRequest>,
    ) -> Result<Response<NewPublishKeyResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();

        if req.id.is_empty() {
            return Err(Status::invalid_argument("Media ID is required"));
        }

        let _media_id = MediaId::from_string(req.id.clone());

        let _room = self
            .room_service
            .get_room(&room_id)
            .await
            .map_err(|e| match e {
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                _ => Status::internal("Failed to get room"),
            })?;

        self.room_service
            .check_permission(&room_id, &user_id, PermissionBits::ADD_MEDIA)
            .await
            .map_err(|e| {
                Status::permission_denied(format!(
                    "User does not have permission to publish streams in this room: {e}"
                ))
            })?;

        let expiration_duration = chrono::Duration::hours(24);
        let now = chrono::Utc::now();
        let expires_at = now + expiration_duration;

        #[derive(serde::Serialize, serde::Deserialize)]
        struct PublishKeyClaims {
            room_id: String,
            media_id: String,
            user_id: String,
            iat: i64,
            exp: i64,
        }

        let _claims = PublishKeyClaims {
            room_id: room_id.as_str().to_string(),
            media_id: req.id.clone(),
            user_id: user_id.as_str().to_string(),
            iat: now.timestamp(),
            exp: expires_at.timestamp(),
        };

        let publish_key = format!(
            "{}:{}:{}:{}",
            room_id.as_str(),
            req.id,
            user_id.as_str(),
            expires_at.timestamp()
        );

        let rtmp_url = "rtmp://localhost:1935/live".to_string();
        let stream_key = format!("{}/{}", room_id.as_str(), req.id);

        tracing::info!(
            room_id = %room_id.as_str(),
            media_id = %req.id,
            user_id = %user_id.as_str(),
            expires_at = %expires_at,
            "Generated publish key for live streaming"
        );

        Ok(Response::new(NewPublishKeyResponse {
            publish_key,
            rtmp_url,
            stream_key,
            expires_at: expires_at.timestamp(),
        }))
    }

    // Playlist Management
    async fn create_playlist(
        &self,
        request: Request<CreatePlaylistRequest>,
    ) -> Result<Response<CreatePlaylistResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();

        let parent_id = if req.parent_id.is_empty() {
            None
        } else {
            Some(synctv_core::models::PlaylistId::from_string(req.parent_id))
        };

        let service_req = synctv_core::service::playlist::CreatePlaylistRequest {
            room_id: room_id.clone(),
            name: req.name,
            parent_id,
            position: None,
            source_provider: None,
            source_config: None,
            provider_instance_name: None,
        };

        let playlist = self
            .room_service
            .playlist_service()
            .create_playlist(room_id, user_id, service_req)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) | synctv_core::Error::PermissionDenied(msg) => {
                    Status::permission_denied(msg)
                }
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                synctv_core::Error::InvalidInput(msg) => Status::invalid_argument(msg),
                _ => Status::internal("Failed to create playlist"),
            })?;

        let item_count = self
            .room_service
            .media_service()
            .count_playlist_media(&playlist.id)
            .await
            .unwrap_or(0) as i32;

        Ok(Response::new(CreatePlaylistResponse {
            playlist: Some(self.playlist_to_proto(&playlist, item_count)),
        }))
    }

    async fn set_playlist(
        &self,
        request: Request<SetPlaylistRequest>,
    ) -> Result<Response<SetPlaylistResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();

        let playlist_id = synctv_core::models::PlaylistId::from_string(req.playlist_id);

        let name = if req.name.is_empty() { None } else { Some(req.name) };
        let position = if req.position == 0 { None } else { Some(req.position) };

        let service_req = synctv_core::service::playlist::SetPlaylistRequest {
            playlist_id,
            name,
            position,
        };

        let playlist = self
            .room_service
            .playlist_service()
            .set_playlist(room_id, user_id, service_req)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) | synctv_core::Error::PermissionDenied(msg) => {
                    Status::permission_denied(msg)
                }
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                synctv_core::Error::InvalidInput(msg) => Status::invalid_argument(msg),
                _ => Status::internal("Failed to update playlist"),
            })?;

        let item_count = self
            .room_service
            .media_service()
            .count_playlist_media(&playlist.id)
            .await
            .unwrap_or(0) as i32;

        Ok(Response::new(SetPlaylistResponse {
            playlist: Some(self.playlist_to_proto(&playlist, item_count)),
        }))
    }

    async fn delete_playlist(
        &self,
        request: Request<DeletePlaylistRequest>,
    ) -> Result<Response<DeletePlaylistResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();

        let playlist_id = synctv_core::models::PlaylistId::from_string(req.playlist_id);

        self.room_service
            .playlist_service()
            .delete_playlist(room_id, user_id, playlist_id)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) | synctv_core::Error::PermissionDenied(msg) => {
                    Status::permission_denied(msg)
                }
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                synctv_core::Error::InvalidInput(msg) => Status::invalid_argument(msg),
                _ => Status::internal("Failed to delete playlist"),
            })?;

        Ok(Response::new(DeletePlaylistResponse { success: true }))
    }

    async fn list_playlists(
        &self,
        request: Request<ListPlaylistsRequest>,
    ) -> Result<Response<ListPlaylistsResponse>, Status> {
        let _user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();

        let playlists = if req.parent_id.is_empty() {
            // Get all playlists in room
            self.room_service
                .playlist_service()
                .get_room_playlists(&room_id)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to list playlists: {}", e);
                    Status::internal("Failed to list playlists")
                })?
        } else {
            // Get children of specific playlist
            let parent_id = synctv_core::models::PlaylistId::from_string(req.parent_id);
            self.room_service
                .playlist_service()
                .get_children(&parent_id)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to list playlists: {}", e);
                    Status::internal("Failed to list playlists")
                })?
        };

        let mut proto_playlists = Vec::with_capacity(playlists.len());
        for pl in &playlists {
            let item_count = self
                .room_service
                .media_service()
                .count_playlist_media(&pl.id)
                .await
                .unwrap_or(0) as i32;
            proto_playlists.push(self.playlist_to_proto(pl, item_count));
        }

        Ok(Response::new(ListPlaylistsResponse {
            playlists: proto_playlists,
        }))
    }

    async fn set_playing(
        &self,
        request: Request<SetPlayingRequest>,
    ) -> Result<Response<SetPlayingResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let room_id = self.get_room_id(&request)?;
        let req = request.into_inner();

        let playlist_id = synctv_core::models::PlaylistId::from_string(req.playlist_id);

        // Verify the playlist exists and belongs to this room
        let playlist = self
            .room_service
            .playlist_service()
            .get_playlist(&playlist_id)
            .await
            .map_err(|_| Status::internal("Failed to get playlist"))?
            .ok_or_else(|| Status::not_found("Playlist not found"))?;

        if playlist.room_id != room_id {
            return Err(Status::permission_denied("Playlist does not belong to this room"));
        }

        // If a specific media_id is provided, switch to it
        let playing_media = if !req.media_id.is_empty() {
            let media_id = MediaId::from_string(req.media_id);
            self.room_service
                .set_playing_media(room_id, user_id, media_id.clone())
                .await
                .map_err(|e| match e {
                    synctv_core::Error::Authorization(msg) | synctv_core::Error::PermissionDenied(msg) => {
                        Status::permission_denied(msg)
                    }
                    synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                    _ => Status::internal("Failed to set playing media"),
                })?;

            // Get the media details
            let media = self
                .room_service
                .media_service()
                .get_media(&media_id)
                .await
                .map_err(|_| Status::internal("Failed to get media"))?;

            media.map(|m| Media {
                id: m.id.as_str().to_string(),
                room_id: m.room_id.as_str().to_string(),
                url: String::new(),
                provider: m.source_provider.clone(),
                title: m.name.clone(),
                metadata: Vec::new(),
                position: m.position,
                added_at: m.added_at.timestamp(),
                added_by: m.creator_id.as_str().to_string(),
                provider_instance_name: m.provider_instance_name.unwrap_or_default(),
                source_config: serde_json::to_vec(&m.source_config).unwrap_or_default(),
            })
        } else {
            // No specific media, just get the first media in playlist
            let (media_list, _total) = self
                .room_service
                .media_service()
                .get_playlist_media_paginated(&playlist_id, 0, 1)
                .await
                .map_err(|_| Status::internal("Failed to get playlist media"))?;

            if let Some(m) = media_list.first() {
                let media_id = m.id.clone();
                let _state = self
                    .room_service
                    .set_playing_media(room_id, user_id, media_id)
                    .await
                    .map_err(|e| match e {
                        synctv_core::Error::Authorization(msg) | synctv_core::Error::PermissionDenied(msg) => {
                            Status::permission_denied(msg)
                        }
                        synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                        _ => Status::internal("Failed to set playing media"),
                    })?;

                Some(Media {
                    id: m.id.as_str().to_string(),
                    room_id: m.room_id.as_str().to_string(),
                    url: String::new(),
                    provider: m.source_provider.clone(),
                    title: m.name.clone(),
                    metadata: Vec::new(),
                    position: m.position,
                    added_at: m.added_at.timestamp(),
                    added_by: m.creator_id.as_str().to_string(),
                    provider_instance_name: m.provider_instance_name.clone().unwrap_or_default(),
                    source_config: serde_json::to_vec(&m.source_config).unwrap_or_default(),
                })
            } else {
                None
            }
        };

        let item_count = self
            .room_service
            .media_service()
            .count_playlist_media(&playlist.id)
            .await
            .unwrap_or(0) as i32;

        Ok(Response::new(SetPlayingResponse {
            playlist: Some(self.playlist_to_proto(&playlist, item_count)),
            playing_media,
        }))
    }

    async fn get_movie_info(
        &self,
        request: Request<GetMovieInfoRequest>,
    ) -> Result<Response<GetMovieInfoResponse>, Status> {
        let _user_id = self.get_user_id(&request)?;
        let _room_id = self.get_room_id(&request)?;
        let _req = request.into_inner();

        // TODO: Implement full GetMovieInfo logic
        // 1. Get media from playlist by media_id
        // 2. Determine provider and call generate_playback()
        // 3. Check movie_proxy setting
        // 4. Build MovieInfo with proxy or direct URLs
        Err(Status::unimplemented("GetMovieInfo not yet implemented"))
    }
}

// ==================== PublicService Implementation ====================
#[tonic::async_trait]
impl PublicService for ClientServiceImpl {
    async fn check_room(
        &self,
        request: Request<CheckRoomRequest>,
    ) -> Result<Response<CheckRoomResponse>, Status> {
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        match self.room_service.get_room(&room_id).await {
            Ok(room) => {
                let settings = self.room_service
                    .get_room_settings(&room_id)
                    .await
                    .unwrap_or_default();
                let requires_password = settings.require_password.0;

                Ok(Response::new(CheckRoomResponse {
                    exists: true,
                    requires_password,
                    name: room.name,
                }))
            }
            Err(_) => Ok(Response::new(CheckRoomResponse {
                exists: false,
                requires_password: false,
                name: String::new(),
            })),
        }
    }

    async fn list_rooms(
        &self,
        request: Request<ListRoomsRequest>,
    ) -> Result<Response<ListRoomsResponse>, Status> {
        let req = request.into_inner();
        let page = if req.page == 0 { 1 } else { req.page };
        let page_size = if req.page_size == 0 || req.page_size > 100 {
            20
        } else {
            req.page_size
        };

        let query = RoomListQuery {
            page,
            page_size,
            search: None,
            status: Some(RoomStatus::Active),
        };

        let (rooms, total) = self
            .room_service
            .list_rooms(&query)
            .await
            .map_err(|e| Status::internal(format!("Failed to list rooms: {e}")))?;

        let mut proto_rooms = Vec::new();
        for room in rooms {
            let member_count = self
                .room_service
                .get_member_count(&room.id)
                .await
                .unwrap_or(0);

            // Load room settings
            let settings = self.room_service
                .get_room_settings(&room.id)
                .await
                .unwrap_or_default();

            proto_rooms.push(Room {
                id: room.id.to_string(),
                name: room.name,
                created_by: room.created_by.to_string(),
                status: match room.status {
                    RoomStatus::Pending => "pending".to_string(),

                    RoomStatus::Active => "active".to_string(),
                    RoomStatus::Banned => "banned".to_string(),
                },
                settings: serde_json::to_vec(&settings).unwrap_or_default(),
                created_at: room.created_at.timestamp(),
                member_count,
            });
        }

        Ok(Response::new(ListRoomsResponse {
            rooms: proto_rooms,
            total: total as i32,
        }))
    }

    async fn get_hot_rooms(
        &self,
        request: Request<GetHotRoomsRequest>,
    ) -> Result<Response<GetHotRoomsResponse>, Status> {
        let req = request.into_inner();
        let limit = if req.limit == 0 || req.limit > 50 {
            10
        } else {
            i64::from(req.limit)
        };

        let query = RoomListQuery {
            page: 1,
            page_size: 100,
            search: None,
            status: Some(RoomStatus::Active),
        };

        let (rooms, _total) = self
            .room_service
            .list_rooms(&query)
            .await
            .map_err(|e| Status::internal(format!("Failed to list rooms: {e}")))?;

        let mut room_stats: Vec<(synctv_core::models::Room, i32, i32)> = Vec::new();
        for room in rooms {
            let online_count = self.connection_manager.room_connection_count(&room.id);
            let member_count = self
                .room_service
                .get_member_count(&room.id)
                .await
                .unwrap_or(0);

            room_stats.push((room, online_count as i32, member_count));
        }

        room_stats.sort_by(|a, b| b.1.cmp(&a.1));

        let mut hot_rooms: Vec<RoomWithStats> = Vec::new();
        for (room, online_count, member_count) in room_stats.into_iter().take(limit as usize) {
            // Load room settings
            let settings = self.room_service
                .get_room_settings(&room.id)
                .await
                .unwrap_or_default();

            let room_proto = Room {
                id: room.id.to_string(),
                name: room.name,
                created_by: room.created_by.to_string(),
                status: match room.status {
                    RoomStatus::Pending => "pending".to_string(),

                    RoomStatus::Active => "active".to_string(),
                    RoomStatus::Banned => "banned".to_string(),
                },
                settings: serde_json::to_vec(&settings).unwrap_or_default(),
                created_at: room.created_at.timestamp(),
                member_count,
            };

            hot_rooms.push(RoomWithStats {
                room: Some(room_proto),
                online_count,
                total_members: member_count,
            });
        }

        Ok(Response::new(GetHotRoomsResponse { rooms: hot_rooms }))
    }

    async fn get_public_settings(
        &self,
        _request: Request<GetPublicSettingsRequest>,
    ) -> Result<Response<GetPublicSettingsResponse>, Status> {
        let reg = self.settings_registry.as_ref()
            .ok_or_else(|| Status::unimplemented("Settings registry not configured"))?;

        let s = reg.to_public_settings();
        Ok(Response::new(GetPublicSettingsResponse {
            signup_enabled: s.signup_enabled,
            allow_room_creation: s.allow_room_creation,
            max_rooms_per_user: s.max_rooms_per_user,
            max_members_per_room: s.max_members_per_room,
            disable_create_room: s.disable_create_room,
            create_room_need_review: s.create_room_need_review,
            room_ttl: s.room_ttl,
            room_must_need_pwd: s.room_must_need_pwd,
            signup_need_review: s.signup_need_review,
            enable_password_signup: s.enable_password_signup,
            enable_guest: s.enable_guest,
            movie_proxy: s.movie_proxy,
            live_proxy: s.live_proxy,
            rtmp_player: s.rtmp_player,
            ts_disguised_as_png: s.ts_disguised_as_png,
            custom_publish_host: s.custom_publish_host,
            email_whitelist_enabled: s.email_whitelist_enabled,
            p2p_zone: s.p2p_zone,
        }))
    }
}

// ==================== EmailService Implementation ====================
#[tonic::async_trait]
impl EmailService for ClientServiceImpl {
    async fn send_verification_email(
        &self,
        request: Request<SendVerificationEmailRequest>,
    ) -> Result<Response<SendVerificationEmailResponse>, Status> {
        let email_service = self.email_service.as_ref()
            .ok_or_else(|| Status::unimplemented("Email service not configured"))?;
        let email_token_service = self.email_token_service.as_ref()
            .ok_or_else(|| Status::unimplemented("Email token service not configured"))?;

        let req = request.into_inner();

        // Check if user exists with this email
        let user = self
            .user_service
            .get_by_email(&req.email)
            .await
            .map_err(|e| Status::internal(format!("Database error: {e}")))?;

        let user = match user {
            Some(u) => u,
            None => {
                return Ok(Response::new(SendVerificationEmailResponse {
                    message: "If an account exists with this email, a verification code will be sent.".to_string(),
                }));
            }
        };

        // Generate and send verification email
        let _token = email_service
            .send_verification_email(&req.email, email_token_service, &user.id)
            .await
            .map_err(|e| Status::internal(format!("Failed to send email: {e}")))?;

        tracing::info!("Sent verification email to {}", req.email);

        Ok(Response::new(SendVerificationEmailResponse {
            message: "Verification code sent to your email".to_string(),
        }))
    }

    async fn confirm_email(
        &self,
        request: Request<ConfirmEmailRequest>,
    ) -> Result<Response<ConfirmEmailResponse>, Status> {
        let email_token_service = self.email_token_service.as_ref()
            .ok_or_else(|| Status::unimplemented("Email token service not configured"))?;

        let req = request.into_inner();

        // Check if user exists
        let user = self
            .user_service
            .get_by_email(&req.email)
            .await
            .map_err(|e| Status::internal(format!("Database error: {e}")))?
            .ok_or_else(|| Status::not_found("User not found"))?;

        // Validate token
        let validated_user_id = email_token_service
            .validate_token(&req.token, synctv_core::service::EmailTokenType::EmailVerification)
            .await
            .map_err(|e| Status::invalid_argument(format!("Invalid token: {e}")))?;

        // Verify token matches user
        if validated_user_id != user.id {
            return Err(Status::invalid_argument("Token does not match email"));
        }

        // Mark email as verified
        self.user_service
            .set_email_verified(&user.id, true)
            .await
            .map_err(|e| Status::internal(format!("Failed to update email verification: {e}")))?;

        tracing::info!("Email verified for user {}", user.id.as_str());

        Ok(Response::new(ConfirmEmailResponse {
            message: "Email verified successfully".to_string(),
            user_id: user.id.to_string(),
        }))
    }

    async fn request_password_reset(
        &self,
        request: Request<RequestPasswordResetRequest>,
    ) -> Result<Response<RequestPasswordResetResponse>, Status> {
        let email_service = self.email_service.as_ref()
            .ok_or_else(|| Status::unimplemented("Email service not configured"))?;
        let email_token_service = self.email_token_service.as_ref()
            .ok_or_else(|| Status::unimplemented("Email token service not configured"))?;

        let req = request.into_inner();

        // Check if user exists (don't reveal if not found for security)
        let user = self
            .user_service
            .get_by_email(&req.email)
            .await
            .map_err(|e| Status::internal(format!("Database error: {e}")))?;

        let Some(user) = user else {
            // Don't reveal whether email exists
            return Ok(Response::new(RequestPasswordResetResponse {
                message: "If an account exists with this email, a password reset code will be sent.".to_string(),
            }));
        };

        // Generate and send reset email
        let _token = email_service
            .send_password_reset_email(&req.email, email_token_service, &user.id)
            .await
            .map_err(|e| Status::internal(format!("Failed to send email: {e}")))?;

        tracing::info!("Password reset requested for user {}", user.id.as_str());

        Ok(Response::new(RequestPasswordResetResponse {
            message: "Password reset code sent to your email".to_string(),
        }))
    }

    async fn confirm_password_reset(
        &self,
        request: Request<ConfirmPasswordResetRequest>,
    ) -> Result<Response<ConfirmPasswordResetResponse>, Status> {
        let email_token_service = self.email_token_service.as_ref()
            .ok_or_else(|| Status::unimplemented("Email token service not configured"))?;

        let req = request.into_inner();

        // Check if user exists
        let user = self
            .user_service
            .get_by_email(&req.email)
            .await
            .map_err(|e| Status::internal(format!("Database error: {e}")))?
            .ok_or_else(|| Status::not_found("User not found"))?;

        // Validate token
        let validated_user_id = email_token_service
            .validate_token(&req.token, synctv_core::service::EmailTokenType::PasswordReset)
            .await
            .map_err(|e| Status::invalid_argument(format!("Invalid token: {e}")))?;

        // Verify token matches user
        if validated_user_id != user.id {
            return Err(Status::invalid_argument("Token does not match email"));
        }

        // Validate new password
        if req.new_password.len() < 8 {
            return Err(Status::invalid_argument("Password must be at least 8 characters"));
        }

        // Update password
        self.user_service
            .set_password(&user.id, &req.new_password)
            .await
            .map_err(|e| Status::internal(format!("Failed to update password: {e}")))?;

        tracing::info!("Password reset completed for user {}", user.id.as_str());

        Ok(Response::new(ConfirmPasswordResetResponse {
            message: "Password reset successfully".to_string(),
            user_id: user.id.to_string(),
        }))
    }
}
