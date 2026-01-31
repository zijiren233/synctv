use std::sync::Arc;
use std::collections::HashMap;
use tonic::{Request, Response, Status};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt;

use synctv_core::service::{UserService, RoomService, RateLimiter, RateLimitConfig, ContentFilter};
use synctv_core::models::{RoomId, UserId, MediaId, ProviderType, PermissionBits, RoomListQuery, RoomStatus, RoomSettings};
use synctv_cluster::sync::{RoomMessageHub, ClusterEvent, PublishRequest, ConnectionManager};

use super::proto::client::{
    client_service_server::ClientService,
    *,
};

/// ClientService implementation
#[derive(Clone)]
pub struct ClientServiceImpl {
    user_service: Arc<UserService>,
    room_service: Arc<RoomService>,
    message_hub: Arc<RoomMessageHub>,
    redis_publish_tx: Option<tokio::sync::mpsc::UnboundedSender<PublishRequest>>,
    rate_limiter: Arc<RateLimiter>,
    rate_limit_config: Arc<RateLimitConfig>,
    content_filter: Arc<ContentFilter>,
    connection_manager: Arc<ConnectionManager>,
}

impl ClientServiceImpl {
    pub fn new(
        user_service: UserService,
        room_service: RoomService,
        message_hub: RoomMessageHub,
        redis_publish_tx: Option<tokio::sync::mpsc::UnboundedSender<PublishRequest>>,
        rate_limiter: RateLimiter,
        rate_limit_config: RateLimitConfig,
        content_filter: ContentFilter,
        connection_manager: ConnectionManager,
    ) -> Self {
        Self {
            user_service: Arc::new(user_service),
            room_service: Arc::new(room_service),
            message_hub: Arc::new(message_hub),
            redis_publish_tx,
            rate_limiter: Arc::new(rate_limiter),
            rate_limit_config: Arc::new(rate_limit_config),
            content_filter: Arc::new(content_filter),
            connection_manager: Arc::new(connection_manager),
        }
    }

    /// Extract user_id from request extensions (injected by auth interceptor)
    fn get_user_id(&self, request: &Request<impl std::fmt::Debug>) -> Result<UserId, Status> {
        let auth_context = request
            .extensions()
            .get::<super::interceptors::AuthContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;

        Ok(UserId::from_string(auth_context.user_id.clone()))
    }

