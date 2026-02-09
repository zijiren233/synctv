// Custom HLS remuxer using xiu's libraries but our storage abstraction
//
// Architecture:
// - Uses xiu's FlvVideoTagDemuxer/FlvAudioTagDemuxer for FLV parsing
// - Uses xiu's TsMuxer for TS segment generation
// - Uses our HlsStorage trait for segment/playlist storage
// - Generates M3U8 dynamically in memory, no file writes
//
// Based on xiu's implementation but with storage abstraction

use crate::livestream::segment_manager::SegmentManager;
use bytes::BytesMut;
use dashmap::DashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;
use streamhub::{
    define::{
        BroadcastEvent, BroadcastEventReceiver, FrameData, FrameDataReceiver,
        NotifyInfo, StreamHubEvent, StreamHubEventSender, SubscribeType, SubscriberInfo,
    },
    stream::StreamIdentifier,
    utils::{RandomDigitCount, Uuid},
};
use tokio::sync::mpsc;
use tracing as log;
use xflv::{
    define::{frame_type, FlvData},
    demuxer::{FlvAudioTagDemuxer, FlvVideoTagDemuxer},
};
use xmpegts::{
    define::{epsi_stream_type, MPEG_FLAG_IDR_FRAME},
    ts::TsMuxer,
};

/// Segment metadata for M3U8 generation
#[derive(Debug, Clone)]
pub struct SegmentInfo {
    /// Segment sequence number
    pub sequence: u64,
    /// Segment duration in milliseconds
    pub duration: i64,
    /// TS filename (nanoid, e.g., "a1b2c3d4e5f6")
    pub ts_name: String,
    /// Storage key (e.g., "`live/room_123/a1b2c3d4e5f6.ts`")
    pub storage_key: String,
    /// Whether this is a discontinuity point
    pub discontinuity: bool,
    /// Creation time (for cleanup)
    pub created_at: Instant,
}

/// Registry of active streams (for M3U8 generation)
pub type StreamRegistry = Arc<DashMap<String, Arc<parking_lot::RwLock<StreamProcessorState>>>>;

/// Stream processor state that can be accessed by HTTP server
pub struct StreamProcessorState {
    pub app_name: String,
    pub stream_name: String,
    pub segments: VecDeque<SegmentInfo>,
    pub is_ended: bool,
}

impl StreamProcessorState {
    /// Generate M3U8 content dynamically with custom TS URL generator
    ///
    /// # Arguments
    /// * `gen_ts_url` - Closure that takes TS name and returns full URL (can add auth tokens)
    ///
    /// # Example
    /// ```ignore
    /// let m3u8 = state.generate_m3u8(|ts_name| {
    ///     format!("/api/room/live/hls/data/{}/{}/{}?token={}", room_id, movie_id, ts_name, token)
    /// });
    /// ```
    pub fn generate_m3u8<F>(&self, mut gen_ts_url: F) -> String
    where
        F: FnMut(&str) -> String,
    {
        let mut m3u8_content = String::new();

        // Header
        m3u8_content.push_str("#EXTM3U\n");
        m3u8_content.push_str("#EXT-X-VERSION:3\n");

        // Target duration (max segment duration in seconds, rounded up)
        let max_duration_sec = self.segments
            .iter()
            .map(|s| (s.duration + 999) / 1000)
            .max()
            .unwrap_or(10);
        m3u8_content.push_str(&format!("#EXT-X-TARGETDURATION:{max_duration_sec}\n"));

        // Media sequence (first segment in playlist)
        let first_seq = self.segments.front().map_or(0, |s| s.sequence);
        m3u8_content.push_str(&format!("#EXT-X-MEDIA-SEQUENCE:{first_seq}\n"));

        // Segments
        for segment in &self.segments {
            if segment.discontinuity {
                m3u8_content.push_str("#EXT-X-DISCONTINUITY\n");
            }

            let duration_sec = segment.duration as f64 / 1000.0;
            m3u8_content.push_str(&format!("#EXTINF:{duration_sec:.3},\n"));

            // Use closure to generate segment URL (allows custom auth, CDN URLs, etc)
            let segment_url = gen_ts_url(&segment.ts_name);
            m3u8_content.push_str(&format!("{segment_url}\n"));
        }

        if self.is_ended {
            m3u8_content.push_str("#EXT-X-ENDLIST\n");
        }

        m3u8_content
    }
}

