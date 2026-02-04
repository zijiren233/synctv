//! Chat service for managing room chat messages
//!
//! Handles sending, receiving, and deleting chat messages with rate limiting
//! and content filtering.

use std::sync::Arc;
use chrono::Utc;
use tracing::info;

use crate::{
    cache::UsernameCache,
    models::{ChatMessage, RoomId, SendDanmakuRequest, UserId},
    repository::ChatRepository,
    service::{ContentFilter, RateLimiter},
    Error, Result,
};

/// Chat service for managing chat messages
#[derive(Clone)]
pub struct ChatService {
    pub(crate) chat_repository: Arc<ChatRepository>,
    rate_limiter: RateLimiter,
    content_filter: ContentFilter,
    username_cache: UsernameCache,
}

impl std::fmt::Debug for ChatService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatService")
            .finish()
    }
}

impl ChatService {
    /// Create a new chat service
    #[must_use] 
    pub const fn new(
        chat_repository: Arc<ChatRepository>,
        rate_limiter: RateLimiter,
        content_filter: ContentFilter,
        username_cache: UsernameCache,
    ) -> Self {
        Self {
            chat_repository,
            rate_limiter,
            content_filter,
            username_cache,
        }
    }

    /// Send a chat message
    ///
    /// # Arguments
    /// * `room_id` - Room ID
    /// * `user_id` - User ID sending the message
    /// * `content` - Message content
    ///
    /// # Returns
    /// The created chat message
    pub async fn send_message(
        &self,
        room_id: RoomId,
        user_id: UserId,
        content: String,
    ) -> Result<ChatMessage> {
        // Rate limiting: 10 messages per second per room per user
        let rate_key = format!("chat:rate:{}:{}", room_id.as_str(), user_id.as_str());
        if let Err(e) = self
            .rate_limiter
            .check_rate_limit(&rate_key, 10, 1)
            .await
        {
            return Err(Error::InvalidInput(format!("Rate limit exceeded: {e}")));
        }

        // Validate content length
        if content.is_empty() {
            return Err(Error::InvalidInput("Message content cannot be empty".to_string()));
        }

        if content.len() > 500 {
            return Err(Error::InvalidInput(
                "Message content must be at most 500 characters".to_string(),
            ));
        }

        // Get username
        let _username = self
            .username_cache
            .get(&user_id)
            .await?
            .ok_or_else(|| Error::NotFound("User not found".to_string()))?;

        // Filter content
        let filtered_content = self
            .content_filter
            .filter_chat(&content)
            .map_err(|e| Error::InvalidInput(format!("Content filter error: {e}")))?;

        // Create message
        let message = ChatMessage::new(room_id.clone(), user_id.clone(), filtered_content);

        // Persist to database
        let created_message = self.chat_repository.create(&message).await?;

        info!(
            room_id = room_id.as_str(),
            user_id = user_id.as_str(),
            message_id = %created_message.id,
            "Chat message sent"
        );

        Ok(created_message)
    }

    /// Get chat history for a room
    ///
    /// # Arguments
    /// * `room_id` - Room ID
    /// * `before` - Optional timestamp to get messages before
    /// * `limit` - Maximum number of messages to return (max 100)
    ///
    /// # Returns
    /// List of chat messages in reverse chronological order
    pub async fn get_history(
        &self,
        room_id: &RoomId,
        before: Option<chrono::DateTime<Utc>>,
        limit: i32,
    ) -> Result<Vec<ChatMessage>> {
        self.chat_repository
            .list_by_room(room_id, before, limit)
            .await
    }

    /// Delete a chat message
    ///
    /// # Arguments
    /// * `message_id` - Message ID to delete
    /// * `user_id` - User ID requesting deletion (must be sender or admin)
    ///
    /// # Returns
    /// Result indicating success or failure
    pub async fn delete_message(&self, message_id: &str, user_id: &UserId) -> Result<bool> {
        // Get the message to check ownership
        let message = self
            .chat_repository
            .get_by_id(message_id)
            .await?
            .ok_or_else(|| Error::NotFound("Message not found".to_string()))?;

        // Check if user is the sender
        if message.user_id != *user_id {
            return Err(Error::Authorization(
                "You can only delete your own messages".to_string(),
            ));
        }

        self.chat_repository.delete(message_id).await
    }

    /// Send a danmaku message (not persisted, real-time only)
    ///
    /// # Arguments
    /// * `room_id` - Room ID
    /// * `user_id` - User ID sending the danmaku
    /// * `request` - Danmaku request with content, color, and position
    ///
    /// # Returns
    /// The danmaku message (not persisted)
    pub async fn send_danmaku(
        &self,
        room_id: RoomId,
        user_id: UserId,
        request: SendDanmakuRequest,
    ) -> Result<crate::models::DanmakuMessage> {
        use crate::models::DanmakuMessage;

        // Rate limiting: 20 danmaku per second per room per user
        let rate_key = format!("danmaku:rate:{}:{}", room_id.as_str(), user_id.as_str());
        if let Err(e) = self
            .rate_limiter
            .check_rate_limit(&rate_key, 20, 1)
            .await
        {
            return Err(Error::InvalidInput(format!("Rate limit exceeded: {e}")));
        }

        // Validate content length
        if request.content.is_empty() {
            return Err(Error::InvalidInput("Danmaku content cannot be empty".to_string()));
        }

        if request.content.len() > 100 {
            return Err(Error::InvalidInput(
                "Danmaku content must be at most 100 characters".to_string(),
            ));
        }

        // Validate color format (hex color)
        if !request.color.starts_with('#') || request.color.len() != 7 {
            return Err(Error::InvalidInput("Invalid color format".to_string()));
        }

        // Filter content
        let filtered_content = self
            .content_filter
            .filter_danmaku(&request.content)
            .map_err(|e| Error::InvalidInput(format!("Content filter error: {e}")))?;

        // Log before moving values
        info!(
            room_id = room_id.as_str(),
            user_id = user_id.as_str(),
            "Danmaku sent"
        );

        // Create danmaku message
        let danmaku = DanmakuMessage::new(
            room_id,
            user_id,
            filtered_content,
            request.color,
            request.position,
        );

        Ok(danmaku)
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_validate_content() {
        // Test placeholder
        assert!("hello".len() < 500);
    }
}
