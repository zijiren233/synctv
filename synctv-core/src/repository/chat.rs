use chrono::{DateTime, Utc};
use sqlx::{postgres::PgRow, PgPool, Row};

use crate::{
    models::{ChatMessage, RoomId, UserId},
    Result,
};

/// Chat message repository for database operations
#[derive(Clone)]
pub struct ChatRepository {
    pool: PgPool,
}

impl ChatRepository {
    #[must_use] 
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new chat message
    pub async fn create(&self, message: &ChatMessage) -> Result<ChatMessage> {
        let row = sqlx::query(
            r"
            INSERT INTO chat_messages (id, room_id, user_id, content, created_at)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, room_id, user_id, content, created_at, deleted_at
            ",
        )
        .bind(&message.id)
        .bind(message.room_id.as_str())
        .bind(message.user_id.as_str())
        .bind(&message.content)
        .bind(message.created_at)
        .fetch_one(&self.pool)
        .await?;

        self.row_to_message(row)
    }

    /// Get chat history for a room
    /// Returns messages in reverse chronological order (newest first)
    pub async fn list_by_room(
        &self,
        room_id: &RoomId,
        before: Option<DateTime<Utc>>,
        limit: i32,
    ) -> Result<Vec<ChatMessage>> {
        let limit = limit.min(100); // Cap at 100 messages per request

        let rows = if let Some(before_time) = before {
            sqlx::query(
                r"
                SELECT id, room_id, user_id, content, created_at, deleted_at
                FROM chat_messages
                WHERE room_id = $1 AND created_at < $2 AND deleted_at IS NULL
                ORDER BY created_at DESC
                LIMIT $3
                ",
            )
            .bind(room_id.as_str())
            .bind(before_time)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r"
                SELECT id, room_id, user_id, content, created_at, deleted_at
                FROM chat_messages
                WHERE room_id = $1 AND deleted_at IS NULL
                ORDER BY created_at DESC
                LIMIT $2
                ",
            )
            .bind(room_id.as_str())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter()
            .map(|row| self.row_to_message(row))
            .collect()
    }

    /// Get a specific message by ID
    pub async fn get_by_id(&self, message_id: &str) -> Result<Option<ChatMessage>> {
        let row = sqlx::query(
            r"
            SELECT id, room_id, user_id, content, created_at, deleted_at
            FROM chat_messages
            WHERE id = $1 AND deleted_at IS NULL
            ",
        )
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_message(row)?)),
            None => Ok(None),
        }
    }

    /// Soft delete a message
    pub async fn delete(&self, message_id: &str) -> Result<bool> {
        let result = sqlx::query(
            r"
            UPDATE chat_messages
            SET deleted_at = $2
            WHERE id = $1 AND deleted_at IS NULL
            ",
        )
        .bind(message_id)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Get message count for a room
    pub async fn count_by_room(&self, room_id: &RoomId) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*) as count
            FROM chat_messages
            WHERE room_id = $1 AND deleted_at IS NULL
            ",
        )
        .bind(room_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    /// Delete old messages for a room (keep only last N messages)
    pub async fn cleanup_old_messages(&self, room_id: &RoomId, keep_count: i32) -> Result<u64> {
        let result = sqlx::query(
            r"
            DELETE FROM chat_messages
            WHERE room_id = $1
            AND id NOT IN (
                SELECT id FROM chat_messages
                WHERE room_id = $1 AND deleted_at IS NULL
                ORDER BY created_at DESC
                LIMIT $2
            )
            ",
        )
        .bind(room_id.as_str())
        .bind(keep_count)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Convert database row to `ChatMessage`
    fn row_to_message(&self, row: PgRow) -> Result<ChatMessage> {
        Ok(ChatMessage {
            id: row.try_get("id")?,
            room_id: RoomId::from_string(row.try_get("room_id")?),
            user_id: UserId::from_string(row.try_get("user_id")?),
            content: row.try_get("content")?,
            created_at: row.try_get("created_at")?,
            deleted_at: row.try_get("deleted_at")?,
        })
    }
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_create_message() {
        // Integration test placeholder
    }
}
