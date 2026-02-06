//! Direct URL `MediaProvider`
//!
//! Provides direct playback for HTTP(S) URLs

use super::{MediaProvider, PlaybackInfo, PlaybackResult, ProviderContext, ProviderError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Direct URL `MediaProvider`
pub struct DirectUrlProvider {}

impl DirectUrlProvider {
    #[must_use] 
    pub const fn new() -> Self {
        Self {}
    }

    /// Detect format from URL
    fn detect_format(url: &str) -> String {
        if url.contains(".m3u8") || url.ends_with(".m3u8") {
            "m3u8"
        } else if url.contains(".flv") || url.ends_with(".flv") {
            "flv"
        } else if url.contains(".mp4") || url.ends_with(".mp4") {
            "mp4"
        } else if url.contains(".mkv") || url.ends_with(".mkv") {
            "mkv"
        } else if url.contains(".webm") || url.ends_with(".webm") {
            "webm"
        } else if url.contains(".avi") || url.ends_with(".avi") {
            "avi"
        } else {
            "video"
        }
        .to_string()
    }
}

impl Default for DirectUrlProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// `DirectUrl` source configuration
#[derive(Debug, Deserialize, Serialize)]
struct DirectUrlSourceConfig {
    url: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    proxy: bool,
}

impl TryFrom<&Value> for DirectUrlSourceConfig {
    type Error = ProviderError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        serde_json::from_value(value.clone()).map_err(|e| {
            ProviderError::InvalidConfig(format!("Failed to parse DirectUrl source config: {e}"))
        })
    }
}

#[async_trait]
impl MediaProvider for DirectUrlProvider {
    fn name(&self) -> &'static str {
        "direct_url"
    }

    async fn generate_playback(
        &self,
        _ctx: &ProviderContext<'_>,
        source_config: &Value,
    ) -> Result<PlaybackResult, ProviderError> {
        let config = DirectUrlSourceConfig::try_from(source_config)?;

        let format = Self::detect_format(&config.url);

        let mut playback_infos = HashMap::new();
        playback_infos.insert(
            "direct".to_string(),
            PlaybackInfo {
                urls: vec![config.url.clone()],
                format: format.clone(),
                headers: config.headers,
                subtitles: Vec::new(),
                expires_at: None,
            },
        );

        let mut metadata = HashMap::new();
        metadata.insert("format".to_string(), json!(format));
        metadata.insert("is_live".to_string(), json!(false));
        metadata.insert("proxy".to_string(), json!(config.proxy));

        // Extract filename from URL
        if let Some(filename) = config.url.split('/').next_back() {
            metadata.insert("filename".to_string(), json!(filename));
        }

        Ok(PlaybackResult {
            playback_infos,
            default_mode: "direct".to_string(),
            metadata,
            dash: None,
            hevc_dash: None,
        })
    }

    fn cache_key(&self, _ctx: &ProviderContext<'_>, source_config: &Value) -> String {
        if let Ok(config) = DirectUrlSourceConfig::try_from(source_config) {
            format!("direct_url:{:x}", md5::compute(config.url.as_bytes()))
        } else {
            "direct_url:unknown".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_format() {
        assert_eq!(
            DirectUrlProvider::detect_format("http://example.com/video.mp4"),
            "mp4"
        );
        assert_eq!(
            DirectUrlProvider::detect_format("http://example.com/stream.m3u8"),
            "m3u8"
        );
        assert_eq!(
            DirectUrlProvider::detect_format("http://example.com/stream.flv"),
            "flv"
        );
        assert_eq!(
            DirectUrlProvider::detect_format("http://example.com/video"),
            "video"
        );
    }
}