    /// Extract auth context from request extensions
    fn get_auth_context(&self, request: &Request<impl std::fmt::Debug>) -> Result<super::interceptors::AuthContext, Status> {
        request
            .extensions()
            .get::<super::interceptors::AuthContext>()
            .cloned()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))
    }

    /// Handle incoming client message from bidirectional stream
    async fn handle_client_message(
        msg: ClientMessage,
        message_hub: &RoomMessageHub,
        room_service: &RoomService,
        user_id: &UserId,
        username: &str,
        current_room: &Arc<parking_lot::Mutex<Option<RoomId>>>,
        connection_id: &str,
        outgoing_tx: &tokio::sync::mpsc::UnboundedSender<ServerMessage>,
        redis_publish_tx: &Option<tokio::sync::mpsc::UnboundedSender<PublishRequest>>,
        rate_limiter: &RateLimiter,
        rate_limit_config: &RateLimitConfig,
        content_filter: &ContentFilter,
        connection_manager: &ConnectionManager,
    ) -> Result<(), Status> {
        use chrono::Utc;

        // Record message activity
        connection_manager.record_message(connection_id);

        match msg.message {
            Some(client_message::Message::Chat(chat)) => {
                let room_id = RoomId::from_string(chat.room_id);

                // Check rate limit
                let rate_limit_key = format!("user:{}:chat", user_id.as_str());
                rate_limiter
                    .check_rate_limit(
                        &rate_limit_key,
                        rate_limit_config.chat_per_second,
                        rate_limit_config.window_seconds,
                    )
                    .await
                    .map_err(|e| Status::resource_exhausted(e.to_string()))?;

                // Filter and sanitize content
                let sanitized_content = content_filter
                    .filter_chat(&chat.content)
                    .map_err(|e| Status::invalid_argument(e.to_string()))?;

                // Check if user is in the room
                room_service
                    .check_permission(&room_id, user_id, PermissionBits::SEND_CHAT)
                    .await
                    .map_err(|e| Status::permission_denied(e.to_string()))?;

                // Subscribe to room if not already subscribed
                {
                    let mut current = current_room.lock();
                    if current.is_none() {
                        // Register with connection manager
                        if let Err(e) = connection_manager.join_room(connection_id, room_id.clone()) {
                            return Err(Status::resource_exhausted(format!("Cannot join room: {}", e)));
                        }

                        let mut rx = message_hub.subscribe(
                            room_id.clone(),
                            user_id.clone(),
                            connection_id.to_string(),
                        );

                        // Forward messages from hub to client
                        let outgoing_tx_clone = outgoing_tx.clone();
                        tokio::spawn(async move {
                            while let Some(event) = rx.recv().await {
                                if let Some(server_msg) = Self::convert_event_to_server_message(event) {
                                    let _ = outgoing_tx_clone.send(server_msg);
                                }
                            }
                        });

                        *current = Some(room_id.clone());
                    }
                }

                // Create and broadcast chat event with sanitized content
                let event = ClusterEvent::ChatMessage {
                    room_id: room_id.clone(),
                    user_id: user_id.clone(),
                    username: username.to_string(),
                    message: sanitized_content,
                    timestamp: Utc::now(),
                };

                // Broadcast to local subscribers
                message_hub.broadcast(&room_id, event.clone());

                // Publish to Redis for multi-replica sync
                if let Some(tx) = redis_publish_tx {
                    let _ = tx.send(PublishRequest {
                        room_id: room_id.clone(),
                        event,
                    });
                }

                // TODO: Persist to database
            }

            Some(client_message::Message::Danmaku(danmaku)) => {
                let room_id = RoomId::from_string(danmaku.room_id);

                // Check rate limit
                let rate_limit_key = format!("user:{}:danmaku", user_id.as_str());
                rate_limiter
                    .check_rate_limit(
                        &rate_limit_key,
                        rate_limit_config.danmaku_per_second,
                        rate_limit_config.window_seconds,
                    )
                    .await
                    .map_err(|e| Status::resource_exhausted(e.to_string()))?;

                // Filter and sanitize danmaku content
                let sanitized_content = content_filter
                    .filter_danmaku(&danmaku.content)
                    .map_err(|e| Status::invalid_argument(e.to_string()))?;

                // Check permission
                room_service
                    .check_permission(&room_id, user_id, PermissionBits::SEND_DANMAKU)
                    .await
                    .map_err(|e| Status::permission_denied(e.to_string()))?;

                // Subscribe to room if needed
                {
                    let mut current = current_room.lock();
                    if current.is_none() {
                        // Register with connection manager
                        if let Err(e) = connection_manager.join_room(connection_id, room_id.clone()) {
                            return Err(Status::resource_exhausted(format!("Cannot join room: {}", e)));
                        }

                        let mut rx = message_hub.subscribe(
                            room_id.clone(),
                            user_id.clone(),
                            connection_id.to_string(),
                        );

                        let outgoing_tx_clone = outgoing_tx.clone();
                        tokio::spawn(async move {
                            while let Some(event) = rx.recv().await {
                                if let Some(server_msg) = Self::convert_event_to_server_message(event) {
                                    let _ = outgoing_tx_clone.send(server_msg);
                                }
                            }
                        });

                        *current = Some(room_id.clone());
                    }
                }

                // Create and broadcast danmaku event with sanitized content
                let event = ClusterEvent::Danmaku {
                    room_id: room_id.clone(),
                    user_id: user_id.clone(),
                    username: username.to_string(),
                    message: sanitized_content,
                    position: 0.0, // TODO: Use video position
                    timestamp: Utc::now(),
                };

                // Broadcast to local subscribers
                message_hub.broadcast(&room_id, event.clone());

                // Publish to Redis for multi-replica sync
                if let Some(tx) = redis_publish_tx {
                    let _ = tx.send(PublishRequest {
                        room_id: room_id.clone(),
                        event,
                    });
                }

                // TODO: Store in memory with TTL
            }

            Some(client_message::Message::Heartbeat(heartbeat)) => {
                // Send heartbeat acknowledgement
                let ack = ServerMessage {
                    message: Some(server_message::Message::HeartbeatAck(HeartbeatAck {
                        timestamp: heartbeat.timestamp,
                    })),
                };
                outgoing_tx.send(ack).map_err(|_| Status::internal("Failed to send heartbeat ack"))?;
            }

            None => {
                return Err(Status::invalid_argument("Empty message"));
            }
        }

        Ok(())
    }

    /// Convert ClusterEvent to ServerMessage
    fn convert_event_to_server_message(event: ClusterEvent) -> Option<ServerMessage> {
        match event {
            ClusterEvent::ChatMessage {
                room_id,
                user_id,
                username,
                message,
                timestamp,
            } => Some(ServerMessage {
                message: Some(server_message::Message::Chat(ChatMessageReceive {
                    id: nanoid::nanoid!(12),
                    room_id: room_id.as_str().to_string(),
                    user_id: user_id.as_str().to_string(),
                    username,
                    content: message,
                    timestamp: timestamp.timestamp(),
                })),
            }),

            ClusterEvent::Danmaku {
                room_id,
                user_id,
                message,
                timestamp,
                ..
            } => Some(ServerMessage {
                message: Some(server_message::Message::Danmaku(DanmakuMessageReceive {
                    room_id: room_id.as_str().to_string(),
                    user_id: user_id.as_str().to_string(),
                    content: message,
                    color: "#FFFFFF".to_string(), // Default white
                    position: 2, // Scroll
                    timestamp: timestamp.timestamp(),
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
                        permissions: permissions.0,
                        joined_at: chrono::Utc::now().timestamp(),
                        is_online: true,
                    }),
                })),
            }),

            ClusterEvent::UserLeft {
                room_id,
                user_id,
                ..
            } => Some(ServerMessage {
                message: Some(server_message::Message::UserLeft(UserLeftRoom {
                    room_id: room_id.as_str().to_string(),
                    user_id: user_id.as_str().to_string(),
                })),
            }),

            ClusterEvent::PlaybackStateChanged {
                room_id,
                state,
                ..
            } => Some(ServerMessage {
                message: Some(server_message::Message::PlaybackState(PlaybackStateChanged {
                    room_id: room_id.as_str().to_string(),
                    state: Some(PlaybackState {
                        room_id: room_id.as_str().to_string(),
                        current_media_id: state.current_media_id.map(|id| id.as_str().to_string()).unwrap_or_default(),
                        position: state.position,
                        speed: state.speed,
                        is_playing: state.is_playing,
                        updated_at: state.updated_at.timestamp(),
                        version: state.version,
                    }),
                })),
            }),

            ClusterEvent::RoomSettingsChanged { room_id, .. } => Some(ServerMessage {
                message: Some(server_message::Message::RoomSettings(RoomSettingsChanged {
                    room_id: room_id.as_str().to_string(),
                    settings: vec![], // TODO: Include settings
                })),
            }),

            _ => None,
        }
    }
}

