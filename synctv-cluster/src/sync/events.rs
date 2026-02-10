use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use synctv_core::models::id::{MediaId, RoomId, UserId};
use synctv_core::models::permission::PermissionBits;
use synctv_core::models::playback::RoomPlaybackState;

/// Events that are synchronized across cluster nodes via Redis Pub/Sub
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClusterEvent {
    /// Chat message sent in a room
    /// If position is set, this can be displayed as a danmaku (bullet comment)
    ChatMessage {
        room_id: RoomId,
        user_id: UserId,
        username: String,
        message: String,
        timestamp: DateTime<Utc>,
        /// Video position in seconds (for danmaku display)
        position: Option<f64>,
        /// Hex color (for danmaku display)
        color: Option<String>,
    },

    /// Room playback state changed (play, pause, seek, etc.)
    PlaybackStateChanged {
        room_id: RoomId,
        user_id: UserId,
        username: String,
        state: RoomPlaybackState,
        timestamp: DateTime<Utc>,
    },

    /// User joined a room
    UserJoined {
        room_id: RoomId,
        user_id: UserId,
        username: String,
        permissions: PermissionBits,
        timestamp: DateTime<Utc>,
    },

    /// User left a room
    UserLeft {
        room_id: RoomId,
        user_id: UserId,
        username: String,
        timestamp: DateTime<Utc>,
    },

    /// Media added to room playlist
    MediaAdded {
        room_id: RoomId,
        user_id: UserId,
        username: String,
        media_id: MediaId,
        media_title: String,
        timestamp: DateTime<Utc>,
    },

    /// Media removed from room playlist
    MediaRemoved {
        room_id: RoomId,
        user_id: UserId,
        username: String,
        media_id: MediaId,
        timestamp: DateTime<Utc>,
    },

    /// User permissions changed in a room
    PermissionChanged {
        room_id: RoomId,
        target_user_id: UserId,
        target_username: String,
        changed_by: UserId,
        changed_by_username: String,
        new_permissions: PermissionBits,
        timestamp: DateTime<Utc>,
    },

    /// Room settings updated
    RoomSettingsChanged {
        room_id: RoomId,
        user_id: UserId,
        username: String,
        timestamp: DateTime<Utc>,
    },

    /// WebRTC signaling message (offer, answer, `ice_candidate`)
    WebRTCSignaling {
        room_id: RoomId,
        message_type: String, // "offer", "answer", "ice_candidate"
        from: String,         // "user_id:conn_id" (server-set, prevents forgery)
        to: String,           // "user_id:conn_id"
        data: String,         // Opaque SDP/ICE data
        timestamp: DateTime<Utc>,
    },

    /// User joined WebRTC call in room
    WebRTCJoin {
        room_id: RoomId,
        user_id: UserId,
        conn_id: String,
        username: String,
        timestamp: DateTime<Utc>,
    },

    /// User left WebRTC call in room
    WebRTCLeave {
        room_id: RoomId,
        user_id: UserId,
        conn_id: String,
        timestamp: DateTime<Utc>,
    },

    /// Notification for all clients (system-wide)
    SystemNotification {
        message: String,
        level: NotificationLevel,
        timestamp: DateTime<Utc>,
    },

    /// Kick an active publisher (RTMP stream termination).
    /// Broadcast cluster-wide when admin bans user/room or deletes media/room.
    KickPublisher {
        room_id: RoomId,
        media_id: MediaId,
        reason: String,
        timestamp: DateTime<Utc>,
    },
}

/// Notification severity level
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
}

impl ClusterEvent {
    /// Get the room ID for events that belong to a specific room
    #[must_use]
    pub const fn room_id(&self) -> Option<&RoomId> {
        match self {
            Self::ChatMessage { room_id, .. }
            | Self::PlaybackStateChanged { room_id, .. }
            | Self::UserJoined { room_id, .. }
            | Self::UserLeft { room_id, .. }
            | Self::MediaAdded { room_id, .. }
            | Self::MediaRemoved { room_id, .. }
            | Self::PermissionChanged { room_id, .. }
            | Self::RoomSettingsChanged { room_id, .. }
            | Self::WebRTCSignaling { room_id, .. }
            | Self::WebRTCJoin { room_id, .. }
            | Self::WebRTCLeave { room_id, .. }
            | Self::KickPublisher { room_id, .. } => Some(room_id),
            Self::SystemNotification { .. } => None,
        }
    }

    /// Get the user ID that initiated this event
    #[must_use]
    pub const fn user_id(&self) -> Option<&UserId> {
        match self {
            Self::ChatMessage { user_id, .. }
            | Self::PlaybackStateChanged { user_id, .. }
            | Self::UserJoined { user_id, .. }
            | Self::UserLeft { user_id, .. }
            | Self::MediaAdded { user_id, .. }
            | Self::MediaRemoved { user_id, .. }
            | Self::RoomSettingsChanged { user_id, .. }
            | Self::WebRTCJoin { user_id, .. }
            | Self::WebRTCLeave { user_id, .. } => Some(user_id),
            Self::PermissionChanged { changed_by, .. } => Some(changed_by),
            Self::WebRTCSignaling { .. } | Self::SystemNotification { .. }
            | Self::KickPublisher { .. } => None,
        }
    }

