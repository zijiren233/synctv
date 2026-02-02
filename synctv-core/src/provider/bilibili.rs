//! Bilibili MediaProvider Adapter
//!
//! Adapter that calls BilibiliClient to implement MediaProvider trait

use super::{
    provider_client::{
        create_remote_bilibili_client, load_local_bilibili_client, BilibiliClientArc,
    },
    MediaProvider, PlaybackInfo, PlaybackResult, ProviderContext, ProviderError, SubtitleTrack,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::service::ProviderInstanceManager;

/// Bilibili MediaProvider
///
/// Holds a reference to ProviderInstanceManager to select appropriate provider instance.
pub struct BilibiliProvider {
    provider_instance_manager: Arc<ProviderInstanceManager>,
}

/// Bilibili video info
#[derive(Debug, Clone, Serialize)]
pub struct BilibiliVideoInfo {
    pub bvid: String,
    pub cid: u64,
    pub epid: u64,
    pub name: String,
    pub cover_image: String,
    pub r#live: bool,
}

/// Bilibili page info response
#[derive(Debug, Clone, Serialize)]
pub struct BilibiliPageInfo {
    pub title: String,
    pub actors: Vec<String>,
    pub videos: Vec<BilibiliVideoInfo>,
}

impl BilibiliProvider {
    /// Create a new BilibiliProvider with ProviderInstanceManager
    pub fn new(provider_instance_manager: Arc<ProviderInstanceManager>) -> Self {
        Self {
            provider_instance_manager,
        }
    }

    /// Get Bilibili client for the given instance name
    ///
    /// Selection priority:
    /// 1. Instance specified by instance_name parameter
    /// 2. Fallback to singleton local client
    async fn get_client(&self, instance_name: Option<&str>) -> BilibiliClientArc {
        if let Some(name) = instance_name {
            if let Some(channel) = self.provider_instance_manager.get(name).await {
                // Remote instance - create gRPC client
                return create_remote_bilibili_client(channel);
            }
        }

        // Fallback to singleton local client
        load_local_bilibili_client()
    }

    // ========== Provider API Methods ==========

    /// Match URL to determine type and ID
    pub async fn r#match(
        &self,
        url: String,
        instance_name: Option<&str>,
    ) -> Result<synctv_providers::grpc::bilibili::MatchResp, ProviderError> {
        let client = self.get_client(instance_name).await;
        let req = synctv_providers::grpc::bilibili::MatchReq { url };
        client.r#match(req).await.map_err(|e| e.into())
    }

    /// Parse video page
    pub async fn parse_video_page(
        &self,
        req: synctv_providers::grpc::bilibili::ParseVideoPageReq,
        instance_name: Option<&str>,
    ) -> Result<synctv_providers::grpc::bilibili::VideoPageInfo, ProviderError> {
        let client = self.get_client(instance_name).await;
        client.parse_video_page(req).await.map_err(|e| e.into())
    }

    /// Parse PGC page
    pub async fn parse_pgc_page(
        &self,
        req: synctv_providers::grpc::bilibili::ParsePgcPageReq,
        instance_name: Option<&str>,
    ) -> Result<synctv_providers::grpc::bilibili::VideoPageInfo, ProviderError> {
        let client = self.get_client(instance_name).await;
        client.parse_pgc_page(req).await.map_err(|e| e.into())
    }

    /// Parse live page
    pub async fn parse_live_page(
        &self,
        req: synctv_providers::grpc::bilibili::ParseLivePageReq,
        instance_name: Option<&str>,
    ) -> Result<synctv_providers::grpc::bilibili::VideoPageInfo, ProviderError> {
        let client = self.get_client(instance_name).await;
        client.parse_live_page(req).await.map_err(|e| e.into())
    }

    /// Generate QR code for login
    pub async fn new_qr_code(
        &self,
        instance_name: Option<&str>,
    ) -> Result<synctv_providers::grpc::bilibili::NewQrCodeResp, ProviderError> {
        let client = self.get_client(instance_name).await;
        client
            .new_qr_code(synctv_providers::grpc::bilibili::Empty {})
            .await
            .map_err(|e| e.into())
    }

    /// Check QR code login status
    pub async fn login_with_qr_code(
        &self,
        req: synctv_providers::grpc::bilibili::LoginWithQrCodeReq,
        instance_name: Option<&str>,
    ) -> Result<synctv_providers::grpc::bilibili::LoginWithQrCodeResp, ProviderError> {
        let client = self.get_client(instance_name).await;
        client.login_with_qr_code(req).await.map_err(|e| e.into())
    }

    /// Get new captcha
    pub async fn new_captcha(
        &self,
        instance_name: Option<&str>,
    ) -> Result<synctv_providers::grpc::bilibili::NewCaptchaResp, ProviderError> {
        let client = self.get_client(instance_name).await;
        client
            .new_captcha(synctv_providers::grpc::bilibili::Empty {})
            .await
            .map_err(|e| e.into())
    }

    /// Send SMS verification code
    pub async fn new_sms(
        &self,
        req: synctv_providers::grpc::bilibili::NewSmsReq,
        instance_name: Option<&str>,
    ) -> Result<synctv_providers::grpc::bilibili::NewSmsResp, ProviderError> {
        let client = self.get_client(instance_name).await;
        client.new_sms(req).await.map_err(|e| e.into())
    }

    /// Login with SMS code
    pub async fn login_with_sms(
        &self,
        req: synctv_providers::grpc::bilibili::LoginWithSmsReq,
        instance_name: Option<&str>,
    ) -> Result<synctv_providers::grpc::bilibili::LoginWithSmsResp, ProviderError> {
        let client = self.get_client(instance_name).await;
        client.login_with_sms(req).await.map_err(|e| e.into())
    }

    /// Get user info
    pub async fn user_info(
        &self,
        req: synctv_providers::grpc::bilibili::UserInfoReq,
        instance_name: Option<&str>,
    ) -> Result<synctv_providers::grpc::bilibili::UserInfoResp, ProviderError> {
        let client = self.get_client(instance_name).await;
        client.user_info(req).await.map_err(|e| e.into())
    }
}

