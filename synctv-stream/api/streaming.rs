// Live streaming API abstractions for synctv-api integration
//
// This module provides flexible APIs and abstractions for implementing
// live streaming HTTP endpoints in synctv-api.
//
// Architecture:
// - synctv-stream provides infrastructure + abstractions (this module)
// - synctv-api implements HTTP endpoints using these abstractions
//
// Features:
// - Lazy-load FLV streaming (create pull streams on demand)
// - HLS streaming with M3U8 playlist generation
// - GOP cache for instant playback
// - Publisher/Puller architecture
// - Cross-node gRPC relay

use crate::{
    libraries::gop_cache::GopCache,
    relay::StreamRegistryTrait,
    streaming::{
        pull_manager::PullStreamManager,
        segment_manager::SegmentManager,
        protocols::hls::remuxer::StreamRegistry as HlsStreamRegistry,
        protocols::httpflv::HttpFlvSession,
    },
    error::StreamResult,
};
use anyhow::Result;
use bytes::Bytes;
use std::sync::Arc;
use streamhub::define::StreamHubEventSender;
use tokio::sync::mpsc;

/// Live streaming infrastructure bundle
///
/// Provides all necessary components for implementing live streaming endpoints:
/// - FLV streaming sessions
/// - HLS playlist generation
/// - HLS segment serving
/// - Publisher discovery
/// - GOP cache access
#[derive(Clone)]
pub struct LiveStreamingInfrastructure {
    /// Registry for finding publishers (Redis)
    pub registry: Arc<dyn StreamRegistryTrait>,
    /// StreamHub event sender for subscribing to streams
    pub stream_hub_event_sender: StreamHubEventSender,
    /// GOP cache for instant playback
    pub gop_cache: Arc<GopCache>,
    /// Pull stream manager for lazy-load streaming
    pub pull_manager: Arc<PullStreamManager>,
    /// Segment manager for HLS storage
    pub segment_manager: Option<Arc<SegmentManager>>,
    /// HLS stream registry for M3U8 generation
    pub hls_stream_registry: Option<HlsStreamRegistry>,
}

impl LiveStreamingInfrastructure {
    /// Create new live streaming infrastructure
    pub fn new(
        registry: Arc<dyn StreamRegistryTrait>,
        stream_hub_event_sender: StreamHubEventSender,
        gop_cache: Arc<GopCache>,
        pull_manager: Arc<PullStreamManager>,
    ) -> Self {
        Self {
            registry,
            stream_hub_event_sender,
            gop_cache,
            pull_manager,
            segment_manager: None,
            hls_stream_registry: None,
        }
    }

    /// Add HLS segment manager
    pub fn with_segment_manager(mut self, segment_manager: Arc<SegmentManager>) -> Self {
        self.segment_manager = Some(segment_manager);
        self
    }

    /// Add HLS stream registry
    pub fn with_hls_stream_registry(mut self, hls_stream_registry: HlsStreamRegistry) -> Self {
        self.hls_stream_registry = Some(hls_stream_registry);
        self
    }

    /// Check if publisher exists for a room/media
    pub async fn has_publisher(&self, room_id: &str, media_id: &str) -> Result<bool> {
        self.registry
            .get_publisher(room_id, media_id)
            .await
            .map(|opt| opt.is_some())
            .map_err(|e| anyhow::anyhow!("Failed to check publisher: {}", e))
    }

    /// Get publisher info
    pub async fn get_publisher(&self, room_id: &str, media_id: &str) -> Result<crate::relay::PublisherInfo> {
        self.registry
            .get_publisher(room_id, media_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("No publisher found for {}/{}", room_id, media_id))
    }
}

/// FLV streaming API
///
/// Provides methods for creating FLV streaming sessions
pub struct FlvStreamingApi;

