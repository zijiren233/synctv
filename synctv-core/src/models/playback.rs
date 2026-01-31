use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::id::{MediaId, RoomId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomPlaybackState {
    pub room_id: RoomId,
    pub current_media_id: Option<MediaId>,
    pub position: f64, // seconds
    pub speed: f64,    // 0.5, 1.0, 1.5, 2.0, etc.
    pub is_playing: bool,
    pub updated_at: DateTime<Utc>,
    pub version: i32, // For optimistic locking
}

impl RoomPlaybackState {
    pub fn new(room_id: RoomId) -> Self {
        Self {
            room_id,
            current_media_id: None,
            position: 0.0,
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

    pub fn seek(&mut self, position: f64) {
        self.position = position;
        self.updated_at = Utc::now();
        self.version += 1;
    }

    pub fn change_speed(&mut self, speed: f64) {
        self.speed = speed;
        self.updated_at = Utc::now();
        self.version += 1;
    }

    pub fn switch_media(&mut self, media_id: MediaId) {
        self.current_media_id = Some(media_id);
        self.position = 0.0;
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
    pub position: f64,
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
