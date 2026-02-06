//! RTMP `MediaProvider`
//!
//! Provides playback URLs for RTMP live streams

use super::{
    MediaProvider, PlaybackInfo, PlaybackResult, ProviderContext, ProviderError,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

/// RTMP `MediaProvider`
pub struct RtmpProvider {
    base_url: String,
}

impl RtmpProvider {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }
}

impl Default for RtmpProvider {
    fn default() -> Self {
        Self::new("https://localhost:8080")
    }
}

#[async_trait]
impl MediaProvider for RtmpProvider {
    fn name(&self) -> &'static str {
        "rtmp"
    }

    async fn generate_playback(
        &self,
        _ctx: &ProviderContext<'_>,
        source_config: &Value,
    ) -> Result<PlaybackResult, ProviderError> {
        let stream_key = source_config
            .get("stream_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::InvalidConfig("Missing stream_key".to_string()))?;

        let room_id = source_config
            .get("room_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::InvalidConfig("Missing room_id".to_string()))?;

        let mut playback_infos = HashMap::new();

        // HLS URL
        playback_infos.insert(
            "hls".to_string(),
            PlaybackInfo {
                urls: vec![format!("{}/live/{}/{}/index.m3u8", self.base_url, room_id, stream_key)],
                format: "m3u8".to_string(),
                headers: HashMap::new(),
                subtitles: Vec::new(),
                expires_at: None,
            },
        );

        // FLV URL
        playback_infos.insert(
            "flv".to_string(),
            PlaybackInfo {
                urls: vec![format!("{}/live/{}/{}.flv", self.base_url, room_id, stream_key)],
                format: "flv".to_string(),
                headers: HashMap::new(),
                subtitles: Vec::new(),
                expires_at: None,
            },
        );

        let mut metadata = HashMap::new();
        metadata.insert("is_live".to_string(), json!(true));
        metadata.insert("stream_key".to_string(), json!(stream_key));
        metadata.insert("room_id".to_string(), json!(room_id));

        Ok(PlaybackResult {
            playback_infos,
            default_mode: "hls".to_string(),
            metadata,
            dash: None,
            hevc_dash: None,
        })
    }

    fn cache_key(&self, _ctx: &ProviderContext<'_>, source_config: &Value) -> String {
        let room_id = source_config
            .get("room_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        format!("rtmp:{room_id}")
    }
}