impl FlvStreamingApi {
    /// Create a new FLV streaming session
    ///
    /// Returns a channel receiver that streams FLV data.
    /// The caller is responsible for converting this to an HTTP response.
    ///
    /// # Arguments
    /// * `infrastructure` - Live streaming infrastructure
    /// * `room_id` - Room identifier
    /// * `media_id` - Media/stream identifier
    ///
    /// # Returns
    /// A channel receiver that yields FLV data chunks
    ///
    /// # Example
    /// ```ignore
    /// let rx = FlvStreamingApi::create_session(infrastructure, "room123", "media456").await?;
    /// let body = Body::from_stream(UnboundedReceiverStream::new(rx));
    /// ```
    pub async fn create_session(
        infrastructure: &LiveStreamingInfrastructure,
        room_id: &str,
        media_id: &str,
    ) -> Result<mpsc::UnboundedReceiver<Result<Bytes, std::io::Error>>> {
        // Ensure publisher exists
        infrastructure.has_publisher(room_id, media_id).await?
            .then_some(())
            .ok_or_else(|| anyhow::anyhow!("No publisher for {}/{}", room_id, media_id))?;

        // Create channel for FLV data
        let (tx, rx) = mpsc::unbounded_channel();

        // Create FLV session
        let stream_name = format!("{}/{}", room_id, media_id);
        let mut flv_session = HttpFlvSession::new(
            "live".to_string(),
            stream_name,
            infrastructure.stream_hub_event_sender.clone(),
            tx,
        );

        // Spawn FLV session task
        tokio::spawn(async move {
            if let Err(e) = flv_session.run().await {
                tracing::error!("FLV session error: {}", e);
            }
        });

        Ok(rx)
    }

    /// Create FLV streaming session with lazy-load pull
    ///
    /// This ensures a pull stream is created if one doesn't exist.
    /// Useful for cross-node scenarios where the publisher is on a different server.
    pub async fn create_session_with_pull(
        infrastructure: &LiveStreamingInfrastructure,
        room_id: &str,
        media_id: &str,
    ) -> Result<mpsc::UnboundedReceiver<Result<Bytes, std::io::Error>>> {
        // Ensure publisher exists
        infrastructure.get_publisher(room_id, media_id).await?;

        // Lazy-load: Create pull stream if needed
        let _pull_stream = infrastructure
            .pull_manager
            .get_or_create_pull_stream(room_id, media_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create pull stream: {}", e))?;

        // Create FLV session
        Self::create_session(infrastructure, room_id, media_id).await
    }
}

/// HLS streaming API
///
/// Provides methods for HLS playlist generation and segment serving
pub struct HlsStreamingApi;

impl HlsStreamingApi {
    /// Generate HLS M3U8 playlist for a stream
    ///
    /// # Arguments
    /// * `infrastructure` - Live streaming infrastructure
    /// * `room_id` - Room identifier
    /// * `media_id` - Media/stream identifier
    /// * `url_generator` - Closure that generates segment URLs (allows auth tokens, CDN, etc)
    ///
    /// # Returns
    /// M3U8 playlist content as a string
    ///
    /// # Example
    /// ```ignore
    /// // Simple URL format
    /// let playlist = HlsStreamingApi::generate_playlist(
    ///     infrastructure,
    ///     "room123",
    ///     "media456",
    ///     |ts_name| format!("/api/rooms/{}/live/hls/segments/{}.ts", room_id, ts_name)
    /// ).await?;
    ///
    /// // With authentication token
    /// let playlist = HlsStreamingApi::generate_playlist(
    ///     infrastructure,
    ///     "room123",
    ///     "media456",
    ///     |ts_name| format!("/api/rooms/{}/live/hls/segments/{}.ts?token={}", room_id, ts_name, token)
    /// ).await?;
    ///
    /// // With CDN URL
    /// let playlist = HlsStreamingApi::generate_playlist(
    ///     infrastructure,
    ///     "room123",
    ///     "media456",
    ///     |ts_name| format!("https://cdn.example.com/hls/{}.ts", ts_name)
    /// ).await?;
    /// ```
    pub async fn generate_playlist<F>(
        infrastructure: &LiveStreamingInfrastructure,
        room_id: &str,
        media_id: &str,
        url_generator: F,
    ) -> Result<String>
    where
        F: Fn(&str) -> String,
    {
        // Ensure publisher exists
        infrastructure.has_publisher(room_id, media_id).await?
            .then_some(())
            .ok_or_else(|| anyhow::anyhow!("No publisher for {}/{}", room_id, media_id))?;

        // Generate from HLS stream registry
        if let Some(hls_registry) = &infrastructure.hls_stream_registry {
            // Registry key format: "live/room_id:media_id" (same as remuxer uses)
            let stream_key = format!("live/{}:{}", room_id, media_id);

            let playlist = hls_registry.get(&stream_key)
                .map(|stream_state| {
                    let state = stream_state.read();
                    // Use caller-provided URL generator for maximum flexibility
                    state.generate_m3u8(url_generator)
                })
                .unwrap_or_else(|| {
                    // Empty playlist if stream not in registry yet
                    format!(
                        "#EXTM3U\n\
                         #EXT-X-VERSION:3\n\
                         #EXT-X-TARGETDURATION:10\n\
                         #EXT-X-MEDIA-SEQUENCE:0\n"
                    )
                });

            Ok(playlist)
        } else {
            // Fallback: empty playlist
            Ok(format!(
                "#EXTM3U\n\
                 #EXT-X-VERSION:3\n\
                 #EXT-X-TARGETDURATION:10\n\
                 #EXT-X-MEDIA-SEQUENCE:0\n"
            ))
        }
    }

