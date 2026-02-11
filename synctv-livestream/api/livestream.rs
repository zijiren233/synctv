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
    relay::StreamRegistryTrait,
    livestream::{
        pull_manager::PullStreamManager,
        external_publish_manager::ExternalPublishManager,
        segment_manager::SegmentManager,
    },
    protocols::hls::remuxer::StreamRegistry as HlsStreamRegistry,
    protocols::httpflv::HttpFlvSession,
};
use anyhow::Result;
use bytes::Bytes;
use dashmap::DashMap;
use std::sync::Arc;
use synctv_xiu::streamhub::define::StreamHubEventSender;
use tokio::sync::mpsc;

/// RAII guard that decrements a stream's subscriber count on drop.
///
/// Hold this for the lifetime of a viewer connection:
/// - **FLV**: lives in the streaming task — dropped when the viewer disconnects
/// - **HLS**: dropped at the end of each request (transient touch of `last_active_time`)
///
/// The cleanup task in both managers checks `subscriber_count == 0 && idle > 5 min`
/// before tearing down the stream, so this guard is essential for correct lifecycle.
pub struct StreamSubscriberGuard(Option<Box<dyn FnOnce() + Send>>);

impl StreamSubscriberGuard {
    fn new(on_drop: impl FnOnce() + Send + 'static) -> Self {
        Self(Some(Box::new(on_drop)))
    }
}

impl Drop for StreamSubscriberGuard {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

/// Legacy type alias — prefer `StreamTracker` for new code.
pub type UserStreamTracker = Arc<StreamTracker>;

/// Tracks active RTMP publishers with five cross-referenced indexes
/// for fast lookup in any direction:
///
/// 1. `user_id → Set<(room_id, media_id)>` — kick all streams for a user (supports multiple)
/// 2. `room_id → Set<media_id>` — kick all streams in a room
/// 3. `(room_id, media_id) → user_id` — find who is publishing a specific stream
/// 4. `(rtmp_app_name, rtmp_stream_name) → (room_id, media_id)` — map RTMP identifiers to logical stream
/// 5. `(room_id, media_id) → (rtmp_app_name, rtmp_stream_name)` — reverse map for cleanup
///
/// The RTMP mapping is needed because `stream_name` in RTMP may be a JWT token,
/// not the `media_id`. On unpublish, we only know `(app_name, stream_name)` and
/// need to resolve the logical `(room_id, media_id)`.
///
/// All mutations atomically update all indexes.
/// A single user may publish to multiple rooms/media simultaneously.
pub struct StreamTracker {
    /// `user_id` → Set of "`room_id:media_id`" composite keys
    by_user: DashMap<String, dashmap::DashSet<String>>,
    /// `room_id` → Set<`media_id`>
    by_room: DashMap<String, dashmap::DashSet<String>>,
    /// "`room_id:media_id`" → `user_id`
    by_stream: DashMap<String, String>,
    /// "`app_name\0stream_name`" → "`room_id:media_id`" (RTMP→logical)
    by_rtmp: DashMap<String, String>,
    /// "`room_id:media_id`" → "`app_name\0stream_name`" (logical→RTMP, for cleanup)
    rtmp_reverse: DashMap<String, String>,
}

impl Default for StreamTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamTracker {
    #[must_use] 
    pub fn new() -> Self {
        Self {
            by_user: DashMap::new(),
            by_room: DashMap::new(),
            by_stream: DashMap::new(),
            by_rtmp: DashMap::new(),
            rtmp_reverse: DashMap::new(),
        }
    }

    fn stream_key(room_id: &str, media_id: &str) -> String {
        format!("{room_id}:{media_id}")
    }

    fn parse_stream_key(key: &str) -> Option<(String, String)> {
        key.split_once(':').map(|(r, m)| (r.to_string(), m.to_string()))
    }

    fn rtmp_key(app_name: &str, stream_name: &str) -> String {
        format!("{app_name}\0{stream_name}")
    }

