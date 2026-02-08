//! Media repository for database operations
//!
//! Design reference: /Volumes/workspace/rust/design/04-数据库设计.md §2.4.2

use sqlx::{postgres::PgRow, PgPool, Row};

use crate::{
    models::{Media, MediaId, PlaylistId, RoomId},
    Result,
};

/// Media repository for database operations
#[derive(Clone)]
pub struct MediaRepository {
    pool: PgPool,
}

impl MediaRepository {
    #[must_use] 
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Add media to playlist
    pub async fn create(&self, media: &Media) -> Result<Media> {
        let source_config_json = serde_json::to_value(&media.source_config)?;

        let row = sqlx::query(
            r"
            INSERT INTO media (id, playlist_id, room_id, creator_id, name, position,
                              source_provider, source_config, provider_instance_name, added_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             RETURNING id, playlist_id, room_id, creator_id, name, position,
                       source_provider, source_config, provider_instance_name,
                       added_at, deleted_at
            "
        )
        .bind(media.id.as_str())
        .bind(media.playlist_id.as_str())
        .bind(media.room_id.as_str())
        .bind(media.creator_id.as_str())
        .bind(&media.name)
        .bind(media.position)
        .bind(media.source_provider.as_str())
        .bind(&source_config_json)
        .bind(&media.provider_instance_name)
        .bind(media.added_at)
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
            let source_config_json = serde_json::to_value(&item.source_config)?;

            let row = sqlx::query(
                r"
                INSERT INTO media (id, playlist_id, room_id, creator_id, name, position,
                                  source_provider, source_config, provider_instance_name, added_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 RETURNING id, playlist_id, room_id, creator_id, name, position,
                           source_provider, source_config, provider_instance_name,
                           added_at, deleted_at
                "
            )
            .bind(item.id.as_str())
            .bind(item.playlist_id.as_str())
            .bind(item.room_id.as_str())
            .bind(item.creator_id.as_str())
            .bind(&item.name)
            .bind(item.position)
            .bind(item.source_provider.as_str())
            .bind(&source_config_json)
            .bind(&item.provider_instance_name)
            .bind(item.added_at)
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
        let source_config_json = serde_json::to_value(&media.source_config)?;

        let row = sqlx::query(
            r"
            UPDATE media
            SET name = $2, position = $3, source_config = $4,
                provider_instance_name = $5
             WHERE id = $1 AND deleted_at IS NULL
             RETURNING id, playlist_id, room_id, creator_id, name, position,
                       source_provider, source_config, provider_instance_name,
                       added_at, deleted_at
            "
        )
        .bind(media.id.as_str())
        .bind(&media.name)
        .bind(media.position)
        .bind(&source_config_json)
        .bind(&media.provider_instance_name)
        .fetch_one(&self.pool)
        .await?;

        self.row_to_media(row)
    }

    /// Get media by ID
    pub async fn get_by_id(&self, media_id: &MediaId) -> Result<Option<Media>> {
        let row = sqlx::query(
            r"
            SELECT id, playlist_id, room_id, creator_id, name, position,
                   source_provider, source_config, provider_instance_name,
                   added_at, deleted_at
             FROM media
             WHERE id = $1 AND deleted_at IS NULL
            "
        )
        .bind(media_id.as_str())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_media(row)?)),
            None => Ok(None),
        }
    }

    /// Get playlist for a room (all media in room's root playlist and sub-playlists)
    pub async fn get_playlist(&self, room_id: &RoomId) -> Result<Vec<Media>> {
        let rows = sqlx::query(
            r"
            SELECT id, playlist_id, room_id, creator_id, name, position,
                   source_provider, source_config, provider_instance_name,
                   added_at, deleted_at
             FROM media
             WHERE room_id = $1 AND deleted_at IS NULL
             ORDER BY playlist_id, position ASC
            "
        )
        .bind(room_id.as_str())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|row| self.row_to_media(row)).collect()
    }

    /// Get media in a specific playlist
    pub async fn get_by_playlist(&self, playlist_id: &PlaylistId) -> Result<Vec<Media>> {
        let rows = sqlx::query(
            r"
            SELECT id, playlist_id, room_id, creator_id, name, position,
                   source_provider, source_config, provider_instance_name,
                   added_at, deleted_at
             FROM media
             WHERE playlist_id = $1 AND deleted_at IS NULL
             ORDER BY position ASC
            "
        )
        .bind(playlist_id.as_str())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|row| self.row_to_media(row)).collect()
    }

    /// Get paginated playlist
    pub async fn get_playlist_paginated(
        &self,
        playlist_id: &PlaylistId,
        page: i32,
        page_size: i32,
    ) -> Result<(Vec<Media>, i64)> {
        let offset = (page - 1) * page_size;

        // Get total count
        let total: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*) FROM media WHERE playlist_id = $1 AND deleted_at IS NULL
            "
        )
        .bind(playlist_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        // Get paginated results
        let rows = sqlx::query(
            r"
            SELECT id, playlist_id, room_id, creator_id, name, position,
                   source_provider, source_config, provider_instance_name,
                   added_at, deleted_at
             FROM media
             WHERE playlist_id = $1 AND deleted_at IS NULL
             ORDER BY position ASC
             LIMIT $2 OFFSET $3
            "
        )
        .bind(playlist_id.as_str())
        .bind(i64::from(page_size))
        .bind(i64::from(offset))
        .fetch_all(&self.pool)
        .await?;

        let items: Vec<Media> = rows.into_iter().map(|row| self.row_to_media(row)).collect::<Result<Vec<Media>>>()?;

        Ok((items, total))
    }

    /// Delete media from playlist (soft delete)
    pub async fn delete(&self, media_id: &MediaId) -> Result<bool> {
        let result = sqlx::query(
            r"
            UPDATE media
             SET deleted_at = $2
             WHERE id = $1 AND deleted_at IS NULL
            "
        )
        .bind(media_id.as_str())
        .bind(chrono::Utc::now())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete all media in a playlist
    pub async fn delete_by_playlist(&self, playlist_id: &PlaylistId) -> Result<usize> {
        let result = sqlx::query(
            r"
            UPDATE media
             SET deleted_at = $2
             WHERE playlist_id = $1 AND deleted_at IS NULL
            "
        )
        .bind(playlist_id.as_str())
        .bind(chrono::Utc::now())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() as usize)
    }

    /// Bulk delete media items by IDs
    pub async fn delete_batch(&self, media_ids: &[MediaId]) -> Result<usize> {
        if media_ids.is_empty() {
            return Ok(0);
        }

        let id_strs: Vec<&str> = media_ids.iter().map(|id| id.as_str()).collect();
        let now = chrono::Utc::now();

        // Build query with ANY array parameter
        let result = sqlx::query(
            r"
            UPDATE media
             SET deleted_at = $2
             WHERE id = ANY($1) AND deleted_at IS NULL
            "
        )
        .bind(&id_strs)
        .bind(now)
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

    /// Bulk reorder media items with new positions
    /// Takes a list of (media_id, new_position) tuples and updates them in a transaction
    pub async fn reorder_batch(&self, updates: &[(MediaId, i32)]) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;

        for (media_id, new_position) in updates {
            sqlx::query("UPDATE media SET position = $2 WHERE id = $1 AND deleted_at IS NULL")
                .bind(media_id.as_str())
                .bind(new_position)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;

        Ok(())
    }

    /// Get next available position in playlist
    pub async fn get_next_position(&self, playlist_id: &PlaylistId) -> Result<i32> {
        let max_pos: Option<i32> = sqlx::query_scalar(
            r"
            SELECT MAX(position) FROM media WHERE playlist_id = $1 AND deleted_at IS NULL
            "
        )
        .bind(playlist_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        Ok(max_pos.unwrap_or(-1) + 1)
    }

    /// Count media items in a playlist
    pub async fn count_by_playlist(&self, playlist_id: &PlaylistId) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*) FROM media WHERE playlist_id = $1 AND deleted_at IS NULL
            "
        )
        .bind(playlist_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    /// Convert database row to Media
    fn row_to_media(&self, row: PgRow) -> Result<Media> {
        Ok(Media {
            id: MediaId::from_string(row.try_get("id")?),
            playlist_id: PlaylistId::from_string(row.try_get("playlist_id")?),
            room_id: RoomId::from_string(row.try_get("room_id")?),
            creator_id: crate::models::UserId::from_string(row.try_get("creator_id")?),
            name: row.try_get("name")?,
            position: row.try_get("position")?,
            source_provider: row.try_get("source_provider")?,
            source_config: row.try_get("source_config")?,
            provider_instance_name: row.try_get("provider_instance_name")?,
            added_at: row.try_get("added_at")?,
            deleted_at: row.try_get("deleted_at")?,
        })
    }
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_create_media() {
        // Integration test placeholder
    }
}