    /// Generate HLS M3U8 playlist with simple base URL (convenience method)
    ///
    /// # Arguments
    /// * `infrastructure` - Live streaming infrastructure
    /// * `room_id` - Room identifier
    /// * `media_id` - Media/stream identifier
    /// * `segment_url_base` - Base URL for segment links (e.g., "/api/rooms/{room_id}/live/hls/segments/")
    ///
    /// # Returns
    /// M3U8 playlist content as a string
    pub async fn generate_playlist_simple(
        infrastructure: &LiveStreamingInfrastructure,
        room_id: &str,
        media_id: &str,
        segment_url_base: &str,
    ) -> Result<String> {
        Self::generate_playlist(infrastructure, room_id, media_id, |ts_name| {
            format!("{}{}.ts", segment_url_base, ts_name)
        }).await
    }

    /// Get HLS segment data
    ///
    /// # Arguments
    /// * `infrastructure` - Live streaming infrastructure
    /// * `room_id` - Room identifier
    /// * `media_id` - Media/stream identifier
    /// * `segment_name` - TS segment name (e.g., "a1b2c3d4e5f6")
    ///
    /// # Returns
    /// Segment data as bytes
    pub async fn get_segment(
        infrastructure: &LiveStreamingInfrastructure,
        room_id: &str,
        media_id: &str,
        segment_name: &str,
    ) -> Result<Bytes> {
        // Ensure publisher exists
        infrastructure.has_publisher(room_id, media_id).await?
            .then_some(())
            .ok_or_else(|| anyhow::anyhow!("No publisher for {}/{}", room_id, media_id))?;

        // Get segment from storage
        if let Some(segment_manager) = &infrastructure.segment_manager {
            // Build storage key: app_name-stream_name-ts_name
            let stream_name = format!("{}:{}", room_id, media_id);
            let storage_key = format!("live-{}-{}", stream_name.replace(':', "-"), segment_name);

            segment_manager
                .storage()
                .read(&storage_key)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to read segment: {}", e))
        } else {
            Err(anyhow::anyhow!("Segment manager not configured"))
        }
    }

    /// Generate HLS M3U8 playlist with lazy-load pull and custom URL generator
    ///
    /// This ensures a pull stream is created if one doesn't exist.
    ///
    /// # Arguments
    /// * `infrastructure` - Live streaming infrastructure
    /// * `room_id` - Room identifier
    /// * `media_id` - Media/stream identifier
    /// * `url_generator` - Closure that generates segment URLs (allows auth tokens, CDN, etc)
    pub async fn generate_playlist_with_pull<F>(
        infrastructure: &LiveStreamingInfrastructure,
        room_id: &str,
        media_id: &str,
        url_generator: F,
    ) -> Result<String>
    where
        F: Fn(&str) -> String,
    {
        // Ensure publisher exists (this also triggers pull stream creation)
        infrastructure.get_publisher(room_id, media_id).await?;

        Self::generate_playlist(infrastructure, room_id, media_id, url_generator).await
    }

    /// Generate HLS M3U8 playlist with lazy-load pull and simple base URL (convenience method)
    ///
    /// This ensures a pull stream is created if one doesn't exist.
    ///
    /// # Arguments
    /// * `infrastructure` - Live streaming infrastructure
    /// * `room_id` - Room identifier
    /// * `media_id` - Media/stream identifier
    /// * `segment_url_base` - Base URL for segment links (e.g., "/api/rooms/{room_id}/live/hls/segments/")
    pub async fn generate_playlist_with_pull_simple(
        infrastructure: &LiveStreamingInfrastructure,
        room_id: &str,
        media_id: &str,
        segment_url_base: &str,
    ) -> Result<String> {
        Self::generate_playlist_with_pull(infrastructure, room_id, media_id, |ts_name| {
            format!("{}{}.ts", segment_url_base, ts_name)
        }).await
    }
}

/// Re-exports for convenience
pub use {
    FlvStreamingApi,
    HlsStreamingApi,
    LiveStreamingInfrastructure,
};
