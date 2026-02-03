//! Unified Message Stream Implementation
//!
//! This module provides a unified implementation for handling real-time messaging
//! that can be used by both gRPC streaming and WebSocket connections.
//!
//! Architecture:
//! - Binary proto encoding/decoding
//! - Shared business logic in impls layer
//! - Transport-agnostic message handling via MessageSender and StreamMessage traits
//! - Cluster-aware broadcasting (local + Redis)
//! - All logic encapsulated in StreamMessageHandler (rate limiting, filtering, permissions)
//! - Complete IO abstraction via StreamMessage trait for both sending and receiving

use std::sync::Arc;
use prost::Message;
use synctv_core::{
    models::{RoomId, UserId, PermissionBits},
    service::{ContentFilter, RateLimitConfig, RateLimiter, RoomService},
};
use synctv_cluster::sync::{ClusterEvent, ClusterManager};

use crate::proto::client::{ClientMessage, ServerMessage};

/// Trait for sending server messages to clients
///
/// Implemented by both gRPC streaming and WebSocket transports
pub trait MessageSender: Send + Sync {
    /// Send a server message to the client
    fn send(&self, message: ServerMessage) -> Result<(), String>;
}

/// Unified IO abstraction for bidirectional messaging
///
/// This trait encapsulates both sending and receiving operations for real-time communication.
/// Implemented by both WebSocket and gRPC streaming transports, allowing complete code reuse.
///
/// The key insight is that WebSocket and gRPC streaming are conceptually identical:
/// - Both are bidirectional byte streams
/// - Both use proto encoding
/// - Both need the same business logic (rate limiting, permissions, broadcasting)
///
/// By implementing this trait, we ensure that ALL connection handling logic lives in impls/,
/// with the transport layer (http/, grpc/) providing only the IO implementation.
#[async_trait::async_trait]
pub trait StreamMessage: Send + Sync {
    /// Receive a client message (blocking/async)
    ///
    /// Returns None when the connection is closed
    async fn recv(&mut self) -> Option<Result<ClientMessage, String>>;

    /// Send a server message
    fn send(&self, message: ServerMessage) -> Result<(), String>;

    /// Check if connection is still alive
    fn is_alive(&self) -> bool;
}

/// Per-connection stream message handler with complete logic encapsulation
///
/// Each connection gets its own handler instance with:
/// - Connection state (room_id, user_id, username)
/// - Message I/O channels
/// - Rate limiting, content filtering, permission checking
/// - Cluster broadcasting
///
/// The handler runs its own message loop, external code only needs to:
/// 1. Create the handler with proper I/O channels
/// 2. Call start() to begin processing
pub struct StreamMessageHandler {
    room_id: RoomId,
    user_id: UserId,
    username: String,
    room_service: Arc<RoomService>,
    cluster_manager: Arc<ClusterManager>,
    rate_limiter: Arc<RateLimiter>,
    rate_limit_config: Arc<RateLimitConfig>,
    content_filter: Arc<ContentFilter>,
    sender: Arc<dyn MessageSender>,
}

impl Clone for StreamMessageHandler {
    fn clone(&self) -> Self {
        Self {
            room_id: self.room_id.clone(),
            user_id: self.user_id.clone(),
            username: self.username.clone(),
            room_service: Arc::clone(&self.room_service),
            cluster_manager: Arc::clone(&self.cluster_manager),
            rate_limiter: Arc::clone(&self.rate_limiter),
            rate_limit_config: Arc::clone(&self.rate_limit_config),
            content_filter: Arc::clone(&self.content_filter),
            sender: Arc::clone(&self.sender),
        }
    }
}

