// RTMP Authentication implementation based on synctv-go
//
// JWT Claims structure (from synctv-go internal/rtmp/rtmp.go):
// type Claims struct {
//     MovieID string `json:"m"`
//     jwt.RegisteredClaims
// }
//
// The token uses HS256 (HMAC) with a shared secret, not RS256.
// For now, we'll use PublishKeyService which already handles token validation.

use crate::protocols::rtmp::auth::{RtmpAuthCallback, Channel};
use crate::error::{StreamError, StreamResult};
use async_trait::async_trait;
use std::sync::Arc;
use synctv_core::service::PublishKeyService;
use synctv_core::models::{RoomId, MediaId};
use tracing::{warn, info, debug};

/// RTMP authentication callback implementation
///
/// Validates RTMP publish requests using JWT tokens from PublishKeyService.
/// Based on synctv-go internal/rtmp/rtmp.go and internal/bootstrap/rtmp.go
pub struct RtmpAuthCallbackImpl {
    publish_key_service: Arc<PublishKeyService>,
}

impl RtmpAuthCallbackImpl {
    pub fn new(publish_key_service: Arc<PublishKeyService>) -> Self {
        Self {
            publish_key_service,
        }
    }

    /// Validate RTMP publish token and extract media_id
    ///
    /// Token format from synctv-go:
    /// - JWT with "m" claim containing movie_id (media_id)
    /// - Claims also include room_id, user_id, permissions
    async fn validate_publish_token(&self, token: &str, room_id: &str, media_id: &str) -> Result<String, String> {
        // Use PublishKeyService to validate the token
        match self
            .publish_key_service
            .verify_publish_key_for_stream(
                token,
                &RoomId::from_string(room_id.to_string()),
                &MediaId::from_string(media_id.to_string()),
            )
            .await
        {
            Ok(user_id) => {
                info!(
                    "RTMP publish token validated: room_id={}, media_id={}, user_id={}",
                    room_id,
                    media_id,
                    user_id.as_str()
                );
                Ok(media_id.to_string())
            }
            Err(e) => {
                warn!("RTMP publish token validation failed: {}", e);
                Err(format!("Invalid token: {}", e))
            }
        }
    }
}

#[async_trait]
impl RtmpAuthCallback for RtmpAuthCallbackImpl {
    async fn authenticate(
        &self,
        app_name: &str,
        channel_name: &str,
        is_publisher: bool,
    ) -> StreamResult<Channel> {
        debug!(
            "RTMP auth request: app={}, channel={}, publisher={}",
            app_name, channel_name, is_publisher
        );

        // Based on synctv-go internal/bootstrap/rtmp.go:
        // - reqAppName = room_id
        // - reqChannelName = JWT token (for publishers) or movie_id (for players)
        // - Room validation: check if banned or pending

        if is_publisher {
            // Publisher: channel_name is the JWT token
            // The actual media_id should be extracted from the JWT token
            // For now, we'll validate the token and extract media_id from it

            // First, try to decode the token to get the media_id
            // Token format: JWT with claims including "media_id" (from PublishClaims)
            match self
                .publish_key_service
                .validate_publish_key(channel_name)
                .await
            {
                Ok(claims) => {
                    info!(
                        "RTMP publisher authenticated: room_id={}, media_id={}, user_id={}",
                        claims.room_id, claims.media_id, claims.user_id
                    );

                    // Verify room_id matches
                    if claims.room_id != app_name {
                        return Err(StreamError::PermissionDenied(format!(
                            "Room ID mismatch: token has {}, request is for {}",
                            claims.room_id, app_name
                        )));
                    }

                    Ok(Channel {
                        room_id: claims.room_id,
                        channel_name: claims.media_id,
                        is_publisher: true,
                    })
                }
                Err(e) => {
                    warn!("RTMP publisher authentication failed: {}", e);
                    return Err(StreamError::AuthenticationFailed(format!(
                        "Authentication failed: {}",
                        e
                    )));
                }
            }
        } else {
            // Player: channel_name is movie_id directly
            // In synctv-go, players just use the movie_id as channel name
            // No authentication needed for playing (stream must already exist)

            info!(
                "RTMP player authenticated: room_id={}, movie_id={}",
                app_name, channel_name
            );

            Ok(Channel {
                room_id: app_name.to_string(),
                channel_name: channel_name.to_string(),
                is_publisher: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::rtmp::auth::{Channel, NoAuthCallback};
    use std::sync::Arc;

    // Mock PublishKeyService for testing
    struct MockPublishKeyService;

    #[allow(dead_code)]
    impl MockPublishKeyService {
        // For testing purposes - would need real PublishKeyService integration
    }

    #[tokio::test]
    async fn test_channel_struct() {
        let channel = Channel {
            room_id: "test_room".to_string(),
            channel_name: "test_movie".to_string(),
            is_publisher: true,
        };

        assert_eq!(channel.room_id, "test_room");
        assert_eq!(channel.channel_name, "test_movie");
        assert!(channel.is_publisher);
    }

    #[tokio::test]
    async fn test_channel_player() {
        let channel = Channel {
            room_id: "test_room".to_string(),
            channel_name: "test_movie".to_string(),
            is_publisher: false,
        };

        assert_eq!(channel.room_id, "test_room");
        assert_eq!(channel.channel_name, "test_movie");
        assert!(!channel.is_publisher);
    }

    // Note: Integration tests for RtmpAuthCallbackImpl require:
    // - Actual PublishKeyService with JWT signing/verification
    // - Those are tested in synctv-core/src/service/publish_key.rs
}