    /// Register that `user_id` is publishing `(room_id, media_id)` via RTMP
    /// with the given `(rtmp_app_name, rtmp_stream_name)` identifiers.
    ///
    /// The RTMP mapping is essential because `rtmp_stream_name` is typically
    /// a JWT token, not the logical `media_id`.
    ///
    /// A user may publish to multiple streams simultaneously.
    pub fn insert(
        &self,
        user_id: String,
        room_id: String,
        media_id: String,
        rtmp_app_name: &str,
        rtmp_stream_name: &str,
    ) {
        let sk = Self::stream_key(&room_id, &media_id);
        let rk = Self::rtmp_key(rtmp_app_name, rtmp_stream_name);

        // If another user was publishing this exact stream, remove them first
        if let Some((_, old_user)) = self.by_stream.remove(&sk) {
            if old_user != user_id {
                if let Some(user_set) = self.by_user.get(&old_user) {
                    user_set.remove(&sk);
                    if user_set.is_empty() {
                        drop(user_set);
                        self.by_user.remove(&old_user);
                    }
                }
            }
        }

        // Clean up any old RTMP mapping for this stream
        if let Some((_, old_rk)) = self.rtmp_reverse.remove(&sk) {
            self.by_rtmp.remove(&old_rk);
        }

        self.by_user
            .entry(user_id.clone())
            .or_default()
            .insert(sk.clone());

        self.by_room
            .entry(room_id)
            .or_default()
            .insert(media_id);

        self.by_stream.insert(sk.clone(), user_id);
        self.by_rtmp.insert(rk.clone(), sk.clone());
        self.rtmp_reverse.insert(sk, rk);
    }

    /// Remove ALL tracking entries for a user. Returns list of `(room_id, media_id)`.
    #[must_use] 
    pub fn remove_user(&self, user_id: &str) -> Vec<(String, String)> {
        let mut removed = Vec::new();
        if let Some((_, keys)) = self.by_user.remove(user_id) {
            for key in keys.iter() {
                self.by_stream.remove(key.as_str());
                // Clean up RTMP mapping
                if let Some((_, rk)) = self.rtmp_reverse.remove(key.as_str()) {
                    self.by_rtmp.remove(&rk);
                }
                if let Some((room_id, media_id)) = Self::parse_stream_key(&key) {
                    if let Some(set) = self.by_room.get(&room_id) {
                        set.remove(&media_id);
                        if set.is_empty() {
                            drop(set);
                            self.by_room.remove(&room_id);
                        }
                    }
                    removed.push((room_id, media_id));
                }
            }
        }
        removed
    }

    /// Remove tracking by (`room_id`, `media_id`). Returns the `user_id` if present.
    #[must_use] 
    pub fn remove_stream(&self, room_id: &str, media_id: &str) -> Option<String> {
        let sk = Self::stream_key(room_id, media_id);
        if let Some((_, user_id)) = self.by_stream.remove(&sk) {
            // Clean up RTMP mapping
            if let Some((_, rk)) = self.rtmp_reverse.remove(&sk) {
                self.by_rtmp.remove(&rk);
            }
            if let Some(user_set) = self.by_user.get(&user_id) {
                user_set.remove(&sk);
                if user_set.is_empty() {
                    drop(user_set);
                    self.by_user.remove(&user_id);
                }
            }
            if let Some(set) = self.by_room.get(room_id) {
                set.remove(media_id);
                if set.is_empty() {
                    drop(set);
                    self.by_room.remove(room_id);
                }
            }
            Some(user_id)
        } else {
            None
        }
    }

