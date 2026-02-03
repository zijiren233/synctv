use sqlx::{postgres::PgRow, PgPool, Row};

use crate::{
    models::{MediaId, RoomId, RoomPlaybackState},
    Error, Result,
};

/// Room playback state repository
#[derive(Clone)]
pub struct RoomPlaybackStateRepository {
    pool: PgPool,
}

impl RoomPlaybackStateRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create or get playback state for room
    pub async fn create_or_get(&self, room_id: &RoomId) -> Result<RoomPlaybackState> {
        // Try to get existing state
        if let Some(state) = self.get(room_id).await? {
            return Ok(state);
        }

        // Create new state
        let state = RoomPlaybackState::new(room_id.clone());

        let row = sqlx::query(
            "INSERT INTO room_playback_state (room_id, position, speed, is_playing, updated_at, version)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (room_id) DO NOTHING
             RETURNING room_id, playing_media_id, position, speed, is_playing, updated_at, version"
        )
        .bind(room_id.as_str())
        .bind(state.position)
        .bind(state.speed)
        .bind(state.is_playing)
        .bind(state.updated_at)
        .bind(state.version)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => self.row_to_state(row),
            None => self
                .get(room_id)
                .await?
                .ok_or_else(|| Error::Internal("Failed to create playback state".to_string())),
        }
    }

    /// Get playback state
    pub async fn get(&self, room_id: &RoomId) -> Result<Option<RoomPlaybackState>> {
        let row = sqlx::query(
            "SELECT room_id, playing_media_id, position, speed, is_playing, updated_at, version
             FROM room_playback_state
             WHERE room_id = $1",
        )
        .bind(room_id.as_str())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_state(row)?)),
            None => Ok(None),
        }
    }

    /// Update playback state with optimistic locking
    pub async fn update(&self, state: &RoomPlaybackState) -> Result<RoomPlaybackState> {
        let movie_id_str = state.playing_media_id.as_ref().map(|id| id.as_str());

        let row = sqlx::query(
            "UPDATE room_playback_state
             SET playing_media_id = $2, position = $3, speed = $4, is_playing = $5,
                 updated_at = $6, version = version + 1
             WHERE room_id = $1 AND version = $7
             RETURNING room_id, playing_media_id, position, speed, is_playing, updated_at, version",
        )
        .bind(state.room_id.as_str())
        .bind(movie_id_str)
        .bind(state.position)
        .bind(state.speed)
        .bind(state.is_playing)
        .bind(chrono::Utc::now())
        .bind(state.version)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => self.row_to_state(row),
            None => Err(Error::Internal(
                "Playback state was modified by another request (optimistic lock failure)"
                    .to_string(),
            )),
        }
    }

    /// Convert database row to RoomPlaybackState
    fn row_to_state(&self, row: PgRow) -> Result<RoomPlaybackState> {
        let movie_id_opt: Option<String> = row.try_get("playing_media_id")?;

        Ok(RoomPlaybackState {
            room_id: RoomId::from_string(row.try_get("room_id")?),
            playing_media_id: movie_id_opt.map(MediaId::from_string),
            position: row.try_get("position")?,
            speed: row.try_get("speed")?,
            is_playing: row.try_get("is_playing")?,
            updated_at: row.try_get("updated_at")?,
            version: row.try_get("version")?,
        })
    }
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_create_or_get() {
        // Integration test placeholder
    }

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_optimistic_locking() {
        // Test that concurrent updates fail with version mismatch
    }
}