impl StreamMessageHandler {
    /// Create a new stream message handler
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        room_id: RoomId,
        user_id: UserId,
        username: String,
        room_service: Arc<RoomService>,
        cluster_manager: Arc<ClusterManager>,
        rate_limiter: Arc<RateLimiter>,
        rate_limit_config: Arc<RateLimitConfig>,
        content_filter: Arc<ContentFilter>,
        sender: Arc<dyn MessageSender>,
    ) -> Self {
        Self {
            room_id,
            user_id,
            username,
            room_service,
            cluster_manager,
            rate_limiter,
            rate_limit_config,
            content_filter,
            sender,
        }
    }

    /// Run the complete message loop using unified IO abstraction
    ///
    /// This is the NEW recommended method that handles both sending and receiving
    /// in a single unified loop using the StreamMessage trait.
    ///
    /// This method:
    /// 1. Subscribes to cluster events and forwards them to the client
    /// 2. Receives client messages via the StreamMessage trait
    /// 3. Handles rate limiting, content filtering, and permissions
    /// 4. Broadcasts events to the cluster
    /// 5. Handles cleanup on disconnect
    ///
    /// The caller only needs to provide a StreamMessage implementation (WebSocket or gRPC).
    pub async fn run<S: StreamMessage>(&self, stream: &mut S) -> Result<(), String> {
        let room_id_str = self.room_id.as_str().to_string();

        // Subscribe to cluster events
        let (mut event_rx, _connection_id) = self.cluster_manager.subscribe(
            self.room_id.clone(),
            self.user_id.clone()
        );

        // Send initial user joined notification
        stream.send(self.create_user_joined_message(&room_id_str))?;

        // Main message loop using tokio::select! for concurrent operations
        loop {
            tokio::select! {
                // Incoming client message
                client_msg_result = stream.recv() => {
                    match client_msg_result {
                        Some(Ok(msg)) => {
                            if let Err(e) = self.handle_client_message(&msg).await {
                                tracing::error!("Failed to handle client message: {}", e);
                                // Don't break on individual message errors, continue processing
                            }
                        }
                        Some(Err(e)) => {
                            tracing::error!("Error receiving message: {}", e);
                            break;
                        }
                        None => {
                            tracing::info!("Client disconnected gracefully");
                            break;
                        }
                    }
                }

                // Cluster event (broadcast to client)
                event = event_rx.recv() => {
                    match event {
                        Some(event) => {
                            if let Some(msg) = cluster_event_to_server_message(&event, &room_id_str) {
                                if let Err(e) = stream.send(msg) {
                                    tracing::error!("Failed to send server message: {}", e);
                                    break;
                                }
                            }
                        }
                        None => {
                            tracing::error!("Cluster event channel closed");
                            break;
                        }
                    }
                }

                // Heartbeat/health check every 30 seconds
                _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                    if !stream.is_alive() {
                        tracing::info!("Connection no longer alive");
                        break;
                    }
                }
            }
        }

        // Cleanup: notify cluster that user left
        self.cleanup(&room_id_str).await;

        Ok(())
    }

    /// Create initial user joined message
    fn create_user_joined_message(&self, room_id: &str) -> ServerMessage {
        use crate::proto::client::server_message::Message;
        use crate::proto::client::{UserJoinedRoom, RoomMember};

        ServerMessage {
            message: Some(Message::UserJoined(UserJoinedRoom {
                room_id: room_id.to_string(),
                member: Some(RoomMember {
                    room_id: room_id.to_string(),
                    user_id: self.user_id.as_str().to_string(),
                    username: self.username.clone(),
                    role: "member".to_string(),
                    permissions: 0,
                    added_permissions: 0,
                    removed_permissions: 0,
                    admin_added_permissions: 0,
                    admin_removed_permissions: 0,
                    joined_at: chrono::Utc::now().timestamp(),
                    is_online: true,
                }),
            })),
        }
    }

    /// Cleanup on disconnect
    async fn cleanup(&self, room_id: &str) {
        // Notify cluster that user left
        let event = ClusterEvent::UserLeft {
            room_id: self.room_id.clone(),
            user_id: self.user_id.clone(),
            username: self.username.clone(),
            timestamp: chrono::Utc::now(),
        };
        let _ = self.cluster_manager.broadcast(event);

        tracing::info!(
            "Cleanup complete for user {} in room {}",
            self.username,
            room_id
        );
    }

    /// Start the message handling loop
    ///
    /// This method:
    /// 1. Subscribes to cluster events and forwards them to the client
    /// 2. Spawns a task to handle incoming client messages
    /// 3. Returns a sender that the caller should use to send ClientMessages to this handler
    ///
    /// Returns a sender that the caller should use to send ClientMessages
    pub fn start(
        &self,
    ) -> tokio::sync::mpsc::UnboundedSender<ClientMessage> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ClientMessage>();

        // Subscribe to cluster events and forward to client
        let room_id = self.room_id.clone();
        let user_id = self.user_id.clone();
        let room_id_str = room_id.as_str().to_string();
        let (mut rx_events, _connection_id) = self.cluster_manager.subscribe(room_id, user_id);
        let sender = self.sender.clone();

        tokio::spawn(async move {
            while let Some(event) = rx_events.recv().await {
                if let Some(msg) = cluster_event_to_server_message(&event, &room_id_str) {
                    if let Err(e) = sender.send(msg) {
                        tracing::error!("Failed to send message: {}", e);
                        break;
                    }
                }
            }
        });

        // Spawn task to handle incoming messages
        let handler = self.clone();
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Err(e) = handler.handle_client_message(&msg).await {
                    tracing::error!("Failed to handle client message: {}", e);
                }
            }
        });

        tx
    }

    /// Handle incoming client message with all validations
    pub async fn handle_client_message(&self, msg: &ClientMessage) -> Result<(), String> {
        use crate::proto::client::client_message::Message;

        match &msg.message {
            Some(Message::Chat(chat_msg)) => {
                // Check rate limit
                let rate_limit_key = format!("user:{}:chat", self.user_id.as_str());
                self.rate_limiter
                    .check_rate_limit(
                        &rate_limit_key,
                        self.rate_limit_config.chat_per_second,
                        self.rate_limit_config.window_seconds,
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                // Filter and sanitize content
                let sanitized_content = self.content_filter
                    .filter_chat(&chat_msg.content)
                    .map_err(|e| e.to_string())?;

                // Check permission
                self.room_service
                    .check_permission(&self.room_id, &self.user_id, PermissionBits::SEND_CHAT)
                    .await
                    .map_err(|e| e.to_string())?;

                self.handle_chat_message(&sanitized_content).await?;
            }
            Some(Message::Danmaku(danmaku_msg)) => {
                // Check rate limit
                let rate_limit_key = format!("user:{}:danmaku", self.user_id.as_str());
                self.rate_limiter
                    .check_rate_limit(
                        &rate_limit_key,
                        self.rate_limit_config.danmaku_per_second,
                        self.rate_limit_config.window_seconds,
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                // Filter and sanitize content
                let sanitized_content = self.content_filter
                    .filter_danmaku(&danmaku_msg.content)
                    .map_err(|e| e.to_string())?;

                // Check permission
                self.room_service
                    .check_permission(&self.room_id, &self.user_id, PermissionBits::SEND_DANMAKU)
                    .await
                    .map_err(|e| e.to_string())?;

                self.handle_danmaku(&sanitized_content, danmaku_msg.position).await?;
            }
            Some(Message::Heartbeat(_)) => {
                // Heartbeat doesn't need to be broadcast
            }
            None => {
                return Err("Empty message".to_string());
            }
        }

        Ok(())
    }

    async fn handle_chat_message(&self, content: &str) -> Result<(), String> {
        // Save to database
        let _saved_msg = self
            .room_service
            .save_chat_message(
                self.room_id.clone(),
                self.user_id.clone(),
                content.to_string(),
            )
            .await
            .map_err(|e| e.to_string())?;

        let event = ClusterEvent::ChatMessage {
            room_id: self.room_id.clone(),
            user_id: self.user_id.clone(),
            username: self.username.clone(),
            message: content.to_string(),
            timestamp: chrono::Utc::now(),
        };

        // Broadcast to cluster (handles both local and Redis)
        let _result = self.cluster_manager.broadcast(event);

        Ok(())
    }

    async fn handle_danmaku(&self, content: &str, position: i32) -> Result<(), String> {
        let event = ClusterEvent::Danmaku {
            room_id: self.room_id.clone(),
            user_id: self.user_id.clone(),
            username: self.username.clone(),
            message: content.to_string(),
            position: position as f64,
            timestamp: chrono::Utc::now(),
        };

        // Broadcast to cluster (handles both local and Redis)
        let _result = self.cluster_manager.broadcast(event);

        Ok(())
    }

    /// Get room ID
    pub fn get_room_id(&self) -> &RoomId {
        &self.room_id
    }

    /// Get user ID
    pub fn get_user_id(&self) -> UserId {
        self.user_id.clone()
    }
}

/// Convert cluster event to server message
fn cluster_event_to_server_message(
    event: &synctv_cluster::sync::ClusterEvent,
    room_id: &str,
) -> Option<ServerMessage> {
    use crate::proto::client::server_message::Message;
    use crate::proto::client::*;
    use synctv_cluster::sync::ClusterEvent;

    match event {
        ClusterEvent::ChatMessage { username, message, timestamp, .. } => {
            Some(ServerMessage {
                message: Some(Message::Chat(ChatMessageReceive {
                    id: nanoid::nanoid!(12),
                    room_id: room_id.to_string(),
                    user_id: username.clone(),
                    username: username.clone(),
                    content: message.clone(),
                    timestamp: timestamp.timestamp_micros(),
                })),
            })
        }
        ClusterEvent::Danmaku { username, message, position, timestamp, .. } => {
            Some(ServerMessage {
                message: Some(Message::Danmaku(DanmakuMessageReceive {
                    room_id: room_id.to_string(),
                    user_id: username.clone(),
                    content: message.clone(),
                    color: "#FFFFFF".to_string(),
                    position: *position as i32,
                    timestamp: timestamp.timestamp_micros(),
                })),
            })
        }
        ClusterEvent::PlaybackStateChanged { state, .. } => {
            Some(ServerMessage {
                message: Some(Message::PlaybackState(PlaybackStateChanged {
                    room_id: room_id.to_string(),
                    state: Some(PlaybackState {
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
                    }),
                })),
            })
        }
        ClusterEvent::UserJoined { user_id, username, permissions, .. } => {
            Some(ServerMessage {
                message: Some(Message::UserJoined(UserJoinedRoom {
                    room_id: room_id.to_string(),
                    member: Some(RoomMember {
                        room_id: room_id.to_string(),
                        user_id: user_id.as_str().to_string(),
                        username: username.clone(),
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
            })
        }
        ClusterEvent::UserLeft { user_id, .. } => {
            Some(ServerMessage {
                message: Some(Message::UserLeft(UserLeftRoom {
                    room_id: room_id.to_string(),
                    user_id: user_id.as_str().to_string(),
                })),
            })
        }
        ClusterEvent::MediaAdded { media_id, media_title, user_id, username, .. } => {
            Some(ServerMessage {
                message: Some(Message::MediaAdded(crate::proto::client::MediaAdded {
                    room_id: room_id.to_string(),
                    media_id: media_id.as_str().to_string(),
                    title: media_title.clone(),
                    added_by: username.clone(),
                    added_by_user_id: user_id.as_str().to_string(),
                })),
            })
        }
        ClusterEvent::MediaRemoved { media_id, user_id, username, .. } => {
            Some(ServerMessage {
                message: Some(Message::MediaRemoved(crate::proto::client::MediaRemoved {
                    room_id: room_id.to_string(),
                    media_id: media_id.as_str().to_string(),
                    removed_by: username.clone(),
                    removed_by_user_id: user_id.as_str().to_string(),
                })),
            })
        }
        ClusterEvent::PermissionChanged { target_user_id, new_permissions, changed_by_username, .. } => {
            Some(ServerMessage {
                message: Some(Message::PermissionChanged(crate::proto::client::PermissionChanged {
                    room_id: room_id.to_string(),
                    user_id: target_user_id.as_str().to_string(),
                    role: String::new(),
                    effective_permissions: new_permissions.0,
                    added_permissions: 0,
                    removed_permissions: 0,
                    admin_added_permissions: 0,
                    admin_removed_permissions: 0,
                    updated_by: changed_by_username.clone(),
                })),
            })
        }
        ClusterEvent::RoomSettingsChanged { .. } => {
            Some(ServerMessage {
                message: Some(Message::RoomSettings(RoomSettingsChanged {
                    room_id: room_id.to_string(),
                    settings: serde_json::to_vec(&serde_json::json!({}))
                        .unwrap_or_default(),
                })),
            })
        }
        ClusterEvent::SystemNotification { message, level, .. } => {
            let code = match level {
                synctv_cluster::sync::events::NotificationLevel::Info => "INFO",
                synctv_cluster::sync::events::NotificationLevel::Warning => "WARNING",
                synctv_cluster::sync::events::NotificationLevel::Error => "ERROR",
            };
            Some(ServerMessage {
                message: Some(Message::Error(ErrorMessage {
                    code: code.to_string(),
                    message: message.clone(),
                })),
            })
        }
    }
}

/// Binary codec for proto messages
pub struct ProtoCodec;

impl ProtoCodec {
    /// Encode ClientMessage to binary
    pub fn encode_client_message(msg: &ClientMessage) -> Result<Vec<u8>, String> {
        Ok(msg.encode_to_vec())
    }

    /// Decode ClientMessage from binary
    pub fn decode_client_message(data: &[u8]) -> Result<ClientMessage, String> {
        ClientMessage::decode(data)
            .map_err(|e| format!("Failed to decode message: {}", e))
    }

    /// Encode ServerMessage to binary
    pub fn encode_server_message(msg: &ServerMessage) -> Result<Vec<u8>, String> {
        Ok(msg.encode_to_vec())
    }

    /// Decode ServerMessage from binary
    pub fn decode_server_message(data: &[u8]) -> Result<ServerMessage, String> {
        ServerMessage::decode(data)
            .map_err(|e| format!("Failed to decode message: {}", e))
    }
}