    /// Remove by RTMP identifiers (`app_name`, `stream_name`) — used by `on_unpublish`.
    ///
    /// Uses the RTMP→logical mapping to resolve `(room_id, media_id)` from the
    /// RTMP identifiers, then removes all tracking entries.
    ///
    /// Returns `Some((user_id, room_id, media_id))` if found, `None` otherwise.
    pub fn remove_by_app_stream(&self, app_name: &str, stream_name: &str) -> Option<(String, String, String)> {
        let rk = Self::rtmp_key(app_name, stream_name);

        // Look up logical stream from RTMP mapping
        if let Some((_, sk)) = self.by_rtmp.remove(&rk) {
            self.rtmp_reverse.remove(&sk);
            if let Some((room_id, media_id)) = Self::parse_stream_key(&sk) {
                if let Some(user_id) = self.remove_stream_internal(&room_id, &media_id) {
                    tracing::debug!(
                        user_id = %user_id,
                        room_id = %room_id,
                        media_id = %media_id,
                        rtmp_app = %app_name,
                        "Removed publisher from tracker on unpublish (RTMP mapping)"
                    );
                    return Some((user_id, room_id, media_id));
                }
            }
        }

        // Fallback: try direct stream key match (app_name = room_id, stream_name = media_id)
        if let Some(user_id) = self.remove_stream(app_name, stream_name) {
            tracing::debug!(
                user_id = %user_id,
                room_id = %app_name,
                media_id = %stream_name,
                "Removed publisher from tracker on unpublish (direct match)"
            );
            return Some((user_id, app_name.to_string(), stream_name.to_string()));
        }

        None
    }

    /// Internal: remove stream without touching RTMP maps (already cleaned by caller).
    fn remove_stream_internal(&self, room_id: &str, media_id: &str) -> Option<String> {
        let sk = Self::stream_key(room_id, media_id);
        if let Some((_, user_id)) = self.by_stream.remove(&sk) {
            if let Some(user_set) = self.by_user.get(&user_id) {
                user_set.remove(&sk);
                if user_set.is_empty() {
                    drop(user_set);
                    self.by_user.remove(&user_id);
                }
            }
            if let Some(set) = self.by_room.get(room_id) {
                set.remove(media_id);
                if set.is_empty() {
                    drop(set);
                    self.by_room.remove(room_id);
                }
            }
            Some(user_id)
        } else {
            None
        }
    }

    /// Get all (`room_id`, `media_id`) pairs for a user.
    #[must_use] 
    pub fn get_user_streams(&self, user_id: &str) -> Vec<(String, String)> {
        self.by_user
            .get(user_id)
            .map(|set| {
                set.iter()
                    .filter_map(|key| Self::parse_stream_key(&key))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all `media_ids` currently publishing in a room.
    #[must_use] 
    pub fn get_room_streams(&self, room_id: &str) -> Vec<String> {
        self.by_room
            .get(room_id)
            .map(|set| set.iter().map(|e| e.clone()).collect())
            .unwrap_or_default()
    }

    /// Get `user_id` publishing a specific (`room_id`, `media_id`).
    #[must_use] 
    pub fn get_stream_user(&self, room_id: &str, media_id: &str) -> Option<String> {
        self.by_stream.get(&Self::stream_key(room_id, media_id)).map(|e| e.value().clone())
    }

    /// Iterate over all stream entries. Provides `("room_id:media_id", user_id)`.
    pub fn iter_streams(&self) -> impl Iterator<Item = dashmap::mapref::multiple::RefMulti<'_, String, String>> {
        self.by_stream.iter()
    }

    /// Number of tracked streams.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_stream.len()
    }

    /// Whether the tracker is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_stream.is_empty()
    }
}

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
    /// `StreamHub` event sender for subscribing to streams
    pub stream_hub_event_sender: StreamHubEventSender,
    /// Pull stream manager for gRPC relay (cross-node pull)
    pub pull_manager: Arc<PullStreamManager>,
    /// External publish manager for pull-to-publish streams (RTMP/HTTP-FLV sources)
    pub external_publish_manager: Arc<ExternalPublishManager>,
    /// Segment manager for HLS storage
    pub segment_manager: Option<Arc<SegmentManager>>,
    /// HLS stream registry for M3U8 generation
    pub hls_stream_registry: Option<HlsStreamRegistry>,
    /// Tracks active RTMP publishers by `user_id` for kick-on-ban
    pub user_stream_tracker: UserStreamTracker,
}

impl LiveStreamingInfrastructure {
    /// Create new live streaming infrastructure
    pub fn new(
        registry: Arc<dyn StreamRegistryTrait>,
        stream_hub_event_sender: StreamHubEventSender,
        pull_manager: Arc<PullStreamManager>,
        external_publish_manager: Arc<ExternalPublishManager>,
        user_stream_tracker: UserStreamTracker,
    ) -> Self {
        Self {
            registry,
            stream_hub_event_sender,
            pull_manager,
            external_publish_manager,
            segment_manager: None,
            hls_stream_registry: None,
            user_stream_tracker,
        }
    }

