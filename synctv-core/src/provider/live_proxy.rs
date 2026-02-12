//! `LiveProxy` `MediaProvider`
//!
//! Provides playback URLs for live streams sourced from external URLs.
//! The external source URL is stored in `source_config`, and playback URLs
//! point to synctv's own HTTP-FLV and HLS endpoints (same as `RtmpProvider`).
//!
//! The `PullStreamManager` handles the actual pulling from the external source.

use super::{
    MediaProvider, PlaybackInfo, PlaybackResult, ProviderContext, ProviderError,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::IpAddr;

/// `LiveProxy` `MediaProvider`
///
/// Generates playback URLs for live streams from external sources.
/// The external URL is stored in `source_config.url` and validated on creation.
/// Playback URLs point to synctv's own HLS/FLV endpoints.
pub struct LiveProxyProvider {
    base_url: String,
}

impl LiveProxyProvider {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }
}

impl Default for LiveProxyProvider {
    fn default() -> Self {
        Self::new("https://localhost:8080")
    }
}

#[async_trait]
impl MediaProvider for LiveProxyProvider {
    fn name(&self) -> &'static str {
        "live_proxy"
    }

    async fn generate_playback(
        &self,
        _ctx: &ProviderContext<'_>,
        source_config: &Value,
    ) -> Result<PlaybackResult, ProviderError> {
        let media_id = source_config
            .get("media_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::InvalidConfig("Missing media_id".to_string()))?;

        let room_id = source_config
            .get("room_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::InvalidConfig("Missing room_id".to_string()))?;

        let source_url = source_config
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::InvalidConfig("Missing url".to_string()))?;

        let mut playback_infos = HashMap::new();

        // HLS URL — matches actual HTTP route
        playback_infos.insert(
            "hls".to_string(),
            PlaybackInfo {
                urls: vec![format!(
                    "{}/api/room/movie/live/hls/list/{}?room_id={}",
                    self.base_url, media_id, room_id
                )],
                format: "m3u8".to_string(),
                headers: HashMap::new(),
                subtitles: Vec::new(),
                expires_at: None,
            },
        );

        // FLV URL — matches actual HTTP route
        playback_infos.insert(
            "flv".to_string(),
            PlaybackInfo {
                urls: vec![format!(
                    "{}/api/room/movie/live/flv/{}.flv?room_id={}",
                    self.base_url, media_id, room_id
                )],
                format: "flv".to_string(),
                headers: HashMap::new(),
                subtitles: Vec::new(),
                expires_at: None,
            },
        );

        let mut metadata = HashMap::new();
        metadata.insert("is_live".to_string(), json!(true));
        metadata.insert("media_id".to_string(), json!(media_id));
        metadata.insert("room_id".to_string(), json!(room_id));
        metadata.insert("source_url".to_string(), json!(source_url));
        metadata.insert("provider".to_string(), json!("live_proxy"));

        Ok(PlaybackResult {
            playback_infos,
            default_mode: "hls".to_string(),
            metadata,
            dash: None,
            hevc_dash: None,
        })
    }

    async fn validate_source_config(
        &self,
        _ctx: &ProviderContext<'_>,
        source_config: &Value,
    ) -> Result<(), ProviderError> {
        // Validate required fields
        let url = source_config
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::InvalidConfig("Missing url".to_string()))?;

        source_config
            .get("room_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::InvalidConfig("Missing room_id".to_string()))?;

        source_config
            .get("media_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::InvalidConfig("Missing media_id".to_string()))?;

        // Validate URL format (only RTMP and HTTP-FLV are supported for pulling)
        if !url.starts_with("rtmp://")
            && !url.ends_with(".flv")
            && !url.contains(".flv?")
        {
            return Err(ProviderError::InvalidConfig(format!(
                "Unsupported source URL format: {url}. Expected rtmp:// or *.flv"
            )));
        }

        // SSRF protection: validate the host is not a private/internal address
        validate_source_url_host(url)?;

        Ok(())
    }

    fn cache_key(&self, _ctx: &ProviderContext<'_>, source_config: &Value) -> String {
        let room_id = source_config
            .get("room_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let media_id = source_config
            .get("media_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        format!("live_proxy:{room_id}:{media_id}")
    }
}

/// Validate that a source URL's host is not a private/internal address (SSRF protection).
///
/// Supports `rtmp://`, `http://`, and `https://` schemes. Strips `rtmp://` prefix and
/// parses the host portion to check against private IP ranges and well-known internal hostnames.
fn validate_source_url_host(raw: &str) -> Result<(), ProviderError> {
    // For RTMP URLs, extract host from rtmp://host:port/app/stream format
    let host_str = if let Some(rest) = raw.strip_prefix("rtmp://") {
        // Take everything before the first '/' after the host:port
        let authority = rest.split('/').next().unwrap_or(rest);
        // Strip port if present
        if let Some((host, _port)) = authority.rsplit_once(':') {
            host
        } else {
            authority
        }
    } else if let Ok(parsed) = url::Url::parse(raw) {
        // For HTTP(S) URLs, use url crate
        return match parsed.host_str() {
            Some(host) => check_host_not_internal(host),
            None => Err(ProviderError::InvalidConfig("URL has no host".to_string())),
        };
    } else {
        return Err(ProviderError::InvalidConfig(format!("Cannot parse URL: {raw}")));
    };

    check_host_not_internal(host_str)
}

fn check_host_not_internal(host: &str) -> Result<(), ProviderError> {
    // Block well-known internal hostnames
    if matches!(
        host,
        "localhost" | "metadata.google.internal" | "instance-data"
    ) {
        return Err(ProviderError::InvalidConfig(
            "Source URL targets an internal host".to_string(),
        ));
    }

    // Check IP addresses against private ranges
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_ip(ip) {
            return Err(ProviderError::InvalidConfig(
                "Source URL targets a private IP address".to_string(),
            ));
        }
    }

    Ok(())
}

/// Check if an IP address is in a private/reserved range.
const fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
            || v4.is_private()
            || v4.is_link_local()
            || v4.is_unspecified()
            || v4.is_multicast()
            || v4.is_broadcast()
            || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64 // 100.64.0.0/10 (CGNAT)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
            || v6.is_unspecified()
            || v6.is_multicast()
            || (v6.segments()[0] & 0xffc0) == 0xfe80 // link-local
            || (v6.segments()[0] & 0xfe00) == 0xfc00  // unique local
            // IPv4-mapped IPv6 (::ffff:x.x.x.x) - check the embedded IPv4
            || {
                let segs = v6.segments();
                if segs[0] == 0 && segs[1] == 0 && segs[2] == 0 && segs[3] == 0
                    && segs[4] == 0 && segs[5] == 0xffff
                {
                    let o = v6.octets();
                    let v4 = std::net::Ipv4Addr::new(o[12], o[13], o[14], o[15]);
                    is_private_ip(IpAddr::V4(v4))
                } else {
                    false
                }
            }
        }
    }
}