// Note: Default implementation removed as it requires ProviderInstanceManager

/// Bilibili source configuration structs
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BilibiliSourceConfig {
    Video {
        bvid: Option<String>,
        aid: Option<u64>,
        cid: u64,
        #[serde(default)]
        cookies: HashMap<String, String>,
        #[serde(default)]
        provider_instance_name: Option<String>,
    },
    Pgc {
        epid: u64,
        cid: u64,
        #[serde(default)]
        cookies: HashMap<String, String>,
        #[serde(default)]
        provider_instance_name: Option<String>,
    },
    Live {
        room_id: u64,
        #[serde(default)]
        cookies: HashMap<String, String>,
        #[serde(default)]
        provider_instance_name: Option<String>,
    },
}

impl BilibiliSourceConfig {
    /// Get provider_instance_name from any variant
    fn provider_instance_name(&self) -> Option<&str> {
        match self {
            Self::Video {
                provider_instance_name,
                ..
            } => provider_instance_name.as_deref(),
            Self::Pgc {
                provider_instance_name,
                ..
            } => provider_instance_name.as_deref(),
            Self::Live {
                provider_instance_name,
                ..
            } => provider_instance_name.as_deref(),
        }
    }
}

impl TryFrom<&Value> for BilibiliSourceConfig {
    type Error = ProviderError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        serde_json::from_value(value.clone()).map_err(|e| {
            ProviderError::InvalidConfig(format!("Failed to parse Bilibili source config: {}", e))
        })
    }
}

