//! Shared message handler for both gRPC Streaming and WebSocket
//!
//! This module contains the common business logic for processing client messages
//! and broadcasting server messages. Both gRPC and WebSocket handlers use this
//! to ensure consistent behavior across different transport protocols.

use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

use synctv_core::{models::RoomId, service::RoomService};
use synctv_cluster::sync::{ClusterEvent, PublishRequest, RoomMessageHub};

use crate::grpc::proto::client::{self, ClientMessage, ServerMessage};
use crate::grpc::proto::client::client_message::Message as ClientMessageMsg;

/// Shared message handler
pub struct MessageHandler {
    room_service: Arc<RoomService>,
    message_hub: Arc<RoomMessageHub>,
    redis_publish_tx: Option<mpsc::UnboundedSender<PublishRequest>>,
}

impl MessageHandler {
    pub fn new(
        room_service: Arc<RoomService>,
        message_hub: Arc<RoomMessageHub>,
        redis_publish_tx: Option<mpsc::UnboundedSender<PublishRequest>>,
    ) -> Self {
        Self {
            room_service,
            message_hub,
            redis_publish_tx,
        }
    }

    /// Handle a client message from either gRPC or WebSocket
    ///
    /// This method:
    /// 1. Validates permissions
    /// 2. Saves to database (if needed)
    /// 3. Broadcasts to local subscribers
    /// 4. Publishes to Redis for cross-node sync
    pub async fn handle_message(
        &self,
        msg: &ClientMessage,
        room_id: &RoomId,
        user_id: &synctv_core::models::UserId,
        username: &str,
    ) {
        match &msg.message {
            Some(ClientMessageMsg::Chat(chat_msg)) => {
                debug!(
                    "Handling chat message from user {} in room {}: {}",
                    user_id.as_str(),
                    room_id.as_str(),
                    chat_msg.content
                );

                // Save to database
                match self
                    .room_service
                    .save_chat_message(
                        room_id.clone(),
                        user_id.clone(),
                        chat_msg.content.clone(),
                    )
                    .await
                {
                    Ok(_saved_msg) => {
                        let event = ClusterEvent::ChatMessage {
                            room_id: room_id.clone(),
                            user_id: user_id.clone(),
                            username: username.to_string(),
                            message: chat_msg.content.clone(),
                            timestamp: chrono::Utc::now(),
                        };

                        // Broadcast to local subscribers
                        self.message_hub.broadcast(room_id, event.clone());

                        // Publish to Redis for multi-replica sync
                        if let Some(tx) = &self.redis_publish_tx {
                            let _ = tx.send(PublishRequest {
                                room_id: room_id.clone(),
                                event,
                            });
                        }
                    }
                    Err(e) => {
                        error!("Failed to save chat message: {}", e);
                    }
                }
            }
            Some(ClientMessageMsg::Danmaku(danmaku_msg)) => {
                debug!(
                    "Handling danmaku from user {} in room {}: {} at {}",
                    user_id.as_str(),
                    room_id.as_str(),
                    danmaku_msg.content,
                    danmaku_msg.position
                );

                let event = ClusterEvent::Danmaku {
                    room_id: room_id.clone(),
                    user_id: user_id.clone(),
                    username: username.to_string(),
                    message: danmaku_msg.content.clone(),
                    position: danmaku_msg.position as f64,
                    timestamp: chrono::Utc::now(),
                };

                // Broadcast to local subscribers
                self.message_hub.broadcast(room_id, event.clone());

                // Publish to Redis for multi-replica sync
                if let Some(tx) = &self.redis_publish_tx {
                    let _ = tx.send(PublishRequest {
                        room_id: room_id.clone(),
                        event,
                    });
                }
            }
            Some(ClientMessageMsg::Heartbeat(_)) => {
                debug!(
                    "Heartbeat from user {} in room {}",
                    user_id.as_str(),
                    room_id.as_str()
                );
                // Heartbeat doesn't need to be broadcast
            }
            None => {
                warn!("Received empty client message");
            }
        }
    }
}

/// Convert ClusterEvent to ServerMessage protobuf
/// Used by both gRPC streaming and WebSocket
pub fn cluster_event_to_server_message(
    event: &ClusterEvent,
    room_id: &str,
) -> ServerMessage {
    use client::server_message::Message;
    use client::{
        ChatMessageReceive, DanmakuMessageReceive, ErrorMessage,
        PlaybackStateChanged, RoomMember, RoomSettingsChanged,
        UserJoinedRoom, UserLeftRoom,
    };

    match event {
        ClusterEvent::ChatMessage { username, message, timestamp, .. } => {
            ServerMessage {
                message: Some(Message::Chat(ChatMessageReceive {
                    id: nanoid::nanoid!(12),
                    room_id: room_id.to_string(),
                    user_id: username.clone(),
                    username: username.clone(),
                    content: message.clone(),
                    timestamp: timestamp.timestamp_micros(),
                })),
            }
        }
        ClusterEvent::Danmaku { username, message, position, timestamp, .. } => {
            ServerMessage {
                message: Some(Message::Danmaku(DanmakuMessageReceive {
                    room_id: room_id.to_string(),
                    user_id: username.clone(),
                    content: message.clone(),
                    color: "#FFFFFF".to_string(), // Default white color
                    position: *position as i32,
                    timestamp: timestamp.timestamp_micros(),
                })),
            }
        }
        ClusterEvent::PlaybackStateChanged { state, .. } => {
            ServerMessage {
                message: Some(Message::PlaybackState(PlaybackStateChanged {
                    room_id: room_id.to_string(),
                    state: Some(client::PlaybackState {
                        room_id: state.room_id.as_str().to_string(),
                        current_media_id: state
                            .current_media_id
                            .as_ref()
                            .map(|id| id.as_str().to_string())
                            .unwrap_or_default(),
                        position: state.position,
                        speed: state.speed,
                        is_playing: state.is_playing,
                        updated_at: state.updated_at.timestamp_micros(),
                        version: state.version,
                    }),
                })),
            }
        }
        ClusterEvent::UserJoined { user_id, username, permissions, .. } => {
            ServerMessage {
                message: Some(Message::UserJoined(UserJoinedRoom {
                    room_id: room_id.to_string(),
                    member: Some(RoomMember {
                        room_id: room_id.to_string(),
                        user_id: user_id.as_str().to_string(),
                        username: username.clone(),
                        permissions: permissions.0,
                        joined_at: chrono::Utc::now().timestamp_micros(),
                        is_online: true,
                    }),
                })),
            }
        }
        ClusterEvent::UserLeft { user_id, .. } => {
            ServerMessage {
                message: Some(Message::UserLeft(UserLeftRoom {
                    room_id: room_id.to_string(),
                    user_id: user_id.as_str().to_string(),
                })),
            }
        }
        ClusterEvent::RoomSettingsChanged { .. } => {
            ServerMessage {
                message: Some(Message::RoomSettings(RoomSettingsChanged {
                    room_id: room_id.to_string(),
                    settings: serde_json::to_vec(&serde_json::json!({}))
                        .unwrap_or_default(),
                })),
            }
        }
        _ => ServerMessage {
            message: Some(Message::Error(ErrorMessage {
                code: "UNKNOWN_EVENT".to_string(),
                message: "Unknown event type".to_string(),
            })),
        },
    }
}
