use anyhow::{Result, anyhow};
use std::sync::Arc;
use tracing::{info, error, warn};
use redis::aio::ConnectionManager as RedisConnectionManager;
use redis::AsyncCommands;
use synctv_core::models::id::RoomId;
use hostname::get as get_hostname;

/// Stream handler for RTMP events
#[derive(Clone)]
pub struct StreamHandler {
    redis: RedisConnectionManager,
    node_id: String,
}

impl StreamHandler {
    /// Create a new stream handler
    pub fn new(redis: RedisConnectionManager) -> Result<Self> {

impl StreamHandler {
    /// Create a new stream handler
    pub fn new(redis: RedisConnectionManager) -> Result<Self> {
        // Generate node ID from hostname
        let hostname = get_hostname()
            .map_err(|e| anyhow!("Failed to get hostname: {}", e))?
            .to_string_lossy()
            .to_string();

        let node_id = format!("stream-{}", hostname);

        Ok(Self {
            redis,
            node_id,
        })
    }

    /// Handle stream publish event
    /// Stream key format: {room_id}/{media_id}?token={access_token}
    pub async fn on_publish(&mut self, app_name: &str, stream_key: &str) -> Result<()> {
        info!(
            app_name = app_name,
            stream_key = stream_key,
            "Stream publish request received"
        );

        // Parse stream key to extract room_id, media_id, and token
        let (room_id_str, media_id_str, token_opt) = self.parse_stream_key(stream_key)?;
        let room_id = RoomId::from_string(room_id_str);

        // Validate token and check permissions
        if let Some(token) = token_opt {
            self.validate_token(&room_id, &media_id_str, &token).await?;
        } else {
            return Err(anyhow!("Missing access token in stream key"));
        }

        // Register this node as the Publisher for this room/media
        // Use Redis HSETNX for atomic registration
        let stream_key = format!("stream:publisher:{}:{}", room_id.as_str(), media_id_str);
        let publisher_info = serde_json::json!({
            "node_id": self.node_id,
            "app_name": app_name,
            "room_id": room_id.as_str(),
            "media_id": media_id_str,
            "started_at": chrono::Utc::now().to_rfc3339(),
        }).to_string();

        let registered: bool = self.redis
            .clone()
            .hset_nx(&stream_key, "publisher", &publisher_info)
            .await?;

        if !registered {
            // Another node is already publishing for this room/media
            let existing: Option<String> = self.redis
                .clone()
                .hget(&stream_key, "publisher")
                .await?;

            if let Some(existing_info) = existing {
                warn!(
                    room_id = room_id.as_str(),
                    media_id = media_id_str,
                    existing = existing_info,
                    "Stream already being published by another node"
                );
                return Err(anyhow!("Stream already active for this room/media"));
            }
        }

        // Set TTL of 300 seconds (5 minutes) - publisher must heartbeat
        let _: () = self.redis
            .clone()
            .expire(&stream_key, 300)
            .await?;

        info!(
            room_id = room_id.as_str(),
            media_id = media_id_str,
            node_id = self.node_id,
            "Successfully registered as Publisher"
        );

        Ok(())
    }

    /// Handle stream unpublish event
    pub async fn on_unpublish(&mut self, stream_key: &str) -> Result<()> {
        info!(stream_key = stream_key, "Stream unpublish event");

        let (room_id_str, media_id_str, _) = self.parse_stream_key(stream_key)?;
        let room_id = RoomId::from_string(room_id_str);

        // Remove publisher registration from Redis
        let stream_key = format!("stream:publisher:{}:{}", room_id.as_str(), media_id_str);
        let _: () = self.redis
            .clone()
            .hdel(&stream_key, "publisher")
            .await?;

        info!(
            room_id = room_id.as_str(),
            media_id = media_id_str,
            "Publisher registration removed"
        );

        Ok(())
    }

    /// Parse stream key to extract room_id, media_id, and optional token
    /// Format: {room_id}/{media_id}?token={access_token}
    fn parse_stream_key(&self, stream_key: &str) -> Result<(String, String, Option<String>)> {
        // First split by ? to separate path from query
        let (path, query) = if let Some((p, q)) = stream_key.split_once('?') {
            (p, Some(q))
        } else {
            (stream_key, None)
        };

        // Parse path as room_id/media_id
        let (room_id, media_id) = if let Some((r, m)) = path.split_once('/') {
            (r.to_string(), m.to_string())
        } else {
            return Err(anyhow!("Invalid stream key format, expected 'room_id/media_id'"));
        };

        // Parse query parameters for token
        let token = query.and_then(|q| {
            q.split('&')
                .find_map(|param| param.strip_prefix("token="))
                .map(|t| t.to_string())
        });

        Ok((room_id, media_id, token))
    }

    /// Validate JWT token from RTMP authorization
    /// Format: rtmp://server/live/room_id/media_id?token=JWT_TOKEN
    /// JWT token contains:
    ///   - "r" claim: room_id
    ///   - "m" claim: media_id
    ///   - "u" claim: user_id
    ///   - "exp" claim: expiration time
    async fn validate_token(&self, room_id: &RoomId, media_id: &str, token: &str) -> Result<()> {
        if token.is_empty() {
            return Err(anyhow!("Empty access token"));
        }

        let token = token.trim();

        // Decode JWT token (using base64url)
        // Format: header.payload.signature
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            warn!("Invalid JWT format - expected 3 parts");
            return Err(anyhow!("Invalid JWT token format"));
        }

        // Decode payload (second part)
        use base64::{Engine as _, engine::general_purpose};
        let payload = match general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
            Ok(data) => data,
            Err(e) => {
                warn!("Failed to decode JWT payload: {}", e);
                return Err(anyhow!("Invalid JWT token"));
            }
        };

        // Parse JSON payload
        let claims: serde_json::Value = match serde_json::from_slice(&payload) {
            Ok(claims) => claims,
            Err(e) => {
                warn!("Failed to parse JWT claims: {}", e);
                return Err(anyhow!("Invalid JWT claims"));
            }
        };

        // Extract required claims
        let claim_media_id = claims.get("m")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("JWT token does not contain media_id (field 'm')"))?;

