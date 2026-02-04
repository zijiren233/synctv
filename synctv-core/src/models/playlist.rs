//! Playlist model (directory/folder in tree structure)
//!
//! Design reference: /Volumes/workspace/rust/design/04-数据库设计.md §2.4.1

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use super::id::{RoomId, UserId, PlaylistId};

/// Playlist (directory/folder)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: PlaylistId,
    pub room_id: RoomId,
    pub creator_id: UserId,
    pub name: String,
    pub parent_id: Option<PlaylistId>,
    pub position: i32,

    // Dynamic folder fields
    /// Provider type name for dynamic folders (e.g., "alist", "emby")
    /// NULL for static folders (manually added media)
    pub source_provider: Option<String>,
    pub source_config: Option<JsonValue>,
    pub provider_instance_name: Option<String>,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Playlist {
    /// Check if this is a root playlist
    #[must_use] 
    pub const fn is_root(&self) -> bool {
        self.parent_id.is_none() && self.name.is_empty()
    }

    /// Check if this is a dynamic folder
    #[must_use] 
    pub const fn is_dynamic(&self) -> bool {
        self.source_provider.is_some()
    }

    /// Check if this is a static folder
    #[must_use] 
    pub const fn is_static(&self) -> bool {
        self.source_provider.is_none()
    }
}

/// Create playlist request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePlaylistRequest {
    pub room_id: RoomId,
    pub name: String,
    pub parent_id: Option<PlaylistId>,
    pub position: Option<i32>,

    // Dynamic folder fields
    pub source_provider: Option<String>,
    pub source_config: Option<JsonValue>,
    pub provider_instance_name: Option<String>,
}

/// Update playlist request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePlaylistRequest {
    pub name: Option<String>,
    pub position: Option<i32>,
}

/// Playlist with media count (for efficient queries)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistWithCount {
    #[serde(flatten)]
    pub playlist: Playlist,
    pub media_count: i64,
    pub children_count: i64,
}