    /// Get the timestamp of this event
    #[must_use]
    pub const fn timestamp(&self) -> &DateTime<Utc> {
        match self {
            Self::ChatMessage { timestamp, .. }
            | Self::PlaybackStateChanged { timestamp, .. }
            | Self::UserJoined { timestamp, .. }
            | Self::UserLeft { timestamp, .. }
            | Self::MediaAdded { timestamp, .. }
            | Self::MediaRemoved { timestamp, .. }
            | Self::PermissionChanged { timestamp, .. }
            | Self::RoomSettingsChanged { timestamp, .. }
            | Self::WebRTCSignaling { timestamp, .. }
            | Self::WebRTCJoin { timestamp, .. }
            | Self::WebRTCLeave { timestamp, .. }
            | Self::SystemNotification { timestamp, .. }
            | Self::KickPublisher { timestamp, .. } => timestamp,
        }
    }

    /// Get a short description of the event type
    #[must_use]
    pub const fn event_type(&self) -> &'static str {
        match self {
            Self::ChatMessage { .. } => "chat_message",
            Self::PlaybackStateChanged { .. } => "playback_state_changed",
            Self::UserJoined { .. } => "user_joined",
            Self::UserLeft { .. } => "user_left",
            Self::MediaAdded { .. } => "media_added",
            Self::MediaRemoved { .. } => "media_removed",
            Self::PermissionChanged { .. } => "permission_changed",
            Self::RoomSettingsChanged { .. } => "room_settings_changed",
            Self::WebRTCSignaling { .. } => "webrtc_signaling",
            Self::WebRTCJoin { .. } => "webrtc_join",
            Self::WebRTCLeave { .. } => "webrtc_leave",
            Self::SystemNotification { .. } => "system_notification",
            Self::KickPublisher { .. } => "kick_publisher",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cluster_event_serialization() {
        let event = ClusterEvent::ChatMessage {
            room_id: RoomId::from_string("room123".to_string()),
            user_id: UserId::from_string("user456".to_string()),
            username: "testuser".to_string(),
            message: "Hello world!".to_string(),
            timestamp: Utc::now(),
            position: None,
            color: None,
        };

        // Serialize to JSON
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("chat_message"));
        assert!(json.contains("Hello world!"));

        // Deserialize back
        let deserialized: ClusterEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.event_type(), "chat_message");
    }

    #[test]
    fn test_cluster_event_room_id() {
        let event = ClusterEvent::UserJoined {
            room_id: RoomId::from_string("room123".to_string()),
            user_id: UserId::from_string("user456".to_string()),
            username: "testuser".to_string(),
            permissions: PermissionBits(0),
            timestamp: Utc::now(),
        };

        assert_eq!(event.room_id().unwrap().as_str(), "room123");
        assert_eq!(event.user_id().unwrap().as_str(), "user456");
    }

    #[test]
    fn test_system_notification_no_room() {
        let event = ClusterEvent::SystemNotification {
            message: "Server maintenance in 1 hour".to_string(),
            level: NotificationLevel::Warning,
            timestamp: Utc::now(),
        };

        assert!(event.room_id().is_none());
        assert!(event.user_id().is_none());
        assert_eq!(event.event_type(), "system_notification");
    }

    #[test]
    fn test_kick_publisher_serialization() {
        let event = ClusterEvent::KickPublisher {
            room_id: RoomId::from_string("room123".to_string()),
            media_id: MediaId::from_string("media456".to_string()),
            reason: "user_banned".to_string(),
            timestamp: Utc::now(),
        };

        // Serialize to JSON
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("kick_publisher"));
        assert!(json.contains("room123"));
        assert!(json.contains("media456"));
        assert!(json.contains("user_banned"));

        // Deserialize back
        let deserialized: ClusterEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.event_type(), "kick_publisher");
        assert_eq!(deserialized.room_id().unwrap().as_str(), "room123");
        assert!(deserialized.user_id().is_none());

        if let ClusterEvent::KickPublisher { room_id, media_id, reason, .. } = &deserialized {
            assert_eq!(room_id.as_str(), "room123");
            assert_eq!(media_id.as_str(), "media456");
            assert_eq!(reason, "user_banned");
        } else {
            panic!("Expected KickPublisher variant");
        }
    }

    #[test]
    fn test_kick_publisher_has_room_id_no_user_id() {
        let event = ClusterEvent::KickPublisher {
            room_id: RoomId::from_string("room789".to_string()),
            media_id: MediaId::from_string("media012".to_string()),
            reason: "room_deleted".to_string(),
            timestamp: Utc::now(),
        };

        assert_eq!(event.room_id().unwrap().as_str(), "room789");
        assert!(event.user_id().is_none());
        assert_eq!(event.event_type(), "kick_publisher");
    }
}