#[tonic::async_trait]
impl ClientService for ClientServiceImpl {
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
            .register(req.username, req.email, req.password)
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
        let proto_user = Some(User {
            id: user.id.as_str().to_string(),
            username: user.username,
            email: user.email,
            permissions: user.permissions.0,
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
        let proto_user = Some(User {
            id: user.id.as_str().to_string(),
            username: user.username,
            email: user.email,
            permissions: user.permissions.0,
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

    async fn get_current_user(
        &self,
        request: Request<GetCurrentUserRequest>,
    ) -> Result<Response<GetCurrentUserResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;

        // Get user from service
        let user = self.user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get user: {}", e)))?;

        Ok(Response::new(GetCurrentUserResponse {
            user: Some(User {
                id: user.id.to_string(),
                username: user.username,
                email: user.email,
                permissions: user.permissions.0,
                created_at: user.created_at.timestamp(),
            }),
        }))
    }

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
        let settings = if !req.settings.is_empty() {
            Some(serde_json::from_slice(&req.settings)
                .map_err(|e| Status::invalid_argument(format!("Invalid settings: {}", e)))?)
        } else {
            None
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

        // Convert to proto Room
        let proto_room = Some(Room {
            id: room.id.as_str().to_string(),
            name: room.name,
            created_by: room.created_by.as_str().to_string(),
            status: room.status.as_str().to_string(),
            settings: serde_json::to_vec(&room.settings).unwrap_or_default(),
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

        // Convert to proto
        let proto_room = Some(Room {
            id: room.id.as_str().to_string(),
            name: room.name,
            created_by: room.created_by.as_str().to_string(),
            status: room.status.as_str().to_string(),
            settings: serde_json::to_vec(&room.settings).unwrap_or_default(),
            created_at: room.created_at.timestamp(),
            member_count: 0, // TODO: Get actual count
        });

        let proto_playback = Some(PlaybackState {
            room_id: playback_state.room_id.as_str().to_string(),
            current_media_id: playback_state.current_media_id.as_ref().map(|id| id.as_str().to_string()).unwrap_or_default(),
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

        // Convert to proto
        let proto_room = Some(Room {
            id: room.id.as_str().to_string(),
            name: room.name,
            created_by: room.created_by.as_str().to_string(),
            status: room.status.as_str().to_string(),
            settings: serde_json::to_vec(&room.settings).unwrap_or_default(),
            created_at: room.created_at.timestamp(),
            member_count: members.len() as i32,
        });

        let proto_playback = Some(PlaybackState {
            room_id: playback_state.room_id.as_str().to_string(),
            current_media_id: playback_state.current_media_id.as_ref().map(|id| id.as_str().to_string()).unwrap_or_default(),
            position: playback_state.position,
            speed: playback_state.speed,
            is_playing: playback_state.is_playing,
            updated_at: playback_state.updated_at.timestamp(),
            version: playback_state.version,
        });

        let proto_members: Vec<RoomMember> = members
            .into_iter()
            .map(|m| RoomMember {
                room_id: m.room_id.as_str().to_string(),
                user_id: m.user_id.as_str().to_string(),
                username: m.username,
                permissions: m.permissions.0,
                joined_at: m.joined_at.timestamp(),
                is_online: m.is_online,
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

    async fn list_rooms(
        &self,
        request: Request<ListRoomsRequest>,
    ) -> Result<Response<ListRoomsResponse>, Status> {
        let req = request.into_inner();

        // Build query
        let query = RoomListQuery {
            page: if req.page == 0 { 1 } else { req.page },
            page_size: if req.page_size == 0 { 20 } else { req.page_size },
            status: None,
            search: None,
        };

        // Get rooms
        let (rooms, total) = self.room_service
            .list_rooms(&query)
            .await
            .map_err(|e| Status::internal(format!("Failed to list rooms: {}", e)))?;

        // Convert to response
        let room_list = rooms.into_iter().map(|room| {
            Room {
                id: room.id.to_string(),
                name: room.name,
                created_by: room.created_by.to_string(),
                status: match room.status {
                    RoomStatus::Active => "active".to_string(),
                    RoomStatus::Closed => "closed".to_string(),
                },
                settings: serde_json::to_vec(&room.settings).unwrap_or_default(),
                created_at: room.created_at.timestamp(),
                member_count: 0, // TODO: Get actual count in query
            }
        }).collect();

        Ok(Response::new(ListRoomsResponse {
            rooms: room_list,
            total: total as i32,
        }))
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

    async fn update_room_settings(
        &self,
        request: Request<UpdateRoomSettingsRequest>,
    ) -> Result<Response<UpdateRoomSettingsResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Parse settings from JSON bytes
        let settings = if !req.settings.is_empty() {
            serde_json::from_slice(&req.settings)
                .map_err(|e| Status::invalid_argument(format!("Invalid settings: {}", e)))?
        } else {
            RoomSettings::default()
        };

        // Update settings
        let updated_room = self.room_service
            .update_settings(room_id.clone(), user_id, settings)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                _ => Status::internal("Failed to update room settings"),
            })?;

        Ok(Response::new(UpdateRoomSettingsResponse {
            room: Some(Room {
                id: updated_room.id.to_string(),
                name: updated_room.name,
                created_by: updated_room.created_by.to_string(),
                status: match updated_room.status {
                    RoomStatus::Active => "active".to_string(),
                    RoomStatus::Closed => "closed".to_string(),
                },
                settings: serde_json::to_vec(&updated_room.settings).unwrap_or_default(),
                created_at: updated_room.created_at.timestamp(),
                member_count: 0, // TODO: Get actual count
            }),
        }))
    }

    async fn get_room_members(
        &self,
        request: Request<GetRoomMembersRequest>,
    ) -> Result<Response<GetRoomMembersResponse>, Status> {
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Get members
        let members = self.room_service
            .get_room_members(&room_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get room members: {}", e)))?;

        // Convert to response
        let member_list = members.into_iter().map(|m| {
            RoomMember {
                room_id: room_id.to_string(),
                user_id: m.user_id.to_string(),
                username: m.username,
                permissions: m.permissions.0,
                joined_at: m.joined_at.timestamp(),
                is_online: m.is_online,
            }
        }).collect();

        Ok(Response::new(GetRoomMembersResponse {
            members: member_list,
        }))
    }

    async fn update_member_permission(
        &self,
        request: Request<UpdateMemberPermissionRequest>,
    ) -> Result<Response<UpdateMemberPermissionResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id.clone());
        let target_user_id = UserId::from_string(req.user_id);

        // Update permissions
        let member = self.room_service
            .update_member_permission(
                room_id,
                user_id,
                target_user_id,
                PermissionBits(req.permissions),
            )
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                _ => Status::internal("Failed to update member permission"),
            })?;

        Ok(Response::new(UpdateMemberPermissionResponse {
            member: Some(RoomMember {
                room_id: req.room_id,
                user_id: member.user_id.to_string(),
                username: String::new(), // TODO: Get username
                permissions: member.permissions.0,
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
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);
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

    async fn add_media(
        &self,
        request: Request<AddMediaRequest>,
    ) -> Result<Response<AddMediaResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Parse provider
        let provider = if req.provider.is_empty() {
            ProviderType::DirectUrl // Auto-detect would go here
        } else {
            ProviderType::from_str(&req.provider).unwrap_or(ProviderType::DirectUrl)
        };

        // Add movie
        let movie = self
            .room_service
            .add_media(room_id, user_id, req.url.clone(), provider, req.url)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                synctv_core::Error::NotFound(msg) => Status::not_found(msg),
                _ => Status::internal("Failed to add movie"),
            })?;

        // Convert to proto
        let proto_movie = Some(Media {
            id: movie.id.as_str().to_string(),
            room_id: movie.room_id.as_str().to_string(),
            url: movie.url,
            provider: movie.provider.as_str().to_string(),
            title: movie.title,
            metadata: serde_json::to_vec(&movie.metadata).unwrap_or_default(),
            position: movie.position,
            added_at: movie.added_at.timestamp(),
            added_by: movie.added_by.as_str().to_string(),
        });

        Ok(Response::new(AddMediaResponse { media: proto_movie }))
    }

    async fn remove_media(
        &self,
        request: Request<RemoveMediaRequest>,
    ) -> Result<Response<RemoveMediaResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);
        let media_id = MediaId::from_string(req.media_id);

        // Remove movie
        self.room_service
            .remove_media(room_id, user_id, media_id)
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                _ => Status::internal("Failed to remove movie"),
            })?;

        Ok(Response::new(RemoveMediaResponse { success: true }))
    }

    async fn get_playlist(
        &self,
        request: Request<GetPlaylistRequest>,
    ) -> Result<Response<GetPlaylistResponse>, Status> {
        let req = request.into_inner();

        let room_id = RoomId::from_string(req.room_id);

        // Get playlist
        let movies = self
            .room_service
            .get_playlist(room_id)
            .await
            .map_err(|_| Status::internal("Failed to get playlist"))?;

        // Convert to proto
        let proto_movies: Vec<Media> = movies
            .into_iter()
            .map(|m| Media {
                id: m.id.as_str().to_string(),
                room_id: m.room_id.as_str().to_string(),
                url: m.url,
                provider: m.provider.as_str().to_string(),
                title: m.title,
                metadata: serde_json::to_vec(&m.metadata).unwrap_or_default(),
                position: m.position,
                added_at: m.added_at.timestamp(),
                added_by: m.added_by.as_str().to_string(),
            })
            .collect();

        Ok(Response::new(GetPlaylistResponse { media: proto_movies }))
    }

    async fn swap_media(
        &self,
        request: Request<SwapMediaRequest>,
    ) -> Result<Response<SwapMediaResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);
        let media_id1 = MediaId::from_string(req.media_id1);
        let media_id2 = MediaId::from_string(req.media_id2);

        // Swap media positions
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

    async fn play(
        &self,
        request: Request<PlayRequest>,
    ) -> Result<Response<PlayResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Play
        let state = self
            .room_service
            .update_playback(
                room_id,
                user_id,
                |state| state.play(),
                PermissionBits::PLAY_PAUSE,
            )
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                _ => Status::internal("Failed to play"),
            })?;

