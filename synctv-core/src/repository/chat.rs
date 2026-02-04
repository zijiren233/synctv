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
            RETURNING id, room_id, user_id, content, created_at
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
                SELECT id, room_id, user_id, content, created_at
                FROM chat_messages
                WHERE room_id = $1 AND created_at < $2
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
                SELECT id, room_id, user_id, content, created_at
                FROM chat_messages
                WHERE room_id = $1
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
            SELECT id, room_id, user_id, content, created_at
            FROM chat_messages
            WHERE id = $1
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

    /// Delete a message (physical delete)
    pub async fn delete(&self, message_id: &str) -> Result<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM chat_messages
            WHERE id = $1
            ",
        )
        .bind(message_id)
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
            WHERE room_id = $1
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
                WHERE room_id = $1
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

    /// Delete old messages for all rooms in a single query (keep only last N messages per room)
    ///
    /// This is much more efficient than calling `cleanup_old_messages()` for each room individually.
    /// Uses window functions to identify messages to delete across all rooms.
    /// Only processes rooms with recent activity (messages within the last few minutes).
    ///
    /// # Arguments
    /// * `keep_count` - Maximum messages to keep per room (0 = unlimited, no cleanup)
    /// * `activity_window_minutes` - Only cleanup rooms with messages in the last N minutes
    ///
    /// # Returns
    /// Total number of messages deleted across all rooms
    pub async fn cleanup_all_rooms(&self, keep_count: i32, activity_window_minutes: i32) -> Result<u64> {
        // If keep_count is 0, no cleanup needed
        if keep_count <= 0 {
            return Ok(0);
        }

        let result = sqlx::query(
            r"
            DELETE FROM chat_messages
            WHERE id IN (
                SELECT id FROM (
                    SELECT id, room_id,
                           ROW_NUMBER() OVER (PARTITION BY room_id ORDER BY created_at DESC) as rn
                    FROM chat_messages
                    WHERE room_id IN (
                        SELECT DISTINCT room_id
                        FROM chat_messages
                        WHERE created_at >= NOW() - ($2 || ' minutes')::INTERVAL
                    )
                ) ranked_messages
                WHERE rn > $1
            )
            ",
        )
        .bind(keep_count)
        .bind(activity_window_minutes)
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
