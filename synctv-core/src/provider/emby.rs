//! Emby/Jellyfin MediaProvider Adapter
//!
//! Adapter that calls EmbyClient to implement MediaProvider trait

use super::{
    provider_client::{create_remote_emby_client, load_local_emby_client, EmbyClientArc},
    MediaProvider, PlaybackInfo, PlaybackResult, ProviderContext, ProviderError, SubtitleTrack,
};
use crate::service::ProviderInstanceManager;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// Emby MediaProvider
///
/// Holds a reference to ProviderInstanceManager to select appropriate provider instance.
pub struct EmbyProvider {
    provider_instance_manager: Arc<ProviderInstanceManager>,
}

impl EmbyProvider {
    /// Create a new EmbyProvider with ProviderInstanceManager
    pub fn new(provider_instance_manager: Arc<ProviderInstanceManager>) -> Self {
        Self {
            provider_instance_manager,
        }
    }

    /// Get Emby client for the given instance name
    ///
    /// Selection priority:
    /// 1. Instance specified by instance_name parameter
    /// 2. Fallback to singleton local client
    async fn get_client(&self, instance_name: Option<&str>) -> EmbyClientArc {
        if let Some(name) = instance_name {
            if let Some(channel) = self.provider_instance_manager.get(name).await {
                // Remote instance - create gRPC client
                return create_remote_emby_client(channel);
            }
        }

        // Fallback to singleton local client
        load_local_emby_client()
    }
}

/// Emby source configuration
#[derive(Debug, Deserialize, Serialize)]
struct EmbySourceConfig {
    host: String,
    token: String,
    user_id: String,
    item_id: String,
    #[serde(default)]
    provider_instance_name: Option<String>,
}

impl TryFrom<&Value> for EmbySourceConfig {
    type Error = ProviderError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        serde_json::from_value(value.clone()).map_err(|e| {
            ProviderError::InvalidConfig(format!("Failed to parse Emby source config: {}", e))
        })
    }
}

// Note: Default implementation removed as it requires ProviderInstanceManager

#[async_trait]
impl MediaProvider for EmbyProvider {
    fn name(&self) -> &'static str {
        "emby"
    }

    async fn generate_playback(
        &self,
        _ctx: &ProviderContext<'_>,
        source_config: &Value,
    ) -> Result<PlaybackResult, ProviderError> {
        // Parse source_config first
        let config = EmbySourceConfig::try_from(source_config)?;

        // Get appropriate client based on instance_name from config
        let client = self
            .get_client(config.provider_instance_name.as_deref())
            .await;

        // Get item details first
        let item_request = synctv_providers::grpc::emby::GetItemReq {
            host: config.host.clone(),
            token: config.token.clone(),
            item_id: config.item_id.clone(),
        };

        let item = client.get_item(item_request).await?;

        let mut metadata = HashMap::new();
        metadata.insert("name".to_string(), json!(item.name));
        metadata.insert("type".to_string(), json!(item.r#type));
        if !item.series_name.is_empty() {
            metadata.insert("series_name".to_string(), json!(item.series_name));
        }
        if !item.season_name.is_empty() {
            metadata.insert("season_name".to_string(), json!(item.season_name));
        }

        // Get playback info
        let playback_request = synctv_providers::grpc::emby::PlaybackInfoReq {
            host: config.host.clone(),
            token: config.token.clone(),
            user_id: config.user_id.clone(),
            item_id: config.item_id.clone(),
            media_source_id: String::new(), // Use default media source
            audio_stream_index: 0,
            subtitle_stream_index: 0,
            max_streaming_bitrate: 0, // No limit
        };

        let playback_info = client.playback_info(playback_request).await?;

        let mut playback_infos = HashMap::new();

        // Process media sources
        for (idx, source) in playback_info.media_source_info.iter().enumerate() {
            let mode_name = if !source.name.is_empty() {
                source.name.clone()
            } else {
                format!("source_{}", idx)
            };

            // Get direct stream URL (no transcoding)
            let direct_url = if !source.direct_play_url.is_empty() {
                format!(
                    "{}{}",
                    config.host.trim_end_matches('/'),
                    source.direct_play_url
                )
            } else if !source.path.is_empty() {
                // Build direct play URL
                format!(
                    "{}/Items/{}/Download?api_key={}",
                    config.host.trim_end_matches('/'),
                    config.item_id,
                    config.token
                )
            } else {
                continue;
            };

            // Extract subtitles
            let subtitles: Vec<SubtitleTrack> = source
                .media_stream_info
                .iter()
                .filter(|stream| stream.r#type == "Subtitle")
                .map(|stream| {
                    let subtitle_url = format!(
                        "{}/Videos/{}/{}/Subtitles/{}/Stream.{}?api_key={}",
                        config.host.trim_end_matches('/'),
                        config.item_id,
                        source.id,
                        stream.index,
                        stream.codec.to_lowercase(),
                        config.token
                    );

                    SubtitleTrack {
                        language: stream.language.clone(),
                        name: stream.display_title.clone(),
                        url: subtitle_url,
                        format: stream.codec.to_lowercase(),
                    }
                })
                .collect();

            // Detect format from container
            let format = source.container.to_lowercase();
            let format = if format.contains("mp4") || format == "m4v" {
                "mp4"
            } else if format.contains("mkv") {
                "mkv"
            } else if format.contains("webm") {
                "webm"
            } else if format.contains("m3u8") || format == "hls" {
                "hls"
            } else {
                "video"
            }
            .to_string();

            playback_infos.insert(
                mode_name.clone(),
                PlaybackInfo {
                    urls: vec![direct_url],
                    format,
                    headers: HashMap::new(),
                    subtitles,
                    expires_at: None,
                },
            );

            // Also add transcode URLs if available
            if !source.transcoding_url.is_empty() {
                let transcode_url = format!(
                    "{}{}",
                    config.host.trim_end_matches('/'),
                    source.transcoding_url
                );

                playback_infos.insert(
                    format!("{}_transcode", mode_name),
                    PlaybackInfo {
                        urls: vec![transcode_url],
                        format: "hls".to_string(), // Emby transcodes to HLS
                        headers: HashMap::new(),
                        subtitles: Vec::new(), // Subtitles burned in for transcode
                        expires_at: None,
                    },
                );
            }
        }

        // Default to first media source
        let default_mode = playback_infos
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| "direct".to_string());

        Ok(PlaybackResult {
            playback_infos,
            default_mode,
            metadata,
        })
    }

    fn cache_key(&self, _ctx: &ProviderContext<'_>, source_config: &Value) -> String {
        format!("emby:{}", source_config)
    }
}
