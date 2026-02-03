// RTMP Authentication Callback
//
// Provides authentication and authorization for RTMP publish/play requests
// Based on Go implementation: internal/bootstrap/rtmp.go

use async_trait::async_trait;
use crate::error::StreamResult;

/// Channel information for RTMP connection
#[derive(Debug, Clone)]
pub struct Channel {
    pub room_id: String,
    pub channel_name: String,
    pub is_publisher: bool,
}

/// RTMP authentication callback trait
///
/// Implementations should verify:
/// - Room exists and is not banned/pending
/// - For publishers: validate stream_key (JWT token)
/// - For players: check if RTMP player is enabled
#[async_trait]
pub trait RtmpAuthCallback: Send + Sync {
    /// Authenticate RTMP connection
    ///
    /// # Arguments
    /// * `app_name` - Application name (room_id)
    /// * `channel_name` - Stream channel name (stream_key for publish, or channel for play)
    /// * `is_publisher` - Whether this is a publish or play request
    ///
    /// # Returns
    /// * `Ok(Channel)` - Authentication successful, returns channel info
    /// * `Err(StreamError)` - Authentication failed
    async fn authenticate(
        &self,
        app_name: &str,
        channel_name: &str,
        is_publisher: bool,
    ) -> StreamResult<Channel>;
}

/// Default no-op auth callback (accepts all connections)
pub struct NoAuthCallback;

#[async_trait]
impl RtmpAuthCallback for NoAuthCallback {
    async fn authenticate(
        &self,
        app_name: &str,
        channel_name: &str,
        is_publisher: bool,
    ) -> StreamResult<Channel> {
        tracing::warn!(
            "No authentication configured for RTMP: app={}, channel={}, publisher={}",
            app_name,
            channel_name,
            is_publisher
        );

        Ok(Channel {
            room_id: app_name.to_string(),
            channel_name: channel_name.to_string(),
            is_publisher,
        })
    }
}