    /// Add HLS segment manager
    #[must_use] 
    pub fn with_segment_manager(mut self, segment_manager: Arc<SegmentManager>) -> Self {
        self.segment_manager = Some(segment_manager);
        self
    }

    /// Add HLS stream registry
    #[must_use]
    pub fn with_hls_stream_registry(mut self, hls_stream_registry: HlsStreamRegistry) -> Self {
        self.hls_stream_registry = Some(hls_stream_registry);
        self
    }

    /// Kick an active RTMP publisher, forcing their session to disconnect.
    ///
    /// Sends an `UnPublish` event through `StreamHub` which terminates the transceiver's data pipeline.
    /// The RTMP session naturally terminates when its `data_sender` channel closes.
    ///
    /// Returns Ok(()) if the event was sent. The actual disconnection is asynchronous.
    pub fn kick_publisher(&self, room_id: &str, media_id: &str) -> Result<()> {
        use synctv_xiu::streamhub::stream::StreamIdentifier;

        let identifier = StreamIdentifier::Rtmp {
            app_name: room_id.to_string(),
            stream_name: media_id.to_string(),
        };

        self.stream_hub_event_sender
            .send(synctv_xiu::streamhub::define::StreamHubEvent::UnPublish { identifier })
            .map_err(|_| anyhow::anyhow!("Failed to send unpublish event (StreamHub not running)"))?;

        Ok(())
    }

    /// Kick all active RTMP publishers for a given user.
    ///
    /// Looks up all of the user's active streams from the tracker and sends `UnPublish` events.
    /// Used when banning or deleting a user to terminate all their RTMP publish sessions.
    pub fn kick_user_publishers(&self, user_id: &str) {
        let streams = self.user_stream_tracker.remove_user(user_id);
        for (room_id, media_id) in streams {
            tracing::info!(
                user_id = %user_id,
                room_id = %room_id,
                media_id = %media_id,
                "Kicking RTMP publisher for banned user"
            );
            if let Err(e) = self.kick_publisher(&room_id, &media_id) {
                tracing::error!("Failed to kick publisher for user {}: {}", user_id, e);
            }
        }
    }

    /// Kick all active RTMP publishers in a given room.
    ///
    /// Uses the room→media index for O(1) lookup instead of scanning all entries.
    /// Used when banning or deleting a room.
    pub fn kick_room_publishers(&self, room_id: &str) {
        let media_ids = self.user_stream_tracker.get_room_streams(room_id);

        for media_id in media_ids {
            if let Some(user_id) = self.user_stream_tracker.remove_stream(room_id, &media_id) {
                tracing::info!(
                    user_id = %user_id,
                    room_id = %room_id,
                    media_id = %media_id,
                    "Kicking RTMP publisher for banned room"
                );
            }
            if let Err(e) = self.kick_publisher(room_id, &media_id) {
                tracing::error!("Failed to kick publisher in room {}: {}", room_id, e);
            }
        }
    }

    /// Kick a specific stream by room_id and media_id.
    ///
    /// Removes the publisher from Redis and sends an UnPublish event.
    pub async fn kick_stream(&self, room_id: &str, media_id: &str) -> Result<()> {
        // Remove from Redis registry
        self.registry
            .unregister_publisher(room_id, media_id)
            .await?;

        // Remove from local tracker
        let _ = self.user_stream_tracker.remove_stream(room_id, media_id);

        // Send UnPublish to StreamHub
        self.kick_publisher(room_id, media_id)?;

        Ok(())
    }