#[async_trait]
impl MediaProvider for BilibiliProvider {
    fn name(&self) -> &'static str {
        "bilibili"
    }

    async fn generate_playback(
        &self,
        _ctx: &ProviderContext<'_>,
        source_config: &Value,
    ) -> Result<PlaybackResult, ProviderError> {
        // Parse source_config first
        let config = BilibiliSourceConfig::try_from(source_config)?;

        // Get appropriate client based on instance_name from config
        let client = self.get_client(config.provider_instance_name()).await;

        match config {
            BilibiliSourceConfig::Video {
                bvid,
                aid,
                cid,
                cookies,
                ..
            } => {
                let bvid = bvid.unwrap_or_default();
                let aid = aid.unwrap_or(0);
                // Get DASH video URL
                let request = synctv_providers::grpc::bilibili::GetDashVideoUrlReq {
                    aid,
                    bvid: bvid.clone(),
                    cid,
                    cookies: cookies.clone(),
                };

                let dash_resp = client.get_dash_video_url(request).await?;

                let mut playback_infos = HashMap::new();
                let mut metadata = HashMap::new();

                // Add DASH playback info
                if let Some(dash) = dash_resp.dash {
                    let mut dash_urls = Vec::new();

                    // Add video streams
                    for video in &dash.video_streams {
                        dash_urls.push(video.base_url.clone());
                    }

                    // Add audio streams
                    for audio in &dash.audio_streams {
                        dash_urls.push(audio.base_url.clone());
                    }

                    playback_infos.insert(
                        "dash".to_string(),
                        PlaybackInfo {
                            urls: dash_urls,
                            format: "dash".to_string(),
                            headers: {
                                let mut headers = HashMap::new();
                                headers.insert(
                                    "Referer".to_string(),
                                    "https://www.bilibili.com".to_string(),
                                );
                                headers
                            },
                            subtitles: Vec::new(),
                            expires_at: None,
                        },
                    );

                    metadata.insert("duration".to_string(), json!(dash.duration));
                    metadata.insert("min_buffer_time".to_string(), json!(dash.min_buffer_time));
                }

                // Add HEVC DASH if available
                if let Some(hevc_dash) = dash_resp.hevc_dash {
                    let mut hevc_urls = Vec::new();

                    for video in &hevc_dash.video_streams {
                        hevc_urls.push(video.base_url.clone());
                    }

                    for audio in &hevc_dash.audio_streams {
                        hevc_urls.push(audio.base_url.clone());
                    }

                    if !hevc_urls.is_empty() {
                        playback_infos.insert(
                            "hevc_dash".to_string(),
                            PlaybackInfo {
                                urls: hevc_urls,
                                format: "dash".to_string(),
                                headers: {
                                    let mut headers = HashMap::new();
                                    headers.insert(
                                        "Referer".to_string(),
                                        "https://www.bilibili.com".to_string(),
                                    );
                                    headers
                                },
                                subtitles: Vec::new(),
                                expires_at: None,
                            },
                        );
                    }
                }

                // Get subtitles
                let subtitle_request = synctv_providers::grpc::bilibili::GetSubtitlesReq {
                    aid,
                    bvid: bvid.clone(),
                    cid,
                    cookies,
                };

                if let Ok(subtitle_resp) = client.get_subtitles(subtitle_request).await {
                    let subtitles: Vec<SubtitleTrack> = subtitle_resp
                        .subtitles
                        .into_iter()
                        .map(|(name, url)| SubtitleTrack {
                            language: name.clone(),
                            name,
                            url,
                            format: "json".to_string(), // Bilibili uses JSON subtitle format
                        })
                        .collect();

                    // Add subtitles to all playback modes
                    for playback in playback_infos.values_mut() {
                        playback.subtitles = subtitles.clone();
                    }
                }

                metadata.insert("content_type".to_string(), json!("video"));
                metadata.insert("bvid".to_string(), json!(bvid));
                metadata.insert("aid".to_string(), json!(aid));
                metadata.insert("cid".to_string(), json!(cid));

                Ok(PlaybackResult {
                    playback_infos,
                    default_mode: "dash".to_string(),
                    metadata,
                })
            }

            BilibiliSourceConfig::Pgc {
                epid, cid, cookies, ..
            } => {
                // Get PGC DASH URL
                let request = synctv_providers::grpc::bilibili::GetDashPgcurlReq {
                    epid,
                    cid,
                    cookies: cookies.clone(),
                };

                let dash_resp = client.get_dash_pgcurl(request).await?;

                let mut playback_infos = HashMap::new();
                let mut metadata = HashMap::new();

                // Add DASH playback info
                if let Some(dash) = dash_resp.dash {
                    let mut dash_urls = Vec::new();

                    for video in &dash.video_streams {
                        dash_urls.push(video.base_url.clone());
                    }

                    for audio in &dash.audio_streams {
                        dash_urls.push(audio.base_url.clone());
                    }

                    playback_infos.insert(
                        "dash".to_string(),
                        PlaybackInfo {
                            urls: dash_urls,
                            format: "dash".to_string(),
                            headers: {
                                let mut headers = HashMap::new();
                                headers.insert(
                                    "Referer".to_string(),
                                    "https://www.bilibili.com".to_string(),
                                );
                                headers
                            },
                            subtitles: Vec::new(),
                            expires_at: None,
                        },
                    );

                    metadata.insert("duration".to_string(), json!(dash.duration));
                }

                // Add HEVC DASH if available
                if let Some(hevc_dash) = dash_resp.hevc_dash {
                    let mut hevc_urls = Vec::new();

                    for video in &hevc_dash.video_streams {
                        hevc_urls.push(video.base_url.clone());
                    }

                    for audio in &hevc_dash.audio_streams {
                        hevc_urls.push(audio.base_url.clone());
                    }

                    if !hevc_urls.is_empty() {
                        playback_infos.insert(
                            "hevc_dash".to_string(),
                            PlaybackInfo {
                                urls: hevc_urls,
                                format: "dash".to_string(),
                                headers: {
                                    let mut headers = HashMap::new();
                                    headers.insert(
                                        "Referer".to_string(),
                                        "https://www.bilibili.com".to_string(),
                                    );
                                    headers
                                },
                                subtitles: Vec::new(),
                                expires_at: None,
                            },
                        );
                    }
                }

                metadata.insert("content_type".to_string(), json!("pgc"));
                metadata.insert("epid".to_string(), json!(epid));
                metadata.insert("cid".to_string(), json!(cid));

                Ok(PlaybackResult {
                    playback_infos,
                    default_mode: "dash".to_string(),
                    metadata,
                })
            }

            BilibiliSourceConfig::Live {
                room_id, cookies, ..
            } => {
                // Get live streams
                let request = synctv_providers::grpc::bilibili::GetLiveStreamsReq {
                    cid: room_id,
                    hls: true, // Request HLS streams
                    cookies,
                };

                let live_resp = client.get_live_streams(request).await?;

                let mut playback_infos = HashMap::new();
                let mut metadata = HashMap::new();

                // Group streams by quality
                for stream in live_resp.live_streams {
                    let quality_name = if !stream.desc.is_empty() {
                        stream.desc
                    } else {
                        format!("quality_{}", stream.quality)
                    };

                    playback_infos.insert(
                        quality_name.clone(),
                        PlaybackInfo {
                            urls: stream.urls,
                            format: "hls".to_string(),
                            headers: {
                                let mut headers = HashMap::new();
                                headers.insert(
                                    "Referer".to_string(),
                                    "https://live.bilibili.com".to_string(),
                                );
                                headers
                            },
                            subtitles: Vec::new(),
                            expires_at: None,
                        },
                    );
                }

                metadata.insert("content_type".to_string(), json!("live"));
                metadata.insert("room_id".to_string(), json!(room_id));
                metadata.insert("is_live".to_string(), json!(true));

                // Default to highest quality
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
        }
    }

    fn cache_key(&self, _ctx: &ProviderContext<'_>, source_config: &Value) -> String {
        format!("bilibili:{}", source_config)
    }
}