/// Custom HLS remuxer with storage abstraction
pub struct CustomHlsRemuxer {
    /// Event receiver from `StreamHub`
    client_event_consumer: BroadcastEventReceiver,
    /// Event sender to `StreamHub`
    event_producer: StreamHubEventSender,
    /// Segment manager with storage backend
    segment_manager: Arc<SegmentManager>,
    /// Stream registry for M3U8 generation
    stream_registry: StreamRegistry,
}

impl CustomHlsRemuxer {
    #[must_use] 
    pub const fn new(
        consumer: BroadcastEventReceiver,
        event_producer: StreamHubEventSender,
        segment_manager: Arc<SegmentManager>,
        stream_registry: StreamRegistry,
    ) -> Self {
        Self {
            client_event_consumer: consumer,
            event_producer,
            segment_manager,
            stream_registry,
        }
    }

    pub async fn run(&mut self) -> Result<(), HlsRemuxerError> {
        log::info!("Custom HLS remuxer started");

        loop {
            let val = self.client_event_consumer.recv().await?;
            match val {
                BroadcastEvent::Publish { identifier } => {
                    if let StreamIdentifier::Rtmp {
                        app_name,
                        stream_name,
                    } = identifier
                    {
                        log::info!("HLS remuxer: new stream {}/{}", app_name, stream_name);

                        let stream_handler = StreamHandler::new(
                            app_name,
                            stream_name,
                            self.event_producer.clone(),
                            Arc::clone(&self.segment_manager),
                            self.stream_registry.clone(),
                        );

                        tokio::spawn(async move {
                            if let Err(e) = stream_handler.run().await {
                                log::error!("HLS stream handler error: {}", e);
                            }
                        });
                    }
                }
                _ => {
                    log::trace!("HLS remuxer: other event");
                }
            }
        }
    }
}

/// Handler for a single HLS stream
struct StreamHandler {
    app_name: String,
    stream_name: String,
    event_producer: StreamHubEventSender,
    segment_manager: Arc<SegmentManager>,
    stream_registry: StreamRegistry,
    data_consumer: FrameDataReceiver,
    subscriber_id: Uuid,
}

impl StreamHandler {
    fn new(
        app_name: String,
        stream_name: String,
        event_producer: StreamHubEventSender,
        segment_manager: Arc<SegmentManager>,
        stream_registry: StreamRegistry,
    ) -> Self {
        let (_, data_consumer) = mpsc::unbounded_channel();
        let subscriber_id = Uuid::new(RandomDigitCount::Four);

        Self {
            app_name,
            stream_name,
            event_producer,
            segment_manager,
            stream_registry,
            data_consumer,
            subscriber_id,
        }
    }

    async fn run(mut self) -> Result<(), HlsRemuxerError> {
        // Subscribe to stream
        self.subscribe_from_stream_hub().await?;

        // Create registry key
        let registry_key = format!("{}/{}", self.app_name, self.stream_name);

        // Register stream in registry
        let state = Arc::new(parking_lot::RwLock::new(StreamProcessorState {
            app_name: self.app_name.clone(),
            stream_name: self.stream_name.clone(),
            segments: VecDeque::new(),
            is_ended: false,
        }));
        self.stream_registry.insert(registry_key.clone(), state.clone());

        // Process FLV data and generate HLS segments
        let mut processor = StreamProcessor::new(
            &self.app_name,
            &self.stream_name,
            Arc::clone(&self.segment_manager),
            state.clone(),
        );

        processor.process_stream(&mut self.data_consumer).await?;

        // Unsubscribe when done
        self.unsubscribe_from_stream_hub().await?;

        // Remove from registry after some delay (allow clients to finish)
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        self.stream_registry.remove(&registry_key);

        Ok(())
    }

    async fn subscribe_from_stream_hub(&mut self) -> Result<(), HlsRemuxerError> {
        let sub_info = SubscriberInfo {
            id: self.subscriber_id,
            sub_type: SubscribeType::RtmpRemux2Hls,
            sub_data_type: streamhub::define::SubDataType::Frame,
            notify_info: NotifyInfo {
                request_url: String::new(),
                remote_addr: String::new(),
            },
        };

        let identifier = StreamIdentifier::Rtmp {
            app_name: self.app_name.clone(),
            stream_name: self.stream_name.clone(),
        };

        let (event_result_sender, event_result_receiver) = tokio::sync::oneshot::channel();

        let subscribe_event = StreamHubEvent::Subscribe {
            identifier,
            info: sub_info,
            result_sender: event_result_sender,
        };

        self.event_producer
            .send(subscribe_event)
            .map_err(|_| HlsRemuxerError::StreamHubEventSendError)?;

        let receiver = event_result_receiver
            .await
            .map_err(|_| HlsRemuxerError::SubscribeError)??
            .0
            .frame_receiver
            .ok_or(HlsRemuxerError::NoFrameReceiver)?;

        self.data_consumer = receiver;

        log::info!(
            "Subscribed to stream: {}/{}",
            self.app_name,
            self.stream_name
        );

        Ok(())
    }

