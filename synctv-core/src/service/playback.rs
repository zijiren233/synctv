//! Playback state management service
//!
//! Handles playback coordination including play/pause, seeking, speed changes,
//! and media switching with optimistic locking for concurrent updates.

use crate::{
    models::{RoomId, UserId, MediaId, PermissionBits, RoomPlaybackState},
    repository::RoomPlaybackStateRepository,
    service::{permission::PermissionService, media::MediaService},
    Error, Result,
};

/// Playback management service
///
/// Responsible for playback state coordination and optimistic locking.
#[derive(Clone)]
pub struct PlaybackService {
    playback_repo: RoomPlaybackStateRepository,
    permission_service: PermissionService,
    media_service: MediaService,
}

impl std::fmt::Debug for PlaybackService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackService").finish()
    }
}

impl PlaybackService {
    /// Create a new playback service
    pub fn new(
        playback_repo: RoomPlaybackStateRepository,
        permission_service: PermissionService,
        media_service: MediaService,
    ) -> Self {
        Self {
            playback_repo,
            permission_service,
            media_service,
        }
    }

    /// Get playback state for a room
    pub async fn get_state(&self, room_id: &RoomId) -> Result<RoomPlaybackState> {
        self.playback_repo.create_or_get(room_id).await
    }

    /// Play/pause playback
    pub async fn set_playing(
        &self,
        room_id: RoomId,
        user_id: UserId,
        playing: bool,
    ) -> Result<RoomPlaybackState> {
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::PLAY_PAUSE)
            .await?;

        self.update_state(room_id, |state| {
            state.is_playing = playing;
            state.updated_at = chrono::Utc::now();
            state.version += 1;
        })
        .await
    }

    /// Seek to position
    pub async fn seek(
        &self,
        room_id: RoomId,
        user_id: UserId,
        position: f64,
    ) -> Result<RoomPlaybackState> {
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::SEEK)
            .await?;

        self.update_state(room_id, |state| {
            state.position = position;
            state.updated_at = chrono::Utc::now();
            state.version += 1;
        })
        .await
    }

    /// Change playback speed
    pub async fn change_speed(
        &self,
        room_id: RoomId,
        user_id: UserId,
        speed: f64,
    ) -> Result<RoomPlaybackState> {
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::CHANGE_SPEED)
            .await?;

        // Validate speed range
        if speed < 0.25 || speed > 4.0 {
            return Err(Error::InvalidInput("Speed must be between 0.25 and 4.0".to_string()));
        }

        self.update_state(room_id, |state| {
            state.speed = speed;
            state.updated_at = chrono::Utc::now();
            state.version += 1;
        })
        .await
    }

    /// Switch to different media in playlist
    pub async fn switch_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        media_id: MediaId,
    ) -> Result<RoomPlaybackState> {
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::SWITCH_MEDIA)
            .await?;

        // Verify media exists in this room
        let media = self
            .media_service
            .get_media(&media_id)
            .await?
            .ok_or_else(|| Error::NotFound("Media not found".to_string()))?;

        if media.room_id != room_id {
            return Err(Error::Authorization("Media does not belong to this room".to_string()));
        }

        self.update_state(room_id, |state| {
            state.current_media_id = Some(media_id);
            state.position = 0.0;
            state.is_playing = true;
            state.updated_at = chrono::Utc::now();
            state.version += 1;
        })
        .await
    }

    /// Update playback state with generic update function
    pub async fn update_state<F>(
        &self,
        room_id: RoomId,
        update_fn: F,
    ) -> Result<RoomPlaybackState>
    where
        F: FnOnce(&mut RoomPlaybackState),
    {
        // Get current state
        let mut state = self.playback_repo.create_or_get(&room_id).await?;

        // Apply update
        update_fn(&mut state);

        // Save with optimistic locking
        let updated_state = self.playback_repo.update(&state).await?;

        Ok(updated_state)
    }

    /// Reset playback to initial state
    pub async fn reset(&self, room_id: RoomId, user_id: UserId) -> Result<RoomPlaybackState> {
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::PLAY_PAUSE)
            .await?;

        self.update_state(room_id, |state| {
            state.is_playing = false;
            state.position = 0.0;
            state.speed = 1.0;
            state.current_media_id = None;
            state.updated_at = chrono::Utc::now();
            state.version += 1;
        })
        .await
    }

    /// Check if playback is currently active
    pub async fn is_playing(&self, room_id: &RoomId) -> Result<bool> {
        let state = self.get_state(room_id).await?;
        Ok(state.is_playing)
    }

    /// Get current media being played
    pub async fn get_current_media_id(&self, room_id: &RoomId) -> Result<Option<MediaId>> {
        let state = self.get_state(room_id).await?;
        Ok(state.current_media_id)
    }

    /// Get current playback position
    pub async fn get_position(&self, room_id: &RoomId) -> Result<f64> {
        let state = self.get_state(room_id).await?;
        Ok(state.position)
    }

    /// Get current playback speed
    pub async fn get_speed(&self, room_id: &RoomId) -> Result<f64> {
        let state = self.get_state(room_id).await?;
        Ok(state.speed)
    }

    /// Update multiple playback properties at once
    pub async fn update_multiple(
        &self,
        room_id: RoomId,
        user_id: UserId,
        playing: Option<bool>,
        position: Option<f64>,
        speed: Option<f64>,
        media_id: Option<MediaId>,
    ) -> Result<RoomPlaybackState> {
        // Check permissions based on what's being updated
        let mut required_perms = PermissionBits::NONE;
        if playing.is_some() {
            required_perms |= PermissionBits::PLAY_PAUSE;
        }
        if position.is_some() {
            required_perms |= PermissionBits::SEEK;
        }
        if speed.is_some() {
            required_perms |= PermissionBits::CHANGE_SPEED;
        }
        if media_id.is_some() {
            required_perms |= PermissionBits::SWITCH_MEDIA;
        }

        if required_perms != PermissionBits::NONE {
            self.permission_service
                .check_permission(&room_id, &user_id, required_perms)
                .await?;
        }

        // If media_id is provided, verify it exists
        if let Some(ref mid) = media_id {
            let media = self
                .media_service
                .get_media(mid)
                .await?
                .ok_or_else(|| Error::NotFound("Media not found".to_string()))?;

            if media.room_id != room_id {
                return Err(Error::Authorization("Media does not belong to this room".to_string()));
            }
        }

        self.update_state(room_id, |state| {
            if let Some(p) = playing {
                state.is_playing = p;
            }
            if let Some(pos) = position {
                state.position = pos;
            }
            if let Some(s) = speed {
                state.speed = s;
            }
            if let Some(mid) = media_id {
                state.current_media_id = Some(mid);
            }
            state.updated_at = chrono::Utc::now();
            state.version += 1;
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_play_pause() {
        // Integration test placeholder
    }

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_seek() {
        // Integration test placeholder
    }
}
