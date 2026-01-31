// Media Provider Traits
//
// Core interfaces for the provider system

use super::{ProviderContext, ProviderError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Video quality option
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityOption {
    /// Quality name (e.g., "1080P", "720P")
    pub name: String,
    /// Quality code for provider API
    pub code: String,
    /// Bitrate in kbps (optional)
    pub bitrate: Option<u32>,
}

/// Subtitle track
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleTrack {
    /// Language code (e.g., "zh-CN", "en-US")
    pub language: String,
    /// Subtitle name
    pub name: String,
    /// Subtitle URL
    pub url: String,
    /// Format (srt, vtt, ass)
    pub format: String,
}

/// Playback information for a single mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackInfo {
    /// Video URLs (supports adaptive streaming with multiple URLs)
    pub urls: Vec<String>,

    /// Video format (mp4, m3u8, flv, mpd)
    pub format: String,

    /// HTTP headers required for playback
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// Available subtitle tracks
    #[serde(default)]
    pub subtitles: Vec<SubtitleTrack>,

    /// URL expiration time (Unix timestamp in seconds, optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

/// Complete playback result with multiple modes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackResult {
    /// Multiple playback modes (e.g., "direct", "proxied", "high", "low")
    pub playback_infos: HashMap<String, PlaybackInfo>,

    /// Default playback mode to use
    pub default_mode: String,

    /// Additional metadata (duration, thumbnail, etc.)
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

/// Directory item (file or folder)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryItem {
    /// Item name
    pub name: String,

    /// Item type
    pub item_type: ItemType,

    /// Full path
    pub path: String,

    /// File size in bytes (for files)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,

    /// Thumbnail URL (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,

    /// Modified time (Unix timestamp)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<i64>,
}

/// Item type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemType {
    File,
    Folder,
    Video,
}

/// Media provider trait
///
/// Core interface that all providers must implement.
/// Only `generate_playback()` is mandatory.
///
/// Note: MediaProvider is a capability provider, not a concrete instance.
/// It may use different provider_instances internally via ProviderInstanceManager.
#[async_trait]
pub trait MediaProvider: Send + Sync {
    // ========== Basic Information ==========

    /// Provider type name (e.g., "bilibili", "alist", "emby")
    fn name(&self) -> &'static str;

    // ========== Core Method (MANDATORY) ==========

    /// Generate playback information from source_config
    ///
    /// This is the ONLY mandatory method. Called when user plays media.
    ///
    /// # Flow
    /// 1. Read media from database (includes source_config)
    /// 2. Call generate_playback(source_config)
    /// 3. Return PlaybackResult to client
    ///
    /// # Caching
    /// Results are cached in Redis based on `cache_key()`
    ///
    /// # Returns
    /// PlaybackResult with multiple modes:
    /// - "direct": Direct URLs from provider API
    /// - "proxied": URLs proxied through SyncTV server
    /// - Custom modes: Provider-specific (e.g., "cdn1", "cdn2")
    ///
    /// # Example
    /// ```rust
    /// // Bilibili: source_config = {"bvid": "BV1xx", "cid": 123, "prefer_proxy": false}
    /// // Returns: {
    /// //   playback_infos: {"direct": {...}, "proxied": {...}},
    /// //   default_mode: "direct"
    /// // }
    /// ```
    async fn generate_playback(
        &self,
        ctx: &ProviderContext<'_>,
        source_config: &Value,
    ) -> Result<PlaybackResult, ProviderError>;

    // Note: Service/Route registration is handled in synctv-api layer
    // via extension traits to avoid circular dependencies.
    // See synctv-api/src/http/provider_extensions.rs
    // See synctv-api/src/grpc/provider_extensions.rs

    // ========== Caching Strategy ==========

    /// Generate cache key for playback result
    ///
    /// Default implementation supports shared vs user-level caching.
    ///
    /// # Returns
    /// - Shared: "synctv:playback:{provider}:{hash}:shared"
    /// - User: "synctv:playback:{provider}:{hash}:user:{user_id}"
    fn cache_key(&self, ctx: &ProviderContext<'_>, source_config: &Value) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        source_config.to_string().hash(&mut hasher);
        let config_hash = hasher.finish();

        let is_shared = source_config
            .get("shared")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if is_shared {
            format!(
                "{}:playback:{}:{}:shared",
                ctx.key_prefix,
                self.name(),
                config_hash
            )
        } else if let Some(user_id) = ctx.user_id {
            format!(
                "{}:playback:{}:{}:user:{}",
                ctx.key_prefix,
                self.name(),
                config_hash,
                user_id
            )
        } else {
            format!(
                "{}:playback:{}:{}:anonymous",
                ctx.key_prefix,
                self.name(),
                config_hash
            )
        }
    }

    // ========== Validation ==========

    /// Validate source_config before saving to database
    ///
    /// Called when user adds media via add_media API.
    ///
    /// # Flow
    /// 1. User calls parse endpoint → gets ParseResult
    /// 2. Client constructs source_config from ParseResult
    /// 3. Client calls add_media API with source_config
    /// 4. Server calls validate_source_config()
    /// 5. If valid, save to database
    async fn validate_source_config(
        &self,
        _ctx: &ProviderContext<'_>,
        _source_config: &Value,
    ) -> Result<(), ProviderError> {
        Ok(()) // Default: no validation
    }

    // ========== Lifecycle Hooks (Optional) ==========

    /// Called when playback starts
    ///
    /// Use cases:
    /// - Emby: Notify server to start transcoding
    /// - Statistics: Record playback event
    async fn on_playback_start(
        &self,
        _ctx: &ProviderContext<'_>,
        _session_id: &str,
        _source_config: &Value,
    ) -> Result<(), ProviderError> {
        Ok(()) // Default: no-op
    }

    /// Called when playback stops
    ///
    /// Use cases:
    /// - Emby: Notify server to stop transcoding
    /// - Statistics: Record watch duration
    async fn on_playback_stop(
        &self,
        _ctx: &ProviderContext<'_>,
        _session_id: &str,
        _source_config: &Value,
        _position: f64,
    ) -> Result<(), ProviderError> {
        Ok(()) // Default: no-op
    }

    /// Called periodically during playback (every 10s)
    ///
    /// Use cases:
    /// - Emby: Update playback progress on server
    /// - Statistics: Track viewing progress
    async fn on_playback_progress(
        &self,
        _ctx: &ProviderContext<'_>,
        _session_id: &str,
        _source_config: &Value,
        _position: f64,
    ) -> Result<(), ProviderError> {
        Ok(()) // Default: no-op
    }
}

/// Optional trait for providers that support directory browsing
///
/// Implemented by: Alist, Emby
/// Not implemented by: Bilibili, DirectUrl, RTMP
#[async_trait]
pub trait DynamicFolder: MediaProvider {
    /// List directory contents
    ///
    /// Returns files and folders that can be added to playlist.
    async fn list_directory(
        &self,
        ctx: &ProviderContext<'_>,
        path: Option<&str>,
        page: usize,
        page_size: usize,
    ) -> Result<Vec<DirectoryItem>, ProviderError>;

    /// Search within provider
    async fn search(
        &self,
        ctx: &ProviderContext<'_>,
        keyword: &str,
        page: usize,
        page_size: usize,
    ) -> Result<Vec<DirectoryItem>, ProviderError>;
}