    async fn unsubscribe_from_stream_hub(&mut self) -> Result<(), HlsRemuxerError> {
        let sub_info = SubscriberInfo {
            id: self.subscriber_id,
            sub_type: SubscribeType::RtmpRemux2Hls,
            sub_data_type: streamhub::define::SubDataType::Frame,
            notify_info: NotifyInfo {
                request_url: String::new(),
                remote_addr: String::new(),
            },
        };

        let identifier = StreamIdentifier::Rtmp {
            app_name: self.app_name.clone(),
            stream_name: self.stream_name.clone(),
        };

        let unsubscribe_event = StreamHubEvent::UnSubscribe {
            identifier,
            info: sub_info,
        };

        if let Err(e) = self.event_producer.send(unsubscribe_event) {
            log::error!("Unsubscribe error: {}", e);
        }

        log::info!(
            "Unsubscribed from stream: {}/{}",
            self.app_name,
            self.stream_name
        );

        Ok(())
    }
}

/// Processes FLV data and generates HLS segments
struct StreamProcessor {
    app_name: String,
    stream_name: String,
    segment_manager: Arc<SegmentManager>,
    state: Arc<parking_lot::RwLock<StreamProcessorState>>,

    // Demuxers
    video_demuxer: FlvVideoTagDemuxer,
    audio_demuxer: FlvAudioTagDemuxer,

    // TS muxer
    ts_muxer: TsMuxer,
    video_pid: u16,
    audio_pid: u16,

    // Segment tracking
    sequence_no: u64,
    max_segments: usize, // Keep last N segments in M3U8

    // Timing
    segment_duration_ms: i64, // Target segment duration (e.g., 10000ms = 10s)
    last_segment_dts: i64,
    last_dts: i64,
    last_pts: i64,
}

impl StreamProcessor {
    fn new(
        app_name: &str,
        stream_name: &str,
        segment_manager: Arc<SegmentManager>,
        state: Arc<parking_lot::RwLock<StreamProcessorState>>,
    ) -> Self {
        let mut ts_muxer = TsMuxer::new();
        let audio_pid = ts_muxer
            .add_stream(epsi_stream_type::PSI_STREAM_AAC, BytesMut::new())
            .unwrap();
        let video_pid = ts_muxer
            .add_stream(epsi_stream_type::PSI_STREAM_H264, BytesMut::new())
            .unwrap();

        Self {
            app_name: app_name.to_string(),
            stream_name: stream_name.to_string(),
            segment_manager,
            state,
            video_demuxer: FlvVideoTagDemuxer::new(),
            audio_demuxer: FlvAudioTagDemuxer::new(),
            ts_muxer,
            video_pid,
            audio_pid,
            sequence_no: 0,
            max_segments: 6, // Keep last 6 segments
            segment_duration_ms: 10000, // 10 seconds
            last_segment_dts: 0,
            last_dts: 0,
            last_pts: 0,
        }
    }

    async fn process_stream(
        &mut self,
        data_consumer: &mut FrameDataReceiver,
    ) -> Result<(), HlsRemuxerError> {
        let mut retry_count = 0;

        loop {
            if let Some(frame_data) = data_consumer.recv().await {
                let flv_data = match frame_data {
                    FrameData::Audio { timestamp, data } => FlvData::Audio { timestamp, data },
                    FrameData::Video { timestamp, data } => FlvData::Video { timestamp, data },
                    _ => continue,
                };

                retry_count = 0;
                self.process_flv_data(flv_data).await?;
            } else {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                retry_count += 1;
            }

            // Stream ended
            if retry_count > 10 {
                log::info!("Stream ended: {}/{}", self.app_name, self.stream_name);
                self.flush_remaining_segment().await?;
                break;
            }
        }

        Ok(())
    }

