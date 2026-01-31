// RTMP Provider
//
// Handles live streaming via RTMP push/pull.
//
// Features:
// - Live streaming support
// - No authentication required
// - Shared cache (room-level)
// - Permanent URLs (no expiration)
// - Integration with xiu RTMP server
//
// URL format: rtmp://server/live/{room_id}/{stream_key}

use synctv_core::provider::{
    MediaProvider, PlaybackInfo, PlaybackResult, ProviderContext, ProviderError,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

/// RTMP provider for live streaming
pub struct RtmpProvider {
    instance_id: String,
    base_url: String, // Base URL for HLS/FLV playback
}

impl RtmpProvider {
    /// Create new RTMP provider
    ///
    /// # Arguments
    /// - `instance_id`: Unique instance ID (e.g., "rtmp_main")
    /// - `base_url`: Base URL for playback (e.g., "https://synctv.example.com")
    pub fn new(instance_id: String, base_url: String) -> Self {
        Self {
            instance_id,
            base_url,
        }
    }

    /// Create from configuration
    pub fn from_config(instance_id: &str, config: Value) -> Result<Self, ProviderError> {
        let base_url = config
            .get("base_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::MissingField("base_url".to_string()))?;

        Ok(Self::new(instance_id.to_string(), base_url.to_string()))
    }
}

impl Default for RtmpProvider {
    fn default() -> Self {
        Self::new("rtmp_default".to_string(), "http://localhost:8080".to_string())
    }
}

#[async_trait]
impl MediaProvider for RtmpProvider {
    fn name(&self) -> &'static str {
        "rtmp"
    }

    fn instance_id(&self) -> &str {
        &self.instance_id
    }

    async fn generate_playback(
        &self,
        _ctx: &ProviderContext<'_>,
        source_config: &Value,
    ) -> Result<PlaybackResult, ProviderError> {
        // Extract configuration
        let stream_key = source_config
            .get("stream_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::MissingField("stream_key".to_string()))?;

        let room_id = source_config
            .get("room_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::MissingField("room_id".to_string()))?;

        // Generate playback URLs
        // Note: The actual URL structure depends on your xiu server configuration
        let hls_url = format!("{}/live/{}/{}/index.m3u8", self.base_url, room_id, stream_key);
        let flv_url = format!("{}/live/{}/{}.flv", self.base_url, room_id, stream_key);

        // Create HLS playback info
        let hls_info = PlaybackInfo {
            urls: vec![hls_url.clone()],
            format: "m3u8".to_string(),
            headers: HashMap::new(),
            subtitles: vec![],
            expires_at: None, // Live streams don't expire
        };

        // Create FLV playback info
        let flv_info = PlaybackInfo {
            urls: vec![flv_url.clone()],
            format: "flv".to_string(),
            headers: HashMap::new(),
            subtitles: vec![],
            expires_at: None,
        };

        // Build playback modes
        let mut playback_infos = HashMap::new();
        playback_infos.insert("hls".to_string(), hls_info);
        playback_infos.insert("flv".to_string(), flv_info);

        // Metadata
        let mut metadata = HashMap::new();
        metadata.insert("is_live".to_string(), json!(true));
        metadata.insert("stream_key".to_string(), json!(stream_key));
        metadata.insert("room_id".to_string(), json!(room_id));

        Ok(PlaybackResult {
            playback_infos,
            default_mode: "hls".to_string(), // HLS is default
            metadata,
        })
    }

    fn cache_key(&self, ctx: &ProviderContext<'_>, source_config: &Value) -> String {
        // RTMP streams are shared at room level (not user-specific)
        let room_id = source_config
            .get("room_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let stream_key = source_config
            .get("stream_key")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        format!(
            "{}:playback:rtmp:{}:{}:shared",
            ctx.key_prefix, room_id, stream_key
        )
    }

    async fn validate_source_config(
        &self,
        _ctx: &ProviderContext<'_>,
        source_config: &Value,
    ) -> Result<(), ProviderError> {
        // Validate required fields
        source_config
            .get("stream_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::MissingField("stream_key".to_string()))?;

        source_config
            .get("room_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::MissingField("room_id".to_string()))?;

        Ok(())
    }

    fn needs_service_registration(&self) -> bool {
        false // RTMP provider doesn't register custom HTTP/gRPC endpoints
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rtmp_provider() {
        let provider = RtmpProvider::new(
            "rtmp_test".to_string(),
            "https://synctv.example.com".to_string(),
        );

        assert_eq!(provider.name(), "rtmp");
        assert_eq!(provider.instance_id(), "rtmp_test");

        let capabilities = provider.capabilities();
        assert!(!capabilities.can_parse);
        assert!(capabilities.can_play);
        assert!(!capabilities.supports_subtitles);
        assert!(!capabilities.requires_auth);
    }

    #[tokio::test]
    async fn test_generate_playback() {
        let provider = RtmpProvider::new(
            "rtmp_test".to_string(),
            "https://synctv.example.com".to_string(),
        );

        let source_config = json!({
            "stream_key": "test_stream",
            "room_id": "room123"
        });

        let ctx = ProviderContext::new("synctv");
        let result = provider.generate_playback(&ctx, &source_config).await.unwrap();

        assert_eq!(result.default_mode, "hls");
        assert!(result.playback_infos.contains_key("hls"));
        assert!(result.playback_infos.contains_key("flv"));

        let hls_info = &result.playback_infos["hls"];
        assert_eq!(hls_info.format, "m3u8");
        assert!(hls_info.urls[0].contains("index.m3u8"));
        assert!(hls_info.expires_at.is_none());

        assert_eq!(result.metadata["is_live"], json!(true));
    }

    #[tokio::test]
    async fn test_cache_key() {
        let provider = RtmpProvider::default();
        let ctx = ProviderContext::new("synctv");

        let source_config = json!({
            "stream_key": "test_stream",
            "room_id": "room123"
        });

        let key = provider.cache_key(&ctx, &source_config);
        assert!(key.contains("rtmp"));
        assert!(key.contains("room123"));
        assert!(key.contains("test_stream"));
        assert!(key.ends_with(":shared"));
    }

    #[tokio::test]
    async fn test_validate_source_config() {
        let provider = RtmpProvider::default();
        let ctx = ProviderContext::new("synctv");

        // Valid config
        let valid_config = json!({
            "stream_key": "test",
            "room_id": "room123"
        });
        assert!(provider.validate_source_config(&ctx, &valid_config).await.is_ok());

        // Missing stream_key
        let invalid_config = json!({
            "room_id": "room123"
        });
        assert!(provider.validate_source_config(&ctx, &invalid_config).await.is_err());

        // Missing room_id
        let invalid_config = json!({
            "stream_key": "test"
        });
        assert!(provider.validate_source_config(&ctx, &invalid_config).await.is_err());
    }
}
