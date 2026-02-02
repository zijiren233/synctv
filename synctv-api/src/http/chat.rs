//! Chat and Danmaku API endpoints
//!
//! HTTP endpoints for sending and receiving chat messages and danmaku.

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::http::{AppState, AppError, AppResult};
use synctv_core::models::{RoomId, SendDanmakuRequest};

/// Send chat message request
#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub content: String,
}

/// Send chat message response
#[derive(Debug, Serialize)]
pub struct SendMessageResponse {
    pub id: String,
    pub room_id: String,
    pub user_id: String,
    pub username: String,
    pub content: String,
    pub created_at: String,
}

/// Chat history query parameters
#[derive(Debug, Deserialize)]
pub struct ChatHistoryQuery {
    pub before: Option<String>, // ISO 8601 datetime
    pub limit: Option<i32>,
}

/// Chat history response
#[derive(Debug, Serialize)]
pub struct ChatHistoryResponse {
    pub messages: Vec<ChatMessageResponse>,
}

/// Chat message response
#[derive(Debug, Serialize)]
pub struct ChatMessageResponse {
    pub id: String,
    pub room_id: String,
    pub user_id: String,
    pub username: String,
    pub content: String,
    pub created_at: String,
}

/// Send danmaku response
#[derive(Debug, Serialize)]
pub struct SendDanmakuResponse {
    pub room_id: String,
    pub user_id: String,
    pub content: String,
    pub color: String,
    pub position: i32,
    pub timestamp: String,
}

/// Create chat-related routes
pub fn create_chat_router() -> Router<AppState> {
    Router::new()
        .route("/api/rooms/:room_id/chat", post(send_chat_message))
        .route("/api/rooms/:room_id/chat", get(get_chat_history))
        .route(
            "/api/rooms/:room_id/chat/:message_id",
            axum::routing::delete(delete_chat_message),
        )
        .route("/api/rooms/:room_id/danmaku", post(send_danmaku))
}

/// Send chat message
///
/// POST /api/rooms/:room_id/chat
/// Requires authentication
pub async fn send_chat_message(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> AppResult<Json<SendMessageResponse>> {
    use synctv_core::service::ChatService;

    // Get user ID from JWT (would normally come from middleware)
    // For now, we'll need to add authentication middleware
    let user_id = "user_id_from_jwt".to_string(); // TODO: Get from auth middleware

    let room_id = RoomId::from_string(room_id);
    let user_id = synctv_core::models::UserId::from_string(user_id);

    // Create chat service
    let chat_service = create_chat_service(&state);

    // Send message
    let message = chat_service
        .send_message(room_id.clone(), user_id.clone(), req.content)
        .await
        .map_err(|e| AppError::internal_server_error(&format!("Failed to send message: {}", e)))?;

    // Get username
    let username = state
        .user_service
        .get_username(&user_id)
        .await
        .map_err(|e| AppError::internal_server_error(&format!("Failed to get username: {}", e)))?
        .unwrap_or_else(|| "Unknown".to_string());

    Ok(Json(SendMessageResponse {
        id: message.id,
        room_id: room_id.as_str().to_string(),
        user_id: user_id.as_str().to_string(),
        username,
        content: message.content,
        created_at: message.created_at.to_rfc3339(),
    }))
}

/// Get chat history
///
/// GET /api/rooms/:room_id/chat
/// Requires authentication
pub async fn get_chat_history(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Query(query): Query<ChatHistoryQuery>,
) -> AppResult<Json<ChatHistoryResponse>> {
    use synctv_core::service::ChatService;

    let room_id = RoomId::from_string(room_id);
    let limit = query.limit.unwrap_or(50).min(100);
    let before = query.before.and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    // Create chat service
    let chat_service = create_chat_service(&state);

    // Get history
    let messages = chat_service
        .get_history(&room_id, before, limit)
        .await
        .map_err(|e| AppError::internal_server_error(&format!("Failed to get history: {}", e)))?;

    // Convert to response
    let mut response_messages = Vec::new();
    for msg in messages {
        let username = state
            .user_service
            .get_username(&msg.user_id)
            .await
            .map_err(|e| AppError::internal_server_error(&format!("Failed to get username: {}", e)))?
            .unwrap_or_else(|| "Unknown".to_string());

        response_messages.push(ChatMessageResponse {
            id: msg.id,
            room_id: msg.room_id.as_str().to_string(),
            user_id: msg.user_id.as_str().to_string(),
            username,
            content: msg.content,
            created_at: msg.created_at.to_rfc3339(),
        });
    }

    Ok(Json(ChatHistoryResponse {
        messages: response_messages,
    }))
}

/// Delete chat message
///
/// DELETE /api/rooms/:room_id/chat/:message_id
/// Requires authentication (user must be the sender)
pub async fn delete_chat_message(
    State(state): State<AppState>,
    Path((_room_id, message_id)): Path<(String, String)>,
) -> AppResult<impl IntoResponse> {
    use synctv_core::service::ChatService;

    // Get user ID from JWT (would normally come from middleware)
    let user_id = "user_id_from_jwt".to_string(); // TODO: Get from auth middleware
    let user_id = synctv_core::models::UserId::from_string(user_id);

    // Create chat service
    let chat_service = create_chat_service(&state);

    // Delete message
    chat_service
        .delete_message(&message_id, &user_id)
        .await
        .map_err(|e| AppError::internal_server_error(&format!("Failed to delete message: {}", e)))?;

    info!("Deleted chat message {}", message_id);

    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Send danmaku
///
/// POST /api/rooms/:room_id/danmaku
/// Requires authentication
/// Note: Danmaku are not persisted, they are real-time only
pub async fn send_danmaku(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(mut req): Json<SendDanmakuRequest>,
) -> AppResult<Json<SendDanmakuResponse>> {
    use synctv_core::service::ChatService;

    // Get user ID from JWT (would normally come from middleware)
    let user_id = "user_id_from_jwt".to_string(); // TODO: Get from auth middleware
    let user_id = synctv_core::models::UserId::from_string(user_id);

    let room_id = RoomId::from_string(room_id);
    req.room_id = room_id.clone();

    // Create chat service
    let chat_service = create_chat_service(&state);

    // Send danmaku
    let danmaku = chat_service
        .send_danmaku(room_id.clone(), user_id.clone(), req)
        .await
        .map_err(|e| AppError::internal_server_error(&format!("Failed to send danmaku: {}", e)))?;

    // Publish to WebSocket/message hub for real-time delivery
    // TODO: Add message publishing to message hub

    info!(
        room_id = room_id.as_str(),
        user_id = user_id.as_str(),
        "Danmaku sent"
    );

    Ok(Json(SendDanmakuResponse {
        room_id: danmaku.room_id.as_str().to_string(),
        user_id: danmaku.user_id.as_str().to_string(),
        content: danmaku.content,
        color: danmaku.color,
        position: danmaku.position as i32,
        timestamp: danmaku.timestamp.to_rfc3339(),
    }))
}

/// Helper to create chat service
fn create_chat_service(state: &AppState) -> synctv_core::service::ChatService {
    use synctv_core::repository::{ChatRepository, UserRepository};
    use std::sync::Arc;

    let pool = state.user_service.pool().clone();
    synctv_core::service::ChatService::new(
        Arc::new(ChatRepository::new(pool.clone())),
        Arc::new(UserRepository::new(pool)),
        synctv_core::service::RateLimiter::new(None, "synctv:".to_string()).unwrap(),
        synctv_core::service::ContentFilter::new(),
        state.user_service.username_cache().clone(),
    )
}
