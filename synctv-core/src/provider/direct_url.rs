//! Direct URL `MediaProvider`
//!
//! Provides direct playback for HTTP(S) URLs

use super::{MediaProvider, PlaybackInfo, PlaybackResult, ProviderContext, ProviderError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::IpAddr;

/// Direct URL `MediaProvider`
pub struct DirectUrlProvider {}

impl DirectUrlProvider {
    #[must_use] 
    pub const fn new() -> Self {
        Self {}
    }

    /// Validate that a URL does not target internal/private network addresses (SSRF protection).
    fn validate_url_not_internal(raw: &str) -> Result<(), ProviderError> {
        let parsed = url::Url::parse(raw)
            .map_err(|_| ProviderError::InvalidUrl("Failed to parse URL".to_string()))?;

        let host = parsed
            .host_str()
            .ok_or_else(|| ProviderError::InvalidUrl("URL has no host".to_string()))?;

        // Block well-known internal hostnames
        if matches!(
            host,
            "localhost" | "metadata.google.internal" | "instance-data"
        ) {
            return Err(ProviderError::InvalidUrl(
                "URLs targeting internal hosts are not allowed".to_string(),
            ));
        }

        // Block any hostname that resolves to a private/reserved IP
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_private_ip(ip) {
                return Err(ProviderError::InvalidUrl(
                    "URLs targeting private IP addresses are not allowed".to_string(),
                ));
            }
        }

        // Also check via the url crate's typed host (handles bracketed IPv6, etc.)
        if let Some(url::Host::Ipv4(ip)) = parsed.host() {
            if is_private_ip(IpAddr::V4(ip)) {
                return Err(ProviderError::InvalidUrl(
                    "URLs targeting private IP addresses are not allowed".to_string(),
                ));
            }
        }
        if let Some(url::Host::Ipv6(ip)) = parsed.host() {
            if is_private_ip(IpAddr::V6(ip)) {
                return Err(ProviderError::InvalidUrl(
                    "URLs targeting private IP addresses are not allowed".to_string(),
                ));
            }
        }

        Ok(())
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
        super::parse_source_config(value, "DirectUrl")
    }
}

/// Check if an IP address is in a private/reserved range.
const fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()           // 127.0.0.0/8
            || v4.is_private()         // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
            || v4.is_link_local()      // 169.254.0.0/16
            || v4.is_unspecified()     // 0.0.0.0
            || v4.is_multicast()       // 224.0.0.0/4
            || v4.is_broadcast()       // 255.255.255.255
            || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64  // 100.64.0.0/10 (CGNAT)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()           // ::1
            || v6.is_unspecified()     // ::
            || v6.is_multicast()       // ff00::/8
            // fe80::/10 (link-local)
            || (v6.segments()[0] & 0xffc0) == 0xfe80
            // fc00::/7 (unique local)
            || (v6.segments()[0] & 0xfe00) == 0xfc00
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

        // Validate URL scheme: only allow http(s) and rtmp(s)
        if !config.url.starts_with("http://")
            && !config.url.starts_with("https://")
            && !config.url.starts_with("rtmp://")
            && !config.url.starts_with("rtmps://")
        {
            return Err(ProviderError::InvalidConfig(
                "URL must use http, https, rtmp, or rtmps scheme".to_string(),
            ));
        }

        // SSRF protection: reject URLs targeting private/internal networks
        if config.url.starts_with("http://") || config.url.starts_with("https://") {
            Self::validate_url_not_internal(&config.url)?;
        }

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
    fn test_ssrf_blocks_localhost() {
        let result = DirectUrlProvider::validate_url_not_internal("http://localhost/secret");
        assert!(result.is_err());
    }

    #[test]
    fn test_ssrf_blocks_private_ipv4() {
        // 10.x.x.x
        assert!(DirectUrlProvider::validate_url_not_internal("http://10.0.0.1/path").is_err());
        // 172.16.x.x
        assert!(DirectUrlProvider::validate_url_not_internal("http://172.16.0.1/path").is_err());
        // 192.168.x.x
        assert!(DirectUrlProvider::validate_url_not_internal("http://192.168.1.1/path").is_err());
        // 127.x.x.x
        assert!(DirectUrlProvider::validate_url_not_internal("http://127.0.0.1/path").is_err());
        // 0.0.0.0
        assert!(DirectUrlProvider::validate_url_not_internal("http://0.0.0.0/path").is_err());
        // link-local
        assert!(
            DirectUrlProvider::validate_url_not_internal("http://169.254.169.254/latest/meta-data")
                .is_err()
        );
        // CGNAT
        assert!(DirectUrlProvider::validate_url_not_internal("http://100.64.0.1/path").is_err());
    }

    #[test]
    fn test_ssrf_blocks_metadata_endpoints() {
        assert!(
            DirectUrlProvider::validate_url_not_internal("http://metadata.google.internal/v1")
                .is_err()
        );
        assert!(
            DirectUrlProvider::validate_url_not_internal("http://instance-data/latest").is_err()
        );
    }

    #[test]
    fn test_ssrf_blocks_ipv6_loopback() {
        assert!(DirectUrlProvider::validate_url_not_internal("http://[::1]/path").is_err());
    }

    #[test]
    fn test_ssrf_allows_public_urls() {
        assert!(DirectUrlProvider::validate_url_not_internal("https://example.com/video.mp4").is_ok());
        assert!(
            DirectUrlProvider::validate_url_not_internal("https://cdn.example.com/stream.m3u8")
                .is_ok()
        );
        assert!(
            DirectUrlProvider::validate_url_not_internal("http://93.184.216.34/video.mp4").is_ok()
        );
    }

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
