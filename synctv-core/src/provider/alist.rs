//! Alist MediaProvider Adapter
//!
//! Adapter that calls AlistVendorClient to implement MediaProvider trait.
//! VendorClient abstracts local/remote implementation, so MediaProvider doesn't need to know.

use super::{
    provider_client::{AlistClientArc, AlistClientExt, AlistFileInfo},
    MediaProvider, PlaybackInfo, PlaybackResult, ProviderCapabilities, ProviderContext,
    ProviderError, SubtitleTrack,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Alist MediaProvider
///
/// Uses AlistClient trait (abstraction over local/remote) to implement playback.
/// The client can be either LocalAlistClient or RemoteAlistClient.
pub struct AlistProvider {
    instance_id: String,
    base_url: String,
    client: AlistClientArc,
}

impl AlistProvider {
    /// Create a new AlistProvider with client
    ///
    /// The client implements AlistClient trait, which can be either:
    /// - LocalAlistClient (direct HTTP calls)
    /// - RemoteAlistClient (gRPC calls)
    ///
    /// MediaProvider doesn't need to know which implementation is used.
    pub fn new(
        instance_id: impl Into<String>,
        base_url: impl Into<String>,
        client: AlistClientArc,
    ) -> Self {
        Self {
            instance_id: instance_id.into(),
            base_url: base_url.into(),
            client,
        }
    }

    /// Legacy constructor name for compatibility
    pub fn new_with_manager(
        instance_id: impl Into<String>,
        base_url: impl Into<String>,
        client: AlistClientArc,
    ) -> Self {
        Self::new(instance_id, base_url, client)
    }

    /// Detect file format from extension
    fn detect_format(filename: &str) -> String {
        let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
        match ext.as_str() {
            "mp4" | "m4v" | "mov" => "mp4",
            "mkv" => "mkv",
            "avi" => "avi",
            "flv" => "flv",
            "webm" => "webm",
            "m3u8" => "hls",
            _ => "video",
        }
        .to_string()
    }
}

/// Alist source configuration
#[derive(Debug, Deserialize, Serialize)]
struct AlistSourceConfig {
    host: String,
    token: String,
    path: String,
    #[serde(default)]
    password: Option<String>,
}

impl TryFrom<&Value> for AlistSourceConfig {
    type Error = ProviderError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        serde_json::from_value(value.clone())
            .map_err(|e| ProviderError::InvalidConfig(format!("Failed to parse Alist source config: {}", e)))
    }
}

// Note: Default implementation removed as it requires ProviderInstanceManager

#[async_trait]
impl MediaProvider for AlistProvider {
    fn name(&self) -> &'static str {
        "alist"
    }

    fn instance_id(&self) -> &str {
        &self.instance_id
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            can_parse: true,
            can_play: true,
            supports_subtitles: true,
            supports_quality: true,
            requires_auth: true,
        }
    }

    async fn generate_playback(
        &self,
        _ctx: &ProviderContext<'_>,
        source_config: &Value,
    ) -> Result<PlaybackResult, ProviderError> {
        // Parse source_config
        let config = AlistSourceConfig::try_from(source_config)?;

        // Build proto request
        let request = synctv_providers::grpc::alist::FsGetReq {
            host: config.host.clone(),
            token: config.token.clone(),
            path: config.path.clone(),
            password: config.password.clone().unwrap_or_default(),
            user_agent: String::new(),
        };

        // Call client (trait method - implementation handles local/remote)
        let fs_get_data = self.client.fs_get(request).await?;

        let file_info: AlistFileInfo = fs_get_data.into();

        if file_info.is_dir {
            return Err(ProviderError::UnsupportedFormat(
                "Cannot play directory, use browse() instead".to_string(),
            ));
        }

        let mut playback_infos = HashMap::new();
        let mut metadata = HashMap::new();

        // Add basic metadata
        metadata.insert("name".to_string(), json!(file_info.name));
        metadata.insert("size".to_string(), json!(file_info.size));
        metadata.insert("provider".to_string(), json!(file_info.provider));
        if !file_info.thumb.is_empty() {
            metadata.insert("thumbnail".to_string(), json!(file_info.thumb));
        }

        // Try to get video preview info for transcoded URLs (optional)
        let has_video_preview = if let Some(preview) = self
            .client
            .get_video_preview(&config.host, &config.token, &config.path, config.password.as_deref())
            .await?
        {
            // Add transcoding quality options
            for (idx, task) in preview.transcoding_tasks.iter().enumerate() {
                if !task.url.is_empty() {
                    let quality_name = if !task.template_name.is_empty() {
                        task.template_name.clone()
                    } else {
                        format!("quality_{}", idx)
                    };

                    playback_infos.insert(
                        format!("transcoded_{}", quality_name),
                        PlaybackInfo {
                            urls: vec![task.url.clone()],
                            format: "hls".to_string(),
                            headers: HashMap::new(),
                            subtitles: preview
                                .subtitle_tasks
                                .iter()
                                .map(|sub| SubtitleTrack {
                                    language: sub.language.clone(),
                                    name: sub.language.clone(),
                                    url: sub.url.clone(),
                                    format: "srt".to_string(),
                                })
                                .collect(),
                            expires_at: None,
                        },
                    );
                }
            }

            // Add video metadata
            metadata.insert("duration".to_string(), json!(preview.duration));
            metadata.insert("width".to_string(), json!(preview.width));
            metadata.insert("height".to_string(), json!(preview.height));

            true
        } else {
            false
        };

        // Always add direct URL (raw_url) as fallback
        if !file_info.raw_url.is_empty() {
            playback_infos.insert(
                "direct".to_string(),
                PlaybackInfo {
                    urls: vec![file_info.raw_url.clone()],
                    format: Self::detect_format(&file_info.name),
                    headers: HashMap::new(),
                    subtitles: Vec::new(),
                    expires_at: None,
                },
            );
        }

        // Determine default mode
        let default_mode = if has_video_preview && !playback_infos.is_empty() {
            playback_infos
                .keys()
                .find(|k| k.starts_with("transcoded_"))
                .cloned()
                .unwrap_or_else(|| "direct".to_string())
        } else {
            "direct".to_string()
        };

        Ok(PlaybackResult {
            playback_infos,
            default_mode,
            metadata,
        })
    }

    fn cache_key(&self, _ctx: &ProviderContext<'_>, source_config: &Value) -> String {
        // Cache key includes user-specific path (Alist requires per-user credentials)
        if let Ok(config) = AlistSourceConfig::try_from(source_config) {
            let host_hash = format!("{:x}", md5::compute(config.host.as_bytes()));
            format!("alist:{}:{}", host_hash, config.path)
        } else {
            "alist:unknown".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Provider creation tests removed as they require ProviderClient setup

    #[test]
    fn test_detect_format() {
        assert_eq!(AlistProvider::detect_format("video.mp4"), "mp4");
        assert_eq!(AlistProvider::detect_format("video.mkv"), "mkv");
        assert_eq!(AlistProvider::detect_format("video.m3u8"), "hls");
        assert_eq!(AlistProvider::detect_format("video.unknown"), "video");
    }
}
