//! WebSocket handler with binary proto transmission
//!
//! This handler uses the unified StreamMessage trait from impls layer,
//! enabling full code reuse between gRPC and WebSocket.
//!
//! All business logic (rate limiting, content filtering, permissions, broadcasting)
//! is handled by StreamMessageHandler.run() with the WebSocketStream implementation.

use axum::{
    extract::{Path, State, WebSocketUpgrade},
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::http::AppState;
use crate::impls::messaging::{StreamMessageHandler, StreamMessage, ProtoCodec, MessageSender};
use synctv_core::models::{RoomId, UserId};
use synctv_core::service::{RateLimiter, RateLimitConfig, ContentFilter, auth::JwtValidator};
use crate::proto::client::{ClientMessage, ServerMessage};

/// WebSocket stream implementation of StreamMessage trait
///
/// This adapts WebSocket's axum::extract::ws::WebSocket to our unified StreamMessage interface.
struct WebSocketStream {
    receiver: futures::stream::SplitStream<axum::extract::ws::WebSocket>,
    sender: WebSocketMessageSender,
    _is_alive: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl StreamMessage for WebSocketStream {
    async fn recv(&mut self) -> Option<Result<ClientMessage, String>> {
        match self.receiver.next().await {
            Some(Ok(axum::extract::ws::Message::Binary(bytes))) => {
                Some(ProtoCodec::decode_client_message(&bytes))
            }
            Some(Ok(axum::extract::ws::Message::Close(_))) => {
                None // Graceful close
            }
            Some(Err(e)) => Some(Err(format!("WebSocket error: {}", e))),
            None => None, // Stream ended
            Some(Ok(_)) => {
                // Ignore non-binary messages (text, ping, pong)
                // Continue waiting for next message - recursively call recv
                self.recv().await
            }
        }
    }

    fn send(&self, message: ServerMessage) -> Result<(), String> {
        MessageSender::send(&self.sender, message)
    }

    fn is_alive(&self) -> bool {
        self._is_alive.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// WebSocket message sender implementation
struct WebSocketMessageSender {
    sender: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

impl WebSocketMessageSender {
    fn new(sender: tokio::sync::mpsc::UnboundedSender<Vec<u8>>) -> Self {
        Self { sender }
    }
}

impl crate::impls::messaging::MessageSender for WebSocketMessageSender {
    fn send(&self, message: ServerMessage) -> Result<(), String> {
        // Encode to binary proto
        let bytes = ProtoCodec::encode_server_message(&message)?;

        // Send via channel
        self.sender
            .clone()
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
    // TODO: Extract JWT from query parameter or header before upgrade
    // For now, all WebSocket connections use anonymous user IDs
    // In production, clients should send JWT via query param: ws://host/room/123?token=xxx
    ws.on_upgrade(move |socket| handle_socket(socket, state, room_id))
}

async fn handle_socket(
    socket: axum::extract::ws::WebSocket,
    state: AppState,
    room_id: String,
) {
    // TODO: Extract user_id from JWT token passed via query parameter
    let user_id = UserId::new();
    // Try to get username from cache (will be None for anonymous users)
    let username = state
        .user_service
        .get_username(&user_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "anonymous".to_string());

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
            return;
        }
    };

    let rid = RoomId::from_string(room_id.clone());

    // Create rate limiter and content filter with default config
    let rate_limiter = Arc::new(RateLimiter::new(None, "ws".to_string()).unwrap());
    let rate_limit_config = Arc::new(RateLimitConfig::default());
    let content_filter = Arc::new(ContentFilter::new());

    // Create channel for sending messages to WebSocket
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let is_alive = Arc::new(std::sync::atomic::AtomicBool::new(true));

    // Create WebSocket sender - wrapped in Arc for sharing with handler
    let ws_sender_for_handler = Arc::new(WebSocketMessageSender::new(tx.clone()));
    let ws_sender = WebSocketMessageSender::new(tx);

    // Create StreamMessageHandler with all configuration
    let stream_handler = StreamMessageHandler::new(
        rid.clone(),
        user_id.clone(),
        username.clone(),
        state.room_service.clone(),
        cluster_manager,
        rate_limiter,
        rate_limit_config,
        content_filter,
        ws_sender_for_handler,
    );

    // Split WebSocket into sender and receiver
    let (mut ws_sender_sink, ws_receiver) = socket.split();

    // Spawn task to handle server messages -> WebSocket
    let is_alive_clone = is_alive.clone();
    tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if let Err(e) = ws_sender_sink
                .send(axum::extract::ws::Message::Binary(bytes))
                .await
            {
                error!("Failed to send WebSocket message: {}", e);
                is_alive_clone.store(false, std::sync::atomic::Ordering::Relaxed);
                break;
            }
        }
    });

    // Create WebSocketStream and run unified message loop
    let mut stream = WebSocketStream {
        receiver: ws_receiver,
        sender: ws_sender,
        _is_alive: is_alive,
    };

    // Run unified message loop - ALL logic is here!
    if let Err(e) = stream_handler.run(&mut stream).await {
        error!("Stream handler error: {}", e);
    }

    info!(
        "WebSocket connection closed: user={}, room={}",
        user_id.as_str(),
        room_id
    );
}