    /// Ensure a pull stream exists for the given room/media.
    ///
    /// Unified entry point that handles both gRPC relay and external pull:
    /// 1. If a publisher exists in Redis → gRPC relay (cross-node)
    /// 2. If no publisher + `external_source_url` provided → external pull (lazy start)
    /// 3. If no publisher + no URL → error
    ///
    /// Returns a [`StreamSubscriberGuard`] that decrements the subscriber count
    /// when dropped. For FLV, hold it in the streaming task; for HLS, let it
    /// drop at the end of the request (the `last_active_time` touch keeps the
    /// stream alive across polling intervals).
    pub async fn ensure_pull_stream(
        &self,
        room_id: &str,
        media_id: &str,
        external_source_url: Option<&str>,
    ) -> Result<StreamSubscriberGuard> {
        // Check Redis for an existing publisher
        let publisher = self.registry.get_publisher(room_id, media_id).await
            .map_err(|e| anyhow::anyhow!("Failed to check publisher: {e}"))?;

        if publisher.is_some() {
            // Publisher found in Redis — create gRPC relay pull stream
            let stream = self.pull_manager
                .get_or_create_pull_stream(room_id, media_id)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to create pull stream: {e}"))?;
            let guard = StreamSubscriberGuard::new(move || stream.decrement_subscriber_count());
            return Ok(guard);
        }

        // No publisher in Redis — try external publish if URL provided
        if let Some(source_url) = external_source_url {
            let stream = self.external_publish_manager
                .get_or_create(room_id, media_id, source_url)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to create external publish stream: {e}"))?;
            let guard = StreamSubscriberGuard::new(move || stream.decrement_subscriber_count());
            return Ok(guard);
        }

        Err(anyhow::anyhow!("No publisher found for {room_id}/{media_id}"))
    }

    /// Get the registry (for admin queries)
    pub fn registry(&self) -> &Arc<dyn StreamRegistryTrait> {
        &self.registry
    }

    /// Check if publisher exists for a room/media
    pub async fn has_publisher(&self, room_id: &str, media_id: &str) -> Result<bool> {
        self.registry
            .get_publisher(room_id, media_id)
            .await
            .map(|opt| opt.is_some())
            .map_err(|e| anyhow::anyhow!("Failed to check publisher: {e}"))
    }

    /// Get publisher info
    pub async fn get_publisher(&self, room_id: &str, media_id: &str) -> Result<crate::relay::PublisherInfo> {
        self.registry
            .get_publisher(room_id, media_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("No publisher found for {room_id}/{media_id}"))
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
            .ok_or_else(|| anyhow::anyhow!("No publisher for {room_id}/{media_id}"))?;

        // Create channel for FLV data
        let (tx, rx) = mpsc::unbounded_channel();

        // Create FLV session
        let stream_name = format!("{room_id}/{media_id}");
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
    /// Supports both cross-node gRPC relay and external source pulling.
    ///
    /// Returns `(receiver, guard)`. The caller **must** hold the
    /// [`StreamSubscriberGuard`] for the lifetime of the FLV streaming task
    /// so the subscriber count is decremented when the viewer disconnects.
    ///
    /// # Arguments
    /// * `external_source_url` - If provided and no Redis publisher exists, starts an
    ///   external pull from this URL (RTMP or HTTP-FLV).
    pub async fn create_session_with_pull(
        infrastructure: &LiveStreamingInfrastructure,
        room_id: &str,
        media_id: &str,
        external_source_url: Option<&str>,
    ) -> Result<(mpsc::UnboundedReceiver<Result<Bytes, std::io::Error>>, StreamSubscriberGuard)> {
        // Ensure pull stream exists (gRPC relay or external)
        let guard = infrastructure.ensure_pull_stream(room_id, media_id, external_source_url).await?;

        // Create FLV session (subscribes to local StreamHub)
        let rx = Self::create_session(infrastructure, room_id, media_id).await?;
        Ok((rx, guard))
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
            .ok_or_else(|| anyhow::anyhow!("No publisher for {room_id}/{media_id}"))?;

        // Generate from HLS stream registry
        if let Some(hls_registry) = &infrastructure.hls_stream_registry {
            // Registry key format: "live/room_id:media_id" (same as remuxer uses)
            let stream_key = format!("live/{room_id}:{media_id}");

            let playlist = hls_registry.get(&stream_key).map_or_else(|| {
                    // Empty playlist if stream not in registry yet
                    "#EXTM3U\n\
                         #EXT-X-VERSION:3\n\
                         #EXT-X-TARGETDURATION:10\n\
                         #EXT-X-MEDIA-SEQUENCE:0\n".to_string()
                }, |stream_state| {
                    let state = stream_state.read();
                    // Use caller-provided URL generator for maximum flexibility
                    state.generate_m3u8(url_generator)
                });

            Ok(playlist)
        } else {
            // Fallback: empty playlist
            Ok("#EXTM3U\n\
                 #EXT-X-VERSION:3\n\
                 #EXT-X-TARGETDURATION:10\n\
                 #EXT-X-MEDIA-SEQUENCE:0\n".to_string())
        }
    }

    /// Generate HLS M3U8 playlist with simple base URL (convenience method)
    ///
    /// # Arguments
    /// * `infrastructure` - Live streaming infrastructure
    /// * `room_id` - Room identifier
    /// * `media_id` - Media/stream identifier
    /// * `segment_url_base` - Base URL for segment links (e.g., "/`api/rooms/{room_id}/live/hls/segments`/")
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
            format!("{segment_url_base}{ts_name}.ts")
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
            .ok_or_else(|| anyhow::anyhow!("No publisher for {room_id}/{media_id}"))?;

        // Get segment from storage
        if let Some(segment_manager) = &infrastructure.segment_manager {
            // Build storage key: app_name-stream_name-ts_name
            // stream_name format is "room_id:media_id", replace : with - for flat key
            let stream_name = format!("{room_id}:{media_id}");
            let storage_key = format!("live-{}-{}", stream_name.replace(':', "-"), segment_name);

            segment_manager
                .storage()
                .read(&storage_key)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to read segment: {e}"))
        } else {
            Err(anyhow::anyhow!("Segment manager not configured"))
        }
    }

