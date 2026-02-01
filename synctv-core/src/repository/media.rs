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

    /// Batch insert media items
    pub async fn create_batch(&self, items: &[Media]) -> Result<Vec<Media>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::with_capacity(items.len());
        let mut tx = self.pool.begin().await?;

        for item in items {
            let metadata_json = serde_json::to_value(&item.metadata)?;

            let row = sqlx::query(
                "INSERT INTO media (id, room_id, url, provider, title, metadata, position, added_at, added_by)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 RETURNING id, room_id, url, provider, title, metadata, position, added_at, added_by, deleted_at"
            )
            .bind(item.id.as_str())
            .bind(item.room_id.as_str())
            .bind(&item.url)
            .bind(item.provider.as_str())
            .bind(&item.title)
            .bind(&metadata_json)
            .bind(item.position)
            .bind(item.added_at)
            .bind(item.added_by.as_str())
            .fetch_one(&mut *tx)
            .await?;

            results.push(self.row_to_media(row)?);
        }

        // Commit transaction
        tx.commit().await?;

        Ok(results)
    }

    /// Update media
    pub async fn update(&self, media: &Media) -> Result<Media> {
        let metadata_json = serde_json::to_value(&media.metadata)?;

        let row = sqlx::query(
            "UPDATE media
             SET url = $2, title = $3, metadata = $4
             WHERE id = $1 AND deleted_at IS NULL
             RETURNING id, room_id, url, provider, title, metadata, position, added_at, added_by, deleted_at"
        )
        .bind(media.id.as_str())
        .bind(&media.url)
        .bind(&media.title)
        .bind(&metadata_json)
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

    /// Get paginated playlist
    pub async fn get_playlist_paginated(
        &self,
        room_id: &RoomId,
        page: i32,
        page_size: i32,
    ) -> Result<(Vec<Media>, i64)> {
        let offset = (page - 1) * page_size;

        // Get total count
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM media WHERE room_id = $1 AND deleted_at IS NULL"
        )
        .bind(room_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        // Get paginated results
        let rows = sqlx::query(
            "SELECT id, room_id, url, provider, title, metadata, position, added_at, added_by, deleted_at
             FROM media
             WHERE room_id = $1 AND deleted_at IS NULL
             ORDER BY position ASC
             LIMIT $2 OFFSET $3"
        )
        .bind(room_id.as_str())
        .bind(page_size as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await?;

        let items: Vec<Media> = rows.into_iter().map(|row| self.row_to_media(row)).collect::<Result<Vec<Media>>>()?;

        Ok((items, total))
    }

    /// Delete media from playlist (soft delete)
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

    /// Delete all media in a room
    pub async fn delete_by_room(&self, room_id: &RoomId) -> Result<usize> {
        let result = sqlx::query(
            "UPDATE media
             SET deleted_at = $2
             WHERE room_id = $1 AND deleted_at IS NULL"
        )
        .bind(room_id.as_str())
        .bind(chrono::Utc::now())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() as usize)
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

    /// Move media to first position (for setting current media)
    pub async fn move_to_first(&self, media_id: &MediaId) -> Result<()> {
        // Get current position
        let current_pos: i32 = sqlx::query_scalar("SELECT position FROM media WHERE id = $1")
            .bind(media_id.as_str())
            .fetch_one(&self.pool)
            .await?;

        // Shift all items before this one down
        sqlx::query(
            "UPDATE media
             SET position = position + 1
             WHERE id != $1 AND position < $2 AND deleted_at IS NULL"
        )
        .bind(media_id.as_str())
        .bind(current_pos)
        .execute(&self.pool)
        .await?;

        // Set this item to position 0
        sqlx::query("UPDATE media SET position = 0 WHERE id = $1")
            .bind(media_id.as_str())
            .execute(&self.pool)
            .await?;

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

    /// Count media items in a room
    pub async fn count_by_room(&self, room_id: &RoomId) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM media WHERE room_id = $1 AND deleted_at IS NULL"
        )
        .bind(room_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
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
