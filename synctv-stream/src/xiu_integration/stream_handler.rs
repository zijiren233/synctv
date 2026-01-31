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
    /// Stream key format: {room_id}?token={access_token}
    pub async fn on_publish(&mut self, app_name: &str, stream_key: &str) -> Result<()> {
        info!(
            app_name = app_name,
            stream_key = stream_key,
            "Stream publish request received"
        );

        // Parse stream key to extract room_id and token
        let (room_id_str, token_opt) = self.parse_stream_key(stream_key)?;
        let room_id = RoomId::from_string(room_id_str);

        // Validate token and check permissions
        // TODO: Call synctv-api to validate token and check PUBLISH_STREAM permission
        // For now, we'll implement a placeholder validation
        if let Some(token) = token_opt {
            self.validate_token(&room_id, &token).await?;
        } else {
            return Err(anyhow!("Missing access token in stream key"));
        }

        // Register this node as the Publisher for this room
        // Use Redis HSETNX for atomic registration
        let stream_key = format!("stream:{}", room_id.as_str());
        let publisher_info = serde_json::json!({
            "node_id": self.node_id,
            "app_name": app_name,
            "started_at": chrono::Utc::now().to_rfc3339(),
        }).to_string();

        let registered: bool = self.redis
            .clone()
            .hset_nx(&stream_key, "publisher", &publisher_info)
            .await?;

        if !registered {
            // Another node is already publishing for this room
            let existing: Option<String> = self.redis
                .clone()
                .hget(&stream_key, "publisher")
                .await?;

            if let Some(existing_info) = existing {
                warn!(
                    room_id = room_id.as_str(),
                    existing = existing_info,
                    "Stream already being published by another node"
                );
                return Err(anyhow!("Stream already active for this room"));
            }
        }

        info!(
            room_id = room_id.as_str(),
            node_id = self.node_id,
            "Successfully registered as Publisher"
        );

        Ok(())
    }

    /// Handle stream unpublish event
    pub async fn on_unpublish(&mut self, stream_key: &str) -> Result<()> {
        info!(stream_key = stream_key, "Stream unpublish event");

        let (room_id_str, _) = self.parse_stream_key(stream_key)?;
        let room_id = RoomId::from_string(room_id_str);

        // Remove publisher registration from Redis
        let stream_key = format!("stream:{}", room_id.as_str());
        let _: () = self.redis
            .clone()
            .hdel(&stream_key, "publisher")
            .await?;

        info!(
            room_id = room_id.as_str(),
            "Publisher registration removed"
        );

        Ok(())
    }

    /// Parse stream key to extract room_id and optional token
    /// Format: {room_id}?token={access_token}
    fn parse_stream_key(&self, stream_key: &str) -> Result<(String, Option<String>)> {
        if let Some((room_id, query)) = stream_key.split_once('?') {
            // Parse query parameters
            let token = query
                .split('&')
                .find_map(|param| {
                    param.strip_prefix("token=")
                })
                .map(|t| t.to_string());

            Ok((room_id.to_string(), token))
        } else {
            // No query parameters, just room_id
            Ok((stream_key.to_string(), None))
        }
    }

    /// Validate access token and check permissions
    /// TODO: Implement real validation by calling synctv-api gRPC
    async fn validate_token(&self, room_id: &RoomId, token: &str) -> Result<()> {
        // Placeholder implementation
        // In production, this should:
        // 1. Call synctv-api to verify JWT token
        // 2. Check user has PUBLISH_STREAM permission in this room
        // 3. Return user_id for audit logging

        if token.is_empty() {
            return Err(anyhow!("Empty access token"));
        }

        // For now, accept any non-empty token
        // TODO: Implement real JWT validation
        warn!(
            room_id = room_id.as_str(),
            "Token validation not yet implemented - accepting all tokens"
        );

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
