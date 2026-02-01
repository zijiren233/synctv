//! WebSocket handler for real-time messaging
//!
//! Provides WebSocket endpoint using the same protobuf messages as gRPC
//! Uses shared MessageHandler for consistent business logic

use std::sync::Arc;
use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    response::IntoResponse,
};
use futures::{stream::StreamExt, SinkExt};
use prost::Message as ProstMessage;
use serde::Deserialize;
use tracing::{debug, error, info, warn};

use crate::grpc::message_handler::MessageHandler;
use synctv_proto::client::ClientMessage;
use crate::http::AppState;
use synctv_core::models::RoomId;

/// WebSocket connection parameters from query string
#[derive(Debug, Deserialize)]
pub struct WSParams {
    token: String,
}

/// Handle WebSocket connection for a room
///
/// # Route
/// GET /ws/rooms/:room_id?token=<jwt_token>
///
/// # Query Parameters
/// - token: JWT access token
///
/// # Protocol
/// - Sends/receives ClientMessage and ServerMessage protobuf (binary only)
pub async fn websocket_handler(
    Path(room_id): Path<String>,
    Query(params): Query<WSParams>,
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, room_id, params.token, state))
}

/// Handle an upgraded WebSocket connection
async fn handle_socket(
    mut socket: WebSocket,
    room_id: String,
    token: String,
    state: AppState,
) {
    // Verify JWT token
    let claims = match state.jwt_service.verify_access_token(&token) {
        Ok(claims) => claims,
        Err(e) => {
            warn!("WebSocket authentication failed: {}", e);
            send_error(&mut socket, "Authentication failed").await;
            return;
        }
    };

    let user_id = synctv_core::models::UserId(claims.sub.clone());
    let username = claims.sub.clone();

    // Verify room membership
    let room_id_typed = RoomId(room_id.clone());
    if let Err(e) = state.room_service.check_membership(&room_id_typed, &user_id).await {
        warn!(
            "WebSocket: User {} not a member of room {}: {}",
            user_id.as_str(), room_id, e
        );
        send_error(&mut socket, "Not a member of this room").await;
        return;
    }

    // Generate connection ID
    let connection_id = format!("ws_{}_{}", user_id.as_str(), nanoid::nanoid!(8));

    info!(
        "WebSocket connected: user {} ({}) in room {}",
        user_id.as_str(), connection_id, room_id
    );

    // Subscribe to room hub
    let mut event_rx = state.message_hub.subscribe(
        room_id_typed.clone(),
        user_id.clone(),
        connection_id.clone(),
    );

    // Split socket
    let (mut sender, mut receiver) = socket.split();

    // Task 1: Forward hub events to WebSocket
    let room_id_clone = room_id.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let server_msg = crate::grpc::message_handler::cluster_event_to_server_message(&event, &room_id_clone);

            // Encode to binary protobuf
            let mut buf = Vec::new();
            if let Err(e) = server_msg.encode(&mut buf) {
                error!("Failed to encode protobuf: {}", e);
                break;
            }

            if sender.send(WsMessage::Binary(buf)).await.is_err() {
                warn!("Failed to send WebSocket message");
                break;
            }
        }
    });

    // Task 2: Handle incoming WebSocket messages (uses shared handler)
    let room_id_typed_clone = room_id_typed.clone();
    let user_id_clone = user_id.clone();
    let username_clone = username.clone();

    // Create message handler with Redis Pub/Sub support
    let message_handler = Arc::new(MessageHandler::new(
        state.room_service.clone(),
        state.message_hub.clone(),
        state.redis_publish_tx,
    ));

    while let Some(result) = receiver.next().await {
        match result {
            Ok(WsMessage::Binary(data)) => {
                debug!("WebSocket: Received protobuf message: {} bytes", data.len());

                let client_msg = match ClientMessage::decode(&*data) {
                    Ok(msg) => msg,
                    Err(e) => {
                        warn!("Failed to decode protobuf: {}", e);
                        continue;
                    }
                };

                // Use shared message handler (includes local broadcast + Redis Pub/Sub)
                message_handler.handle_message(
                    &client_msg,
                    &room_id_typed_clone,
                    &user_id_clone,
                    &username_clone,
                ).await;
            }
            Ok(WsMessage::Close(_)) => {
                info!("WebSocket closed by client");
                break;
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
            Ok(WsMessage::Text(_)) | Ok(WsMessage::Ping(_)) | Ok(WsMessage::Pong(_)) => {
                // Ignore non-binary messages
            }
        }
    }

    // Unsubscribe
    state.message_hub.unsubscribe(&connection_id);

    info!(
        "WebSocket disconnected: user {} from room {}",
        user_id.as_str(), room_id
    );
}

/// Send error message to WebSocket
async fn send_error(socket: &mut WebSocket, message_text: &str) {
    use synctv_proto::client::{self, server_message::Message};

    let server_msg = client::ServerMessage {
        message: Some(Message::Error(client::ErrorMessage {
            code: "AUTH_FAILED".to_string(),
            message: message_text.to_string(),
        })),
    };

    let mut buf = Vec::new();
    if server_msg.encode(&mut buf).is_ok() {
        let _ = socket.send(WsMessage::Binary(buf)).await;
    }
}
