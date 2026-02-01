use serde_json::Value as JsonValue;
use sqlx::{postgres::PgRow, PgPool, Row};

use crate::{
    models::{Media, MediaId, ProviderType, RoomId, UserId},
    Result,
};

/// Media repository for database operations
#[derive(Clone)]
pub struct MediaRepository {
    pool: PgPool,
}

impl MediaRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Add media to playlist
    pub async fn create(&self, media: &Media) -> Result<Media> {
        let metadata_json = serde_json::to_value(&media.metadata)?;

        let row = sqlx::query(
            "INSERT INTO media (id, room_id, url, provider, title, metadata, position, added_at, added_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING id, room_id, url, provider, title, metadata, position, added_at, added_by, deleted_at"
        )
        .bind(media.id.as_str())
        .bind(media.room_id.as_str())
        .bind(&media.url)
        .bind(media.provider.as_str())
        .bind(&media.title)
        .bind(&metadata_json)
        .bind(media.position)
        .bind(media.added_at)
        .bind(media.added_by.as_str())
        .fetch_one(&self.pool)
        .await?;

        self.row_to_media(row)
    }

    /// Get media by ID
    pub async fn get_by_id(&self, media_id: &MediaId) -> Result<Option<Media>> {
        let row = sqlx::query(
            "SELECT id, room_id, url, provider, title, metadata, position, added_at, added_by, deleted_at
             FROM media
             WHERE id = $1 AND deleted_at IS NULL"
        )
        .bind(media_id.as_str())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_media(row)?)),
            None => Ok(None),
        }
    }

    /// Get playlist for a room
    pub async fn get_playlist(&self, room_id: &RoomId) -> Result<Vec<Media>> {
        let rows = sqlx::query(
            "SELECT id, room_id, url, provider, title, metadata, position, added_at, added_by, deleted_at
             FROM media
             WHERE room_id = $1 AND deleted_at IS NULL
             ORDER BY position ASC"
        )
        .bind(room_id.as_str())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|row| self.row_to_media(row)).collect()
    }

    /// Delete media from playlist
    pub async fn delete(&self, media_id: &MediaId) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE media
             SET deleted_at = $2
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(media_id.as_str())
        .bind(chrono::Utc::now())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Swap positions of two media
    pub async fn swap_positions(&self, media_id1: &MediaId, media_id2: &MediaId) -> Result<()> {
        // Get current positions
        let pos1: i32 = sqlx::query_scalar("SELECT position FROM media WHERE id = $1")
            .bind(media_id1.as_str())
            .fetch_one(&self.pool)
            .await?;

        let pos2: i32 = sqlx::query_scalar("SELECT position FROM media WHERE id = $1")
            .bind(media_id2.as_str())
            .fetch_one(&self.pool)
            .await?;

        // Swap positions in a transaction
        let mut tx = self.pool.begin().await?;

        sqlx::query("UPDATE media SET position = $2 WHERE id = $1")
            .bind(media_id1.as_str())
            .bind(pos2)
            .execute(&mut *tx)
            .await?;

        sqlx::query("UPDATE media SET position = $2 WHERE id = $1")
            .bind(media_id2.as_str())
            .bind(pos1)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(())
    }

    /// Get next available position in playlist
    pub async fn get_next_position(&self, room_id: &RoomId) -> Result<i32> {
        let max_pos: Option<i32> = sqlx::query_scalar(
            "SELECT MAX(position) FROM media WHERE room_id = $1 AND deleted_at IS NULL",
        )
        .bind(room_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        Ok(max_pos.unwrap_or(-1) + 1)
    }

    /// Convert database row to Media
    fn row_to_media(&self, row: PgRow) -> Result<Media> {
        let metadata_json: JsonValue = row.try_get("metadata")?;
        let provider_str: String = row.try_get("provider")?;

        Ok(Media {
            id: MediaId::from_string(row.try_get("id")?),
            room_id: RoomId::from_string(row.try_get("room_id")?),
            url: row.try_get("url")?,
            provider: ProviderType::from_str(&provider_str).unwrap_or(ProviderType::DirectUrl),
            title: row.try_get("title")?,
            metadata: metadata_json,
            position: row.try_get("position")?,
            added_at: row.try_get("added_at")?,
            added_by: UserId::from_string(row.try_get("added_by")?),
            deleted_at: row.try_get("deleted_at")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_create_media() {
        // Integration test placeholder
    }
}