        // Convert to proto
        let proto_state = Some(PlaybackState {
            room_id: state.room_id.as_str().to_string(),
            current_media_id: state.current_media_id.as_ref().map(|id| id.as_str().to_string()).unwrap_or_default(),
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
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Pause
        let state = self
            .room_service
            .update_playback(
                room_id,
                user_id,
                |state| state.pause(),
                PermissionBits::PLAY_PAUSE,
            )
            .await
            .map_err(|e| match e {
                synctv_core::Error::Authorization(msg) => Status::permission_denied(msg),
                _ => Status::internal("Failed to pause"),
            })?;

        // Convert to proto
        let proto_state = Some(PlaybackState {
            room_id: state.room_id.as_str().to_string(),
            current_media_id: state.current_media_id.as_ref().map(|id| id.as_str().to_string()).unwrap_or_default(),
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

    async fn seek(
        &self,
        request: Request<SeekRequest>,
    ) -> Result<Response<SeekResponse>, Status> {
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Seek
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

        // Convert to proto
        let proto_state = Some(PlaybackState {
            room_id: state.room_id.as_str().to_string(),
            current_media_id: state.current_media_id.as_ref().map(|id| id.as_str().to_string()).unwrap_or_default(),
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
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Change rate
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
                _ => Status::internal("Failed to change rate"),
            })?;

        // Convert to proto
        let proto_state = Some(PlaybackState {
            room_id: state.room_id.as_str().to_string(),
            current_media_id: state.current_media_id.as_ref().map(|id| id.as_str().to_string()).unwrap_or_default(),
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
        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);
        let media_id = MediaId::from_string(req.media_id);

        // Switch movie
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
                _ => Status::internal("Failed to switch movie"),
            })?;

        // Convert to proto
        let proto_state = Some(PlaybackState {
            room_id: state.room_id.as_str().to_string(),
            current_media_id: state.current_media_id.as_ref().map(|id| id.as_str().to_string()).unwrap_or_default(),
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
        let req = request.into_inner();

        let room_id = RoomId::from_string(req.room_id);

        // Get playback state
        let state = self
            .room_service
            .get_playback_state(&room_id)
            .await
            .map_err(|_| Status::internal("Failed to get playback state"))?;

        // Convert to proto
        let proto_state = Some(PlaybackState {
            room_id: state.room_id.as_str().to_string(),
            current_media_id: state.current_media_id.as_ref().map(|id| id.as_str().to_string()).unwrap_or_default(),
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

    type MessageStreamStream = std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<ServerMessage, Status>> + Send + 'static>,
    >;

    async fn message_stream(
        &self,
        request: Request<tonic::Streaming<ClientMessage>>,
    ) -> Result<Response<Self::MessageStreamStream>, Status> {
        use tokio::sync::mpsc;
        use nanoid::nanoid;

        // Extract user_id from JWT token
        let user_id = self.get_user_id(&request)?;

        // Get user details from service
        let user = self.user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get user: {}", e)))?;
        let username = user.username.clone();

        // Generate unique connection ID
        let connection_id = nanoid!(16);

        tracing::info!(
            user_id = %user_id.as_str(),
            connection_id = %connection_id,
            "Client establishing MessageStream connection"
        );

        // Register connection with connection manager
        if let Err(e) = self.connection_manager.register(connection_id.clone(), user_id.clone()) {
            tracing::warn!(
                user_id = %user_id.as_str(),
                error = %e,
                "Connection rejected by connection manager"
            );
            return Err(Status::resource_exhausted(e));
        }

        let mut client_stream = request.into_inner();

        // Create channel for outgoing messages
        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel::<ServerMessage>();

        // Track which room this connection is subscribed to (if any)
        let current_room: Arc<parking_lot::Mutex<Option<RoomId>>> = Arc::new(parking_lot::Mutex::new(None));

        // Clone for the task
        let message_hub = self.message_hub.clone();
        let room_service = self.room_service.clone();
        let connection_id_clone = connection_id.clone();
        let user_id_clone = user_id.clone();
        let username_clone = username.clone();
        let current_room_clone = current_room.clone();
        let outgoing_tx_clone = outgoing_tx.clone();
        let redis_publish_tx_clone = self.redis_publish_tx.clone();
        let rate_limiter_clone = self.rate_limiter.clone();
        let rate_limit_config_clone = self.rate_limit_config.clone();
        let content_filter_clone = self.content_filter.clone();
        let connection_manager_clone = self.connection_manager.clone();

        // Spawn task to handle incoming client messages
        tokio::spawn(async move {
            while let Ok(Some(client_msg)) = client_stream.message().await {
                if let Err(e) = Self::handle_client_message(
                    client_msg,
                    &message_hub,
                    &room_service,
                    &user_id_clone,
                    &username_clone,
                    &current_room_clone,
                    &connection_id_clone,
                    &outgoing_tx_clone,
                    &redis_publish_tx_clone,
                    &rate_limiter_clone,
                    &rate_limit_config_clone,
                    &content_filter_clone,
                    &connection_manager_clone,
                ).await {
                    tracing::error!(
                        error = %e,
                        user_id = %user_id_clone.as_str(),
                        "Error handling client message"
                    );

                    // Send error to client
                    let error_msg = ServerMessage {
                        message: Some(server_message::Message::Error(ErrorMessage {
                            code: "INTERNAL_ERROR".to_string(),
                            message: e.to_string(),
                        })),
                    };
                    let _ = outgoing_tx_clone.send(error_msg);
                }
            }

            // Client disconnected, cleanup
            if let Some(room_id) = current_room_clone.lock().as_ref() {
                message_hub.unsubscribe(&connection_id_clone);

                // Notify other users that this user left
                let event = ClusterEvent::UserLeft {
                    room_id: room_id.clone(),
                    user_id: user_id_clone.clone(),
                    username: username_clone.clone(),
                    timestamp: chrono::Utc::now(),
                };
                message_hub.broadcast(room_id, event);
            }

            // Unregister connection from connection manager
            connection_manager_clone.unregister(&connection_id_clone);

            tracing::info!(
                user_id = %user_id_clone.as_str(),
                connection_id = %connection_id_clone,
                "Client disconnected from MessageStream"
            );
        });

        // Convert outgoing channel to stream, wrapping items in Ok()
        let output_stream = UnboundedReceiverStream::new(outgoing_rx)
            .map(Ok::<_, Status>);

        Ok(Response::new(
            Box::pin(output_stream) as Self::MessageStreamStream
        ))
    }

    async fn get_chat_history(
        &self,
        request: Request<GetChatHistoryRequest>,
    ) -> Result<Response<GetChatHistoryResponse>, Status> {
        let req = request.into_inner();
        let _room_id = RoomId::from_string(req.room_id);

        // TODO: Implement ChatRepository and chat history retrieval
        // For now, return empty history
        Ok(Response::new(GetChatHistoryResponse {
            messages: vec![],
        }))
    }

    async fn logout(
        &self,
        _request: Request<LogoutRequest>,
    ) -> Result<Response<LogoutResponse>, Status> {
        // TODO: Implement token blacklist
        Ok(Response::new(LogoutResponse { success: true }))
    }

    async fn update_username(
        &self,
        request: Request<UpdateUsernameRequest>,
    ) -> Result<Response<UpdateUsernameResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        // TODO: Implement username update
        Err(Status::unimplemented("UpdateUsername not yet implemented"))
    }

    async fn update_password(
        &self,
        request: Request<UpdatePasswordRequest>,
    ) -> Result<Response<UpdatePasswordResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        // TODO: Implement password update with old password verification
        Err(Status::unimplemented("UpdatePassword not yet implemented"))
    }

    async fn get_my_rooms(
        &self,
        request: Request<GetMyRoomsRequest>,
    ) -> Result<Response<GetMyRoomsResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        // TODO: Implement get my rooms (rooms created by user)
        Ok(Response::new(GetMyRoomsResponse {
            rooms: vec![],
            total: 0,
        }))
    }

    async fn get_joined_rooms(
        &self,
        request: Request<GetJoinedRoomsRequest>,
    ) -> Result<Response<GetJoinedRoomsResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        // TODO: Implement get joined rooms
        Ok(Response::new(GetJoinedRoomsResponse {
            rooms: vec![],
            total: 0,
        }))
    }

    async fn check_room(
        &self,
        request: Request<CheckRoomRequest>,
    ) -> Result<Response<CheckRoomResponse>, Status> {
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        match self.room_service.get_room(&room_id).await {
            Ok(room) => {
                // Check if room requires password
                let requires_password = room.settings
                    .get("password")
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);

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

    async fn get_hot_rooms(
        &self,
        request: Request<GetHotRoomsRequest>,
    ) -> Result<Response<GetHotRoomsResponse>, Status> {
        let req = request.into_inner();
        let limit = if req.limit == 0 || req.limit > 50 { 10 } else { req.limit };

        // TODO: Implement hot rooms with online count sorting
        Ok(Response::new(GetHotRoomsResponse {
            rooms: vec![],
        }))
    }
}
