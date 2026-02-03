//! Publish key generation for RTMP live streaming
//!
//! Generates JWT tokens for RTMP push authentication.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    models::{MediaId, RoomId, UserId},
    service::auth::JwtService,
    Error, Result,
};

/// Generated publish key for RTMP streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishKey {
    /// JWT token for RTMP authentication
    pub token: String,
    /// Room ID
    pub room_id: String,
    /// Media ID (stream ID)
    pub media_id: String,
    /// User ID who requested the key
    pub user_id: String,
    /// Expiration timestamp
    pub expires_at: i64,
}

/// Claims for RTMP publish token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishClaims {
    /// Room ID
    pub room_id: String,
    /// Media ID
    pub media_id: String,
    /// User ID
    pub user_id: String,
    /// Permission to start live stream
    pub perm_start_live: bool,
    /// Issued at timestamp
    pub iat: i64,
    /// Expiration timestamp
    pub exp: i64,
    /// JWT ID (unique token identifier)
    pub jti: String,
}

/// Publish key service for generating RTMP streaming tokens
#[derive(Clone)]
pub struct PublishKeyService {
    jwt_service: JwtService,
    token_ttl_hours: i64,
}

impl std::fmt::Debug for PublishKeyService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublishKeyService")
            .field("token_ttl_hours", &self.token_ttl_hours)
            .finish()
    }
}

impl PublishKeyService {
    /// Create a new publish key service
    pub fn new(jwt_service: JwtService, token_ttl_hours: i64) -> Self {
        Self {
            jwt_service,
            token_ttl_hours,
        }
    }

    /// Create a new publish key service with default TTL (24 hours)
    pub fn with_default_ttl(jwt_service: JwtService) -> Self {
        Self::new(jwt_service, 24)
    }

    /// Generate a publish key for RTMP streaming
    ///
    /// # Arguments
    /// * `room_id` - Room ID where the stream will be published
    /// * `media_id` - Media ID (stream identifier)
    /// * `user_id` - User ID requesting the publish key
    ///
    /// # Returns
    /// A PublishKey containing the JWT token and metadata
    pub async fn generate_publish_key(
        &self,
        room_id: RoomId,
        media_id: MediaId,
        user_id: UserId,
    ) -> Result<PublishKey> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Error::Internal(format!("Time error: {}", e)))?
            .as_secs() as i64;

        let exp = now + (self.token_ttl_hours * 3600);

        // Create claims
        let claims = PublishClaims {
            room_id: room_id.as_str().to_string(),
            media_id: media_id.as_str().to_string(),
            user_id: user_id.as_str().to_string(),
            perm_start_live: true,
            iat: now,
            exp,
            jti: nanoid::nanoid!(32),
        };

        // Serialize claims to JSON
        let claims_json = serde_json::to_value(&claims)
            .map_err(|e| Error::Internal(format!("Failed to serialize claims: {}", e)))?;

        // Sign with JWT service (using RS256)
        let token = self
            .jwt_service
            .sign_custom(&claims_json)
            .await?;

        Ok(PublishKey {
            token,
            room_id: room_id.as_str().to_string(),
            media_id: media_id.as_str().to_string(),
            user_id: user_id.as_str().to_string(),
            expires_at: exp,
        })
    }

    /// Validate a publish key token
    ///
    /// # Arguments
    /// * `token` - The JWT token to validate
    ///
    /// # Returns
    /// The validated claims if the token is valid and not expired
    pub async fn validate_publish_key(&self, token: &str) -> Result<PublishClaims> {
        // Verify JWT signature and expiration
        let claims_value = self
            .jwt_service
            .verify_custom(token)
            .await?;

        // Deserialize claims
        let claims: PublishClaims = serde_json::from_value(claims_value)
            .map_err(|e| Error::Authentication(format!("Invalid token format: {}", e)))?;

        // Check expiration
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Error::Internal(format!("Time error: {}", e)))?
            .as_secs() as i64;

        if now > claims.exp {
            return Err(Error::Authentication("Token has expired".to_string()));
        }

        // Verify permission
        if !claims.perm_start_live {
            return Err(Error::Authorization(
                "Token does not have START_LIVE permission".to_string(),
            ));
        }

        Ok(claims)
    }

    /// Verify a publish key for a specific room/media
    ///
    /// # Arguments
    /// * `token` - The JWT token
    /// * `room_id` - Expected room ID
    /// * `media_id` - Expected media ID
    ///
    /// # Returns
    /// The user ID if the token is valid for this room/media
    pub async fn verify_publish_key_for_stream(
        &self,
        token: &str,
        room_id: &RoomId,
        media_id: &MediaId,
    ) -> Result<UserId> {
        let claims = self.validate_publish_key(token).await?;

        // Verify room and media match
        if claims.room_id != room_id.as_str() {
            return Err(Error::Authorization(format!(
                "Token room mismatch: expected {}, got {}",
                room_id.as_str(),
                claims.room_id
            )));
        }

        if claims.media_id != media_id.as_str() {
            return Err(Error::Authorization(format!(
                "Token media mismatch: expected {}, got {}",
                media_id.as_str(),
                claims.media_id
            )));
        }

        Ok(UserId::from_string(claims.user_id))
    }
}