    async fn process_flv_data(&mut self, flv_data: FlvData) -> Result<(), HlsRemuxerError> {
        let (pid, pts, dts, flags, payload) = match flv_data {
            FlvData::Video { timestamp, data } => {
                let video_data = self
                    .video_demuxer
                    .demux(timestamp, data)
                    .map_err(|e| HlsRemuxerError::DemuxError(format!("Video demux error: {e:?}")))?;

                if video_data.is_none() {
                    return Ok(());
                }

                let video_data = video_data.unwrap();
                let mut flags = 0;
                let mut payload = BytesMut::new();
                payload.extend_from_slice(&video_data.data);

                // Check if keyframe and if we need new segment
                if video_data.frame_type == frame_type::KEY_FRAME {
                    flags = MPEG_FLAG_IDR_FRAME;

                    if video_data.dts - self.last_segment_dts >= self.segment_duration_ms * 1000 {
                        self.finalize_segment(video_data.dts, false).await?;
                    }
                }

                self.last_dts = video_data.dts;
                self.last_pts = video_data.pts;

                (self.video_pid, video_data.pts, video_data.dts, flags, payload)
            }
            FlvData::Audio { timestamp, data } => {
                let audio_data = self
                    .audio_demuxer
                    .demux(timestamp, data)
                    .map_err(|e| HlsRemuxerError::DemuxError(format!("Audio demux error: {e:?}")))?;

                if !audio_data.has_data {
                    return Ok(());
                }

                let mut payload = BytesMut::new();
                payload.extend_from_slice(&audio_data.data);

                self.last_dts = audio_data.dts;
                self.last_pts = audio_data.pts;

                (self.audio_pid, audio_data.pts, audio_data.dts, 0, payload)
            }
            _ => return Ok(()),
        };

        // Write to TS muxer
        self.ts_muxer
            .write(pid, pts * 90, dts * 90, flags, payload)
            .map_err(|e| HlsRemuxerError::MuxError(format!("TS mux error: {e:?}")))?;

        Ok(())
    }

    async fn finalize_segment(&mut self, current_dts: i64, is_eof: bool) -> Result<(), HlsRemuxerError> {
        let ts_data = self.ts_muxer.get_data();
        let duration_ms = current_dts - self.last_segment_dts;
        let ts_data_len = ts_data.len();

        // Generate TS filename using nanoid (12 chars, like Go's SortUUID)
        let ts_name = nanoid::nanoid!(12);

        // Generate storage key: app_name-stream_name-ts_name
        // stream_name format is "room_id:media_id", replace : with - for flat key
        let storage_key = format!(
            "{}-{}-{}",
            self.app_name,
            self.stream_name.replace(':', "-"),
            ts_name
        );

        // Write segment to storage
        self.segment_manager
            .storage()
            .write(&storage_key, ts_data.into())
            .await
            .map_err(|e| HlsRemuxerError::StorageError(e.to_string()))?;

        log::debug!(
            "Wrote segment: {} ({}ms, {} bytes)",
            storage_key,
            duration_ms,
            ts_data_len
        );

        // Track segment metadata
        let segment_info = SegmentInfo {
            sequence: self.sequence_no,
            duration: duration_ms,
            ts_name,
            storage_key,
            discontinuity: false,
            created_at: Instant::now(),
        };

        // Update shared state with new segment
        {
            let mut state = self.state.write();
            state.segments.push_back(segment_info);

            // Remove old segments from list (but keep in storage for now, cleanup task will handle)
            if state.segments.len() > self.max_segments {
                state.segments.pop_front();
            }

            // Mark stream as ended if this is the last segment
            if is_eof {
                state.is_ended = true;
                log::info!("Stream ended: {}/{}", self.app_name, self.stream_name);
            }
        }

        // Reset for next segment
        self.ts_muxer.reset();
        self.last_segment_dts = current_dts;
        self.sequence_no += 1;

        Ok(())
    }

    async fn flush_remaining_segment(&mut self) -> Result<(), HlsRemuxerError> {
        if self.last_dts > self.last_segment_dts {
            self.finalize_segment(self.last_dts, true).await?;
        }
        Ok(())
    }
}

// Error types
#[derive(Debug, thiserror::Error)]
pub enum HlsRemuxerError {
    #[error("StreamHub event send error")]
    StreamHubEventSendError,

    #[error("Subscribe error")]
    SubscribeError,

    #[error("No frame receiver")]
    NoFrameReceiver,

    #[error("Demux error: {0}")]
    DemuxError(String),

