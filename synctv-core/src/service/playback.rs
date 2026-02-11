//! Playback state management service
//!
//! Handles playback coordination including play/pause, seeking, speed changes,
//! and media switching with optimistic locking for concurrent updates.

use crate::{
    models::{RoomId, UserId, MediaId, PermissionBits, RoomPlaybackState, RoomSettings, PlayMode},
    repository::{RoomPlaybackStateRepository, MediaRepository},
    service::{permission::PermissionService, media::MediaService},
    Error, Result,
};
use rand::seq::IteratorRandom;

/// Playback management service
///
/// Responsible for playback state coordination and optimistic locking.
#[derive(Clone)]
pub struct PlaybackService {
    playback_repo: RoomPlaybackStateRepository,
    permission_service: PermissionService,
    media_service: MediaService,
    media_repo: MediaRepository,
}

impl std::fmt::Debug for PlaybackService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackService").finish()
    }
}

impl PlaybackService {
    /// Create a new playback service
    #[must_use] 
    pub const fn new(
        playback_repo: RoomPlaybackStateRepository,
        permission_service: PermissionService,
        media_service: MediaService,
        media_repo: MediaRepository,
    ) -> Self {
        Self {
            playback_repo,
            permission_service,
            media_service,
            media_repo,
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
            // version is incremented by the SQL UPDATE, not here
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
            // version is incremented by the SQL UPDATE, not here
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
        if !(0.25..=4.0).contains(&speed) {
            return Err(Error::InvalidInput("Speed must be between 0.25 and 4.0".to_string()));
        }

        self.update_state(room_id, |state| {
            state.speed = speed;
            state.updated_at = chrono::Utc::now();
            // version is incremented by the SQL UPDATE, not here
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
            state.playing_media_id = Some(media_id);
            state.position = 0.0;
            state.is_playing = true;
            state.updated_at = chrono::Utc::now();
            // version is incremented by the SQL UPDATE, not here
        })
        .await
    }

    /// Play next media in playlist (auto-play next episode)
    ///
    /// This is called when current media finishes playing.
    /// Returns the new playback state if successful, or None if there's no next media.
    pub async fn play_next(
        &self,
        room_id: &RoomId,
        settings: &RoomSettings,
    ) -> Result<Option<RoomPlaybackState>> {
        // Use new auto_play settings, falling back to legacy fields for compatibility
        let (enabled, mode) = if settings.auto_play.value.enabled || settings.auto_play_next.0 {
            let mode = settings.auto_play.value.mode;
            let enabled = settings.auto_play.value.enabled || settings.auto_play_next.0;

            // If legacy fields suggest a different mode than the new setting, use legacy
            let mode = if settings.loop_playlist.0 {
                PlayMode::RepeatAll
            } else if settings.shuffle_playlist.0 {
                PlayMode::Shuffle
            } else {
                mode
            };

            (enabled, mode)
        } else {
            (false, PlayMode::Sequential)
        };

        if !enabled {
            return Ok(None);
        }

        // Get current state
        let state = self.get_state(room_id).await?;

        // Get playlist
        let playlist = self.media_repo.get_playlist(room_id).await?;

        if playlist.is_empty() {
            return Ok(None);
        }

        // Handle different play modes
        let next_media = match mode {
            PlayMode::Sequential => {
                // Find next media by position
                let current_pos = if let Some(ref current_id) = state.playing_media_id {
                    playlist.iter()
                        .position(|m| &m.id == current_id)
                        .unwrap_or(0)
                } else {
                    0
                };

                if current_pos + 1 < playlist.len() {
                    Some(&playlist[current_pos + 1])
                } else {
                    None // End of playlist
                }
            }

            PlayMode::RepeatOne => {
                // Repeat current media
                if let Some(ref current_id) = state.playing_media_id {
                    playlist.iter()
                        .find(|m| &m.id == current_id)
                } else {
                    playlist.first()
                }
            }

            PlayMode::RepeatAll => {
                // Loop back to start
                let current_pos = if let Some(ref current_id) = state.playing_media_id {
                    playlist.iter()
                        .position(|m| &m.id == current_id)
                        .unwrap_or(0)
                } else {
                    0
                };

                let next_pos = (current_pos + 1) % playlist.len();
                Some(&playlist[next_pos])
            }

            PlayMode::Shuffle => {
                // Random next media (excluding current)
                //
                // NOTE: This is a simplified shuffle implementation that randomly selects
                // the next media from the playlist (excluding the current one).
                //
                // Pros: Simple, efficient, no additional state storage required
                // Cons: May play some media more frequently than others
                //
                // For a production-grade shuffle without repeats, consider implementing
                // Fisher-Yates shuffle algorithm with persistent state storage (Redis):
                // 1. Shuffle the entire playlist once
                // 2. Play through shuffled order
                // 3. Re-shuffle when all items played
                // See: /Volumes/workspace/rust/design/13-自动连播设计.md §3.4
                if let Some(ref current_id) = state.playing_media_id {
                    playlist.iter()
                        .filter(|m| &m.id != current_id)
                        .choose(&mut rand::thread_rng())
                } else {
                    playlist.first()
                }
            }
        };

        // Switch to next media
        if let Some(next) = next_media {
            let new_state = self.update_state(room_id.clone(), |state| {
                state.playing_media_id = Some(next.id.clone());
                state.position = 0.0;
                state.is_playing = true;
                state.updated_at = chrono::Utc::now();
                // version is incremented by the SQL UPDATE, not here
            }).await?;

            tracing::info!(
                room_id = %room_id.as_str(),
                media_id = %next.id.as_str(),
                name = %next.name,
                mode = ?mode,
                "Auto-played next media"
            );

            Ok(Some(new_state))
        } else {
            tracing::info!(
                room_id = %room_id.as_str(),
                mode = ?mode,
                "Playlist ended"
            );
            Ok(None)
        }
    }

    /// Check if media has ended and auto-play next if needed
    ///
    /// This should be called when playback position is updated.
    /// It checks if the current position has reached or exceeded the media duration.
    pub async fn check_and_auto_play(
        &self,
        room_id: &RoomId,
        settings: &RoomSettings,
        current_position: f64,
    ) -> Result<Option<RoomPlaybackState>> {
        // Use new auto_play settings with legacy fallback
        let enabled = settings.auto_play.value.enabled || settings.auto_play_next.0;

        if !enabled {
            return Ok(None);
        }

        // Get current media to check duration
        let state = self.get_state(room_id).await?;
        let playing_media_id = state.playing_media_id;

        let playing_media = match playing_media_id {
            Some(ref id) => self.media_service.get_media(id).await?.ok_or_else(|| {
                Error::NotFound("Current media not found".to_string())
            })?,
            None => return Ok(None),
        };

        // Check if media has metadata with duration
        // For direct URLs, get duration from PlaybackResult metadata
        // For provider-based media, duration check is skipped (client should handle)
        let duration = if playing_media.is_direct() {
            if let Some(playback_result) = playing_media.get_playback_result() {
                playback_result.metadata.get("duration")
                    .and_then(serde_json::Value::as_f64)
            } else {
                return Ok(None);
            }
        } else {
            // For provider-based media, auto-play is handled by client or provider
            return Ok(None);
        };

        // Check if position is near end (within 1 second or past end)
        if let Some(dur) = duration {
            if current_position >= dur - 1.0 {
                // Auto-play next media
                self.play_next(room_id, settings).await
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
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
            state.playing_media_id = None;
            state.updated_at = chrono::Utc::now();
            // version is incremented by the SQL UPDATE, not here
        })
        .await
    }

    /// Check if playback is currently active
    pub async fn is_playing(&self, room_id: &RoomId) -> Result<bool> {
        let state = self.get_state(room_id).await?;
        Ok(state.is_playing)
    }

    /// Get current media being played
    pub async fn get_playing_media_id(&self, room_id: &RoomId) -> Result<Option<MediaId>> {
        let state = self.get_state(room_id).await?;
        Ok(state.playing_media_id)
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
                state.playing_media_id = Some(mid);
            }
            state.updated_at = chrono::Utc::now();
            // version is incremented by the SQL UPDATE, not here
        })
        .await
    }
}

#[cfg(test)]
mod tests {

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
