//! WebSocket handler with binary proto transmission
//!
//! This handler uses the unified StreamMessageHandler from impls layer,
//! enabling full code reuse between gRPC and WebSocket.

use axum::{
    extract::{Path, State, WebSocketUpgrade},
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use tracing::{error, info};

use crate::http::AppState;
use crate::impls::messaging::{StreamMessageHandler, MessageSender, ProtoCodec};
use synctv_core::models::{RoomId, UserId};

/// WebSocket message sender implementation
struct WebSocketMessageSender {
    sender: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

impl WebSocketMessageSender {
    fn new(sender: tokio::sync::mpsc::UnboundedSender<Vec<u8>>) -> Self {
        Self { sender }
    }
}

impl MessageSender for WebSocketMessageSender {
    fn send(&self, message: crate::proto::client::ServerMessage) -> Result<(), String> {
        // Encode to binary proto
        let bytes = ProtoCodec::encode_server_message(&message)?;

        // Send via channel
        self.sender
            .send(bytes)
            .map_err(|e| format!("Failed to send message: {}", e))
    }
}

/// WebSocket handler for room real-time updates
pub async fn websocket_handler(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state, room_id))
}

async fn handle_socket(
    mut socket: axum::extract::ws::WebSocket,
    state: AppState,
    room_id: String,
) {
    // TODO: Extract user_id from JWT (similar to gRPC interceptor)
    let user_id = UserId::new();
    let username = "anonymous".to_string();

    info!(
        "WebSocket connection established: user={}, room={}",
        user_id.as_str(),
        room_id
    );

    // Check if cluster_manager is available
    let cluster_manager = match state.cluster_manager {
        Some(cm) => cm,
        None => {
            error!("ClusterManager not available, WebSocket connection not supported");
            let _ = socket.close().await;
            return;
        }
    };

    // Create channel for sending messages to WebSocket
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

    // Create WebSocket sender
    let ws_sender = Box::new(WebSocketMessageSender::new(tx));

    // Create StreamMessageHandler with all configuration
    let rid = RoomId::from_string(room_id.clone());
    let stream_handler = StreamMessageHandler::new(
        rid.clone(),
        user_id.clone(),
        username,
        state.room_service.clone(),
        cluster_manager,
        state.rate_limiter.clone(),
        state.rate_limit_config.clone(),
        state.content_filter.clone(),
        ws_sender,
    );

    // Start the handler and get receiver for incoming messages
    let mut client_msg_rx = stream_handler.start();

    // Spawn task to handle server messages -> WebSocket
    let mut send_socket = socket.clone();
    tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if let Err(e) = send_socket
                .send(axum::extract::ws::Message::Binary(bytes))
                .await
            {
                error!("Failed to send WebSocket message: {}", e);
                break;
            }
        }
    });

    // Handle WebSocket messages -> stream handler
    while let Some(result) = socket.recv().await {
        match result {
            Ok(axum::extract::ws::Message::Binary(bytes)) => {
                // Decode binary proto and send to handler
                match ProtoCodec::decode_client_message(&bytes) {
                    Ok(client_msg) => {
                        if let Err(e) = client_msg_rx.send(client_msg) {
                            error!("Failed to send message to handler: {}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Failed to decode client message: {}", e);
                    }
                }
            }
            Ok(axum::extract::ws::Message::Close(_)) => {
                info!("WebSocket connection closed by client");
                break;
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
            _ => {
                // Ignore non-binary messages
            }
        }
    }

    info!(
        "WebSocket connection closed: user={}, room={}",
        user_id.as_str(),
        room_id
    );
}