    #[error("Mux error: {0}")]
    MuxError(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Receive error: {0}")]
    ReceiveError(#[from] tokio::sync::broadcast::error::RecvError),
}

impl From<streamhub::errors::StreamHubError> for HlsRemuxerError {
    fn from(_: streamhub::errors::StreamHubError) -> Self {
        Self::SubscribeError
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segment_info_creation() {
        let segment = SegmentInfo {
            sequence: 1,
            duration: 10000,
            ts_name: "test_segment".to_string(),
            storage_key: "live/room123/test_segment".to_string(),
            discontinuity: false,
            created_at: Instant::now(),
        };

        assert_eq!(segment.sequence, 1);
        assert_eq!(segment.duration, 10000);
        assert!(!segment.discontinuity);
    }

    #[test]
    fn test_stream_processor_state_generate_m3u8() {
        let mut state = StreamProcessorState {
            app_name: "live".to_string(),
            stream_name: "room123/media456".to_string(),
            segments: VecDeque::new(),
            is_ended: false,
        };

        // Add some segments
        state.segments.push_back(SegmentInfo {
            sequence: 0,
            duration: 10000,
            ts_name: "segment0.ts".to_string(),
            storage_key: "live/room123/segment0.ts".to_string(),
            discontinuity: false,
            created_at: Instant::now(),
        });

        state.segments.push_back(SegmentInfo {
            sequence: 1,
            duration: 10000,
            ts_name: "segment1.ts".to_string(),
            storage_key: "live/room123/segment1.ts".to_string(),
            discontinuity: false,
            created_at: Instant::now(),
        });

        // Generate M3U8
        let m3u8 = state.generate_m3u8(|ts_name| format!("/api/hls/{}", ts_name));

        // Verify M3U8 content
        assert!(m3u8.contains("#EXTM3U"));
        assert!(m3u8.contains("#EXT-X-VERSION:3"));
        assert!(m3u8.contains("#EXT-X-TARGETDURATION:"));
        assert!(m3u8.contains("#EXT-X-MEDIA-SEQUENCE:0"));
        assert!(m3u8.contains("segment0.ts"));
        assert!(m3u8.contains("segment1.ts"));
    }

    #[test]
    fn test_stream_processor_state_with_discontinuity() {
        let mut state = StreamProcessorState {
            app_name: "live".to_string(),
            stream_name: "room123/media456".to_string(),
            segments: VecDeque::new(),
            is_ended: false,
        };

        // Add segment with discontinuity
        state.segments.push_back(SegmentInfo {
            sequence: 0,
            duration: 10000,
            ts_name: "segment0.ts".to_string(),
            storage_key: "live/room123/segment0.ts".to_string(),
            discontinuity: true,
            created_at: Instant::now(),
        });

        // Generate M3U8
        let m3u8 = state.generate_m3u8(|ts_name| format!("/api/hls/{}", ts_name));

        // Verify discontinuity tag is present
        assert!(m3u8.contains("#EXT-X-DISCONTINUITY"));
    }

    #[test]
    fn test_stream_processor_state_ended() {
        let mut state = StreamProcessorState {
            app_name: "live".to_string(),
            stream_name: "room123/media456".to_string(),
            segments: VecDeque::new(),
            is_ended: true,
        };

        // Add a segment
        state.segments.push_back(SegmentInfo {
            sequence: 0,
            duration: 10000,
            ts_name: "segment0.ts".to_string(),
            storage_key: "live/room123/segment0.ts".to_string(),
            discontinuity: false,
            created_at: Instant::now(),
        });

        // Generate M3U8
        let m3u8 = state.generate_m3u8(|ts_name| format!("/api/hls/{}", ts_name));

        // Verify ENDLIST tag is present
        assert!(m3u8.contains("#EXT-X-ENDLIST"));
    }

    #[test]
    fn test_stream_processor_state_custom_url_generator() {
        let mut state = StreamProcessorState {
            app_name: "live".to_string(),
            stream_name: "room123/media456".to_string(),
            segments: VecDeque::new(),
            is_ended: false,
        };

        // Add segment
        state.segments.push_back(SegmentInfo {
            sequence: 0,
            duration: 10000,
            ts_name: "segment0.ts".to_string(),
            storage_key: "live/room123/segment0.ts".to_string(),
            discontinuity: false,
            created_at: Instant::now(),
        });

        // Generate M3U8 with custom URL generator (e.g., adding auth token)
        let m3u8 = state.generate_m3u8(|ts_name| {
            format!("/api/room/live/hls/data/room123/media456/{}?token=abc123", ts_name)
        });

        // Verify custom URL format is used
        assert!(m3u8.contains("?token=abc123"));
        assert!(m3u8.contains("/api/room/live/hls/data/room123/media456/"));
    }

    #[test]
    fn test_hls_remuxer_error_display() {
        let error = HlsRemuxerError::DemuxError("test error".to_string());
        assert_eq!(error.to_string(), "Demux error: test error");

        let error = HlsRemuxerError::StorageError("storage failed".to_string());
        assert_eq!(error.to_string(), "Storage error: storage failed");
    }
}
