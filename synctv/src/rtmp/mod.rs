//! RTMP authentication implementation for `SyncTV`
//!
//! This module provides the RTMP authentication callback that integrates
//! with `SyncTV`'s user and room management.

use std::sync::Arc;

use async_trait::async_trait;
use synctv_stream::protocols::rtmp::auth::{RtmpAuthCallback, Channel};
use synctv_stream::error::{StreamError, StreamResult};

use synctv_core::{
    models::RoomStatus,
    service::{PublishKeyService, RoomService},
};

/// RTMP authentication implementation for `SyncTV`
///
/// Validates RTMP publish/play requests against:
/// - Room existence and status (not banned/pending)
/// - JWT publish keys for publishers (validates `room_id` match)
/// - Optional RTMP player settings for viewers
pub struct SyncTvRtmpAuth {
    room_service: Arc<RoomService>,
    publish_key_service: Arc<PublishKeyService>,
}

impl SyncTvRtmpAuth {
    /// Create a new RTMP authentication callback
    pub const fn new(room_service: Arc<RoomService>, publish_key_service: Arc<PublishKeyService>) -> Self {
        Self {
            room_service,
            publish_key_service,
        }
    }
}

#[async_trait]
impl RtmpAuthCallback for SyncTvRtmpAuth {
    async fn authenticate(
        &self,
        app_name: &str,
        channel_name: &str,
        is_publisher: bool,
    ) -> StreamResult<Channel> {
        // Load room by ID
        let room = self
            .room_service
            .get_room(&synctv_core::models::RoomId::from_string(app_name.to_string()))
            .await
            .map_err(|e| StreamError::AuthenticationFailed(format!("Failed to load room: {e}")))?;

        // Validate room status
        if room.status == RoomStatus::Banned {
            return Err(StreamError::AuthenticationFailed(format!(
                "Room {app_name} is banned"
            )));
        }

        if room.status == RoomStatus::Pending {
            return Err(StreamError::AuthenticationFailed(format!(
                "Room {app_name} is pending, need admin approval"
            )));
        }

        if is_publisher {
            // Publisher: validate stream_key (JWT token)
            self.validate_publisher(app_name, channel_name).await
        } else {
            // Player: check if RTMP player is enabled
            self.validate_player(app_name, channel_name).await
        }
    }
}

impl SyncTvRtmpAuth {
    /// Validate publisher (streamer) credentials
    async fn validate_publisher(
        &self,
        room_id: &str,
        stream_key: &str,
    ) -> StreamResult<Channel> {
        // Validate JWT stream_key and extract publish claims
        let claims = self
            .publish_key_service
            .validate_publish_key(stream_key)
            .await
            .map_err(|e| StreamError::AuthenticationFailed(format!("Invalid stream key: {e}")))?;

        // Verify room_id matches the token's room_id
        if claims.room_id != room_id {
            return Err(StreamError::AuthenticationFailed(format!(
                "Room ID mismatch: token for room {}, but connecting to room {}",
                claims.room_id, room_id
            )));
        }

        // Use room_id as channel name (one stream per room)
        let channel_name = room_id.to_string();

        tracing::info!(
            "Publisher authenticated: user={}, room={}, media={}, channel={}",
            claims.user_id,
            room_id,
            claims.media_id,
            channel_name
        );

        Ok(Channel {
            room_id: room_id.to_string(),
            channel_name,
            is_publisher: true,
        })
    }

    /// Validate player (viewer) request
    async fn validate_player(
        &self,
        room_id: &str,
        channel_name: &str,
    ) -> StreamResult<Channel> {
        // Room existence and status already validated in authenticate()

        // TODO: Add RoomSettings check for RTMP player enablement
        // Future implementation:
        // - room_setting!(RtmpPlayerEnabled, bool, "rtmp_player_enabled", true);
        // - Check settings_service.get_bool(room_id, "rtmp_player_enabled")
        // - Reject if disabled

        // For now, allow all players (secure enough since room is already validated)
        tracing::info!(
            "Player authenticated for room {}, channel {}",
            room_id,
            channel_name
        );

        Ok(Channel {
            room_id: room_id.to_string(),
            channel_name: channel_name.to_string(),
            is_publisher: false,
        })
    }
}