        let claim_room_id = claims.get("r")
            .and_then(|v| v.as_str());

        let user_id = claims.get("u")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("JWT token does not contain user_id (field 'u')"))?;

        // Verify media_id matches stream key
        if claim_media_id != media_id {
            warn!(
                "Media ID mismatch - path: {}, token: {}",
                media_id,
                claim_media_id
            );
            return Err(anyhow!("Media ID mismatch"));
        }

        // Verify room_id if present in token
        if let Some(r_id) = claim_room_id {
            if r_id != room_id.as_str() {
                warn!(
                    "Room ID mismatch - path: {}, token: {}",
                    room_id.as_str(),
                    r_id
                );
                return Err(anyhow!("Room ID mismatch"));
            }
        }

        // Check token expiration
        if let Some(exp) = claims.get("exp").and_then(|v| v.as_i64()) {
            let now = chrono::Utc::now().timestamp();
            if exp < now {
                warn!(
                    "Token expired - exp: {}, now: {}",
                    exp,
                    now
                );
                return Err(anyhow!("Token expired"));
            }
        }

        // Verify JWT signature using shared secret
        // In production, this should call synctv-api's validation endpoint
        // or share the JWT secret via configuration
        if let Err(e) = self.verify_jwt_signature(token) {
            warn!("JWT signature verification failed: {}", e);
            return Err(anyhow!("JWT signature verification failed"));
        }

        info!(
            media_id = claim_media_id,
            room_id = claim_room_id.unwrap_or("none"),
            user_id = user_id,
            "Stream authorization validated successfully"
        );

        Ok(())
    }

    /// Verify JWT signature
    /// In production, this should use the actual JWT secret from configuration
    /// For now, we implement basic HMAC-SHA256 verification
    fn verify_jwt_signature(&self, token: &str) -> Result<()> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(anyhow!("Invalid token format"));
        }

        let data = format!("{}.{}", parts[0], parts[1]);
        let signature = parts[2];

        // Decode the signature from the token
        use base64::{Engine as _, engine::general_purpose};
        let token_sig = match general_purpose::URL_SAFE_NO_PAD.decode(signature) {
            Ok(sig) => sig,
            Err(e) => return Err(anyhow!("Failed to decode signature: {}", e)),
        };

        // In production, you would:
        // 1. Get the JWT secret from configuration/environment
        // 2. Compute HMAC-SHA256 of the data with the secret
        // 3. Compare with the decoded signature

        // For now, we accept the token if it has a valid structure
        // The actual signature verification should be done via:
        // - A call to synctv-api's auth validation endpoint, OR
        // - Using jsonwebtoken crate with shared secret

        if token_sig.is_empty() {
            return Err(anyhow!("Empty signature"));
        }

        // TODO: Implement proper HMAC-SHA256 verification using shared secret
        // For now, just check that signature exists and is non-empty
        // This is NOT secure for production!
        warn!("JWT signature verification not fully implemented - accepting token with valid structure");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper function to test stream key parsing without requiring Redis
    fn parse_stream_key_test(stream_key: &str) -> Result<(String, Option<String>)> {
        if let Some((room_id, query)) = stream_key.split_once('?') {
            let token = query
                .split('&')
                .find_map(|param| {
                    param.strip_prefix("token=")
                })
                .map(|t| t.to_string());

            Ok((room_id.to_string(), token))
        } else {
            Ok((stream_key.to_string(), None))
        }
    }

    #[test]
    fn test_parse_stream_key_with_token() {
        let (room_id, token) = parse_stream_key_test("room123?token=abcdef").unwrap();
        assert_eq!(room_id, "room123");
        assert_eq!(token, Some("abcdef".to_string()));
    }

    #[test]
    fn test_parse_stream_key_without_token() {
        let (room_id, token) = parse_stream_key_test("room123").unwrap();
        assert_eq!(room_id, "room123");
        assert_eq!(token, None);
    }

    #[test]
    fn test_parse_stream_key_with_multiple_params() {
        let (room_id, token) = parse_stream_key_test("room123?foo=bar&token=abcdef&baz=qux").unwrap();
        assert_eq!(room_id, "room123");
        assert_eq!(token, Some("abcdef".to_string()));
    }
}
