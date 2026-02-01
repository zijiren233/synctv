//! RTMP authentication implementation for SyncTV
//!
//! This module provides the RTMP authentication callback that integrates
//! with SyncTV's user and room management.

use std::sync::Arc;

use async_trait::async_trait;
use synctv_stream::rtmp::auth::{RtmpAuthCallback, Channel};
use synctv_stream::error::{StreamError, StreamResult};

use synctv_core::{
    models::RoomStatus,
    service::{JwtService, RoomService},
};

/// RTMP authentication implementation for SyncTV
///
/// Validates RTMP publish/play requests against:
/// - Room existence and status (not banned/pending)
/// - JWT tokens for publishers
/// - Optional RTMP player settings for viewers
pub struct SyncTvRtmpAuth {
    room_service: Arc<RoomService>,
    jwt_service: JwtService,
}

impl SyncTvRtmpAuth {
    /// Create a new RTMP authentication callback
    pub fn new(room_service: Arc<RoomService>, jwt_service: JwtService) -> Self {
        Self {
            room_service,
            jwt_service,
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
            .map_err(|e| StreamError::AuthenticationFailed(format!("Failed to load room: {}", e)))?;

        // Validate room status
        if room.status == RoomStatus::Banned {
            return Err(StreamError::AuthenticationFailed(format!(
                "Room {} is banned",
                app_name
            )));
        }

        if room.status == RoomStatus::Pending {
            return Err(StreamError::AuthenticationFailed(format!(
                "Room {} is pending, need admin approval",
                app_name
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
        // Validate JWT stream_key
        let _claims = self
            .jwt_service
            .verify_access_token(stream_key)
            .map_err(|e| StreamError::AuthenticationFailed(format!("Invalid stream key: {}", e)))?;

        // TODO: Extract channel name from JWT claims or use default
        let channel_name = "live".to_string();

        tracing::info!("Publisher authenticated for room {}, channel {}", room_id, channel_name);

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
        // TODO: Check if RTMP player is enabled in settings
        // For now, allow all players
        tracing::info!("Player authenticated for room {}, channel {}", room_id, channel_name);

        Ok(Channel {
            room_id: room_id.to_string(),
            channel_name: channel_name.to_string(),
            is_publisher: false,
        })
    }
}
