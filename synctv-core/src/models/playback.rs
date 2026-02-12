use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::id::{MediaId, PlaylistId, RoomId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomPlaybackState {
    pub room_id: RoomId,
    pub playing_media_id: Option<MediaId>,
    pub playing_playlist_id: Option<PlaylistId>,
    pub relative_path: String,
    pub current_time: f64, // playback position in seconds
    pub speed: f64,        // 0.5, 1.0, 1.5, 2.0, etc.
    pub is_playing: bool,
    pub updated_at: DateTime<Utc>,
    pub version: i32, // For optimistic locking
}

impl RoomPlaybackState {
    #[must_use]
    pub fn new(room_id: RoomId) -> Self {
        Self {
            room_id,
            playing_media_id: None,
            playing_playlist_id: None,
            relative_path: String::new(),
            current_time: 0.0,
            speed: 1.0,
            is_playing: false,
            updated_at: Utc::now(),
            version: 0,
        }
    }

    pub fn play(&mut self) {
        self.is_playing = true;
        self.updated_at = Utc::now();
        self.version += 1;
    }

    pub fn pause(&mut self) {
        self.is_playing = false;
        self.updated_at = Utc::now();
        self.version += 1;
    }

    pub fn seek(&mut self, current_time: f64) {
        self.current_time = current_time;
        self.updated_at = Utc::now();
        self.version += 1;
    }

    pub fn change_speed(&mut self, speed: f64) {
        self.speed = speed;
        self.updated_at = Utc::now();
        self.version += 1;
    }

    pub fn switch_media(
        &mut self,
        media_id: MediaId,
        playlist_id: Option<PlaylistId>,
        media_path: String,
    ) {
        self.playing_media_id = Some(media_id);
        self.playing_playlist_id = playlist_id;
        self.relative_path = media_path;
        self.current_time = 0.0;
        self.is_playing = false;
        self.updated_at = Utc::now();
        self.version += 1;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackControlRequest {
    pub room_id: RoomId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeekRequest {
    pub room_id: RoomId,
    pub current_time: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeSpeedRequest {
    pub room_id: RoomId,
    pub speed: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchMediaRequest {
    pub room_id: RoomId,
    pub media_id: MediaId,
}