    /// Generate HLS M3U8 playlist with lazy-load pull and custom URL generator
    ///
    /// This ensures a pull stream is created if one doesn't exist.
    /// Supports both cross-node gRPC relay and external source pulling.
    ///
    /// # Arguments
    /// * `infrastructure` - Live streaming infrastructure
    /// * `room_id` - Room identifier
    /// * `media_id` - Media/stream identifier
    /// * `external_source_url` - If provided and no Redis publisher exists, starts an
    ///   external pull from this URL.
    /// * `url_generator` - Closure that generates segment URLs (allows auth tokens, CDN, etc)
    pub async fn generate_playlist_with_pull<F>(
        infrastructure: &LiveStreamingInfrastructure,
        room_id: &str,
        media_id: &str,
        external_source_url: Option<&str>,
        url_generator: F,
    ) -> Result<String>
    where
        F: Fn(&str) -> String,
    {
        // Ensure pull stream exists (gRPC relay or external).
        // Guard is dropped at end of this function — for HLS this is intentional:
        // each polling request transiently touches last_active_time, keeping the
        // stream alive as long as the viewer keeps requesting playlists.
        let _guard = infrastructure.ensure_pull_stream(room_id, media_id, external_source_url).await?;

        Self::generate_playlist(infrastructure, room_id, media_id, url_generator).await
    }

    /// Generate HLS M3U8 playlist with lazy-load pull and simple base URL (convenience method)
    ///
    /// This ensures a pull stream is created if one doesn't exist.
    /// Supports both cross-node gRPC relay and external source pulling.
    ///
    /// # Arguments
    /// * `infrastructure` - Live streaming infrastructure
    /// * `room_id` - Room identifier
    /// * `media_id` - Media/stream identifier
    /// * `external_source_url` - If provided and no Redis publisher exists, starts an
    ///   external pull from this URL.
    /// * `segment_url_base` - Base URL for segment links (e.g., "/`api/rooms/{room_id}/live/hls/segments`/")
    pub async fn generate_playlist_with_pull_simple(
        infrastructure: &LiveStreamingInfrastructure,
        room_id: &str,
        media_id: &str,
        external_source_url: Option<&str>,
        segment_url_base: &str,
    ) -> Result<String> {
        Self::generate_playlist_with_pull(infrastructure, room_id, media_id, external_source_url, |ts_name| {
            format!("{segment_url_base}{ts_name}.ts")
        }).await
    }
}
