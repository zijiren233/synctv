//! Media track management for SFU
//!
//! This module handles complete WebRTC media track lifecycle including:
//! - Track creation and lifecycle management
//! - RTP packet reception and forwarding
//! - Simulcast quality layer handling
//! - Track statistics collection

use crate::types::{PeerId, TrackId};
use anyhow::Result;
use bytes::Bytes;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, error, info};
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::track::track_remote::TrackRemote;
use webrtc::util::marshal::MarshalSize;

/// Media track kind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    Audio,
    Video,
}

impl From<webrtc::rtp_transceiver::rtp_codec::RTPCodecType> for TrackKind {
    fn from(codec_type: webrtc::rtp_transceiver::rtp_codec::RTPCodecType) -> Self {
        match codec_type {
            webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Audio => Self::Audio,
            webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Video => Self::Video,
            _ => Self::Video, // Default to video
        }
    }
}

impl From<&str> for TrackKind {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "audio" => Self::Audio,
            "video" | _ => Self::Video,
        }
    }
}

/// Simulcast quality layer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QualityLayer {
    High,
    Medium,
    Low,
}

impl QualityLayer {
    /// Select quality layer based on available bandwidth
    /// bandwidth in kbps
    #[must_use] 
    pub const fn from_bandwidth(bandwidth_kbps: u32) -> Self {
        if bandwidth_kbps >= 2000 {
            Self::High // >= 2 Mbps
        } else if bandwidth_kbps >= 1000 {
            Self::Medium // >= 1 Mbps
        } else {
            Self::Low // < 1 Mbps
        }
    }

    /// Get the RID (restriction identifier) for this layer
    #[must_use] 
    pub const fn rid(&self) -> &'static str {
        match self {
            Self::High => "h",
            Self::Medium => "m",
            Self::Low => "l",
        }
    }

    /// Get expected bitrate for this layer (kbps)
    #[must_use] 
    pub const fn expected_bitrate(&self) -> u32 {
        match self {
            Self::High => 2500,    // 2.5 Mbps
            Self::Medium => 1200,  // 1.2 Mbps
            Self::Low => 500,      // 500 kbps
        }
    }

    /// Get spatial layer index (for SVC/Simulcast)
    #[must_use] 
    pub const fn spatial_layer(&self) -> u8 {
        match self {
            Self::High => 2,
            Self::Medium => 1,
            Self::Low => 0,
        }
    }
}

/// RTP packet with metadata for forwarding
#[derive(Debug, Clone)]
pub struct ForwardablePacket {
    /// RTP packet data
    pub data: Bytes,

    /// Source SSRC
    pub ssrc: u32,

    /// Sequence number
    pub sequence_number: u16,

    /// Timestamp
    pub timestamp: u32,

    /// Quality layer (for simulcast)
    pub quality_layer: Option<QualityLayer>,

    /// When packet was received
    pub received_at: Instant,
}

/// Media track in the SFU
pub struct MediaTrack {
    /// Track ID
    pub id: TrackId,

    /// Owner peer ID
    pub peer_id: PeerId,

    /// Track kind (audio/video)
    pub kind: TrackKind,

    /// Remote track from WebRTC
    pub remote_track: Arc<TrackRemote>,

    /// RTP receiver
    pub receiver: Arc<RTCRtpReceiver>,

    /// Current active quality layer (for simulcast video)
    pub active_quality_layer: Arc<RwLock<Option<QualityLayer>>>,

    /// Whether this track is active
    pub active: Arc<RwLock<bool>>,

    /// Track statistics
    stats: Arc<TrackStatsInner>,

    /// Packet forwarding channel
    packet_tx: Option<mpsc::UnboundedSender<ForwardablePacket>>,
}

/// Internal track statistics with atomic counters
struct TrackStatsInner {
    packets_received: AtomicU64,
    bytes_received: AtomicU64,
    packets_sent: AtomicU64,
    bytes_sent: AtomicU64,
    packets_lost: AtomicU64,
    last_packet_time: RwLock<Option<Instant>>,
}

impl MediaTrack {
    /// Create a new media track
    pub fn new(
        id: TrackId,
        peer_id: PeerId,
        remote_track: Arc<TrackRemote>,
        receiver: Arc<RTCRtpReceiver>,
    ) -> Self {
        let kind = TrackKind::from(remote_track.kind());

        info!(
            track_id = %id,
            peer_id = %peer_id,
            kind = ?kind,
            codec = %remote_track.codec().capability.mime_type,
            "Creating media track"
        );

        Self {
            id,
            peer_id,
            kind,
            remote_track,
            receiver,
            active_quality_layer: Arc::new(RwLock::new(None)),
            active: Arc::new(RwLock::new(true)),
            stats: Arc::new(TrackStatsInner {
                packets_received: AtomicU64::new(0),
                bytes_received: AtomicU64::new(0),
                packets_sent: AtomicU64::new(0),
                bytes_sent: AtomicU64::new(0),
                packets_lost: AtomicU64::new(0),
                last_packet_time: RwLock::new(None),
            }),
            packet_tx: None,
        }
    }

    /// Start reading RTP packets from the track
    pub async fn start_reading(
        &mut self,
    ) -> Result<mpsc::UnboundedReceiver<ForwardablePacket>> {
        let (packet_tx, packet_rx) = mpsc::unbounded_channel();
        self.packet_tx = Some(packet_tx.clone());

        let track = Arc::clone(&self.remote_track);
        let stats = Arc::clone(&self.stats);
        let track_id = self.id.clone();
        let quality_layer = Arc::clone(&self.active_quality_layer);
        let active = Arc::clone(&self.active);

        // Spawn RTP packet reading task
        tokio::spawn(async move {
            let mut buf = vec![0u8; 1500]; // MTU size

            loop {
                // Check if track is still active
                if !*active.read() {
                    debug!(track_id = %track_id, "Track deactivated, stopping RTP reader");
                    break;
                }

                // Read RTP packet
                match track.read(&mut buf).await {
                    Ok((rtp_packet, _attributes)) => {
                        // Update statistics
                        let packet_size = rtp_packet.header.marshal_size() + rtp_packet.payload.len();
                        stats.packets_received.fetch_add(1, Ordering::Relaxed);
                        stats.bytes_received.fetch_add(packet_size as u64, Ordering::Relaxed);
                        *stats.last_packet_time.write() = Some(Instant::now());

                        // Create forwardable packet
                        let forwardable = ForwardablePacket {
                            data: Bytes::copy_from_slice(&buf[..packet_size]),
                            ssrc: rtp_packet.header.ssrc,
                            sequence_number: rtp_packet.header.sequence_number,
                            timestamp: rtp_packet.header.timestamp,
                            quality_layer: *quality_layer.read(),
                            received_at: Instant::now(),
                        };

                        // Forward packet to subscribers
                        if let Err(e) = packet_tx.send(forwardable) {
                            error!(
                                track_id = %track_id,
                                error = %e,
                                "Failed to forward RTP packet"
                            );
                            break;
                        }
                    }
                    Err(e) => {
                        error!(
                            track_id = %track_id,
                            error = %e,
                            "Failed to read RTP packet"
                        );
                        break;
                    }
                }
            }

            info!(track_id = %track_id, "RTP reader stopped");
        });

        Ok(packet_rx)
    }

    /// Get track SSRC (Synchronization Source)
    #[must_use] 
    pub fn ssrc(&self) -> u32 {
        self.remote_track.ssrc()
    }

    /// Get track codec
    #[must_use] 
    pub fn codec(&self) -> String {
        self.remote_track.codec().capability.mime_type
    }

    /// Set active quality layer for simulcast
    pub fn set_quality_layer(&self, layer: QualityLayer) {
        let mut current = self.active_quality_layer.write();
        if *current != Some(layer) {
            debug!(
                track_id = %self.id,
                old_layer = ?*current,
                new_layer = ?layer,
                "Switching quality layer"
            );
            *current = Some(layer);
        }
    }

    /// Get active quality layer
    #[must_use] 
    pub fn quality_layer(&self) -> Option<QualityLayer> {
        *self.active_quality_layer.read()
    }

    /// Check if track is video
    #[must_use] 
    pub fn is_video(&self) -> bool {
        self.kind == TrackKind::Video
    }

    /// Check if track is audio
    #[must_use] 
    pub fn is_audio(&self) -> bool {
        self.kind == TrackKind::Audio
    }

    /// Check if track is active
    #[must_use] 
    pub fn is_active(&self) -> bool {
        *self.active.read()
    }

    /// Deactivate track
    pub fn deactivate(&self) {
        *self.active.write() = false;
    }

    /// Get track statistics
    #[must_use] 
    pub fn get_stats(&self) -> TrackStats {
        let packets_received = self.stats.packets_received.load(Ordering::Relaxed);
        let bytes_received = self.stats.bytes_received.load(Ordering::Relaxed);
        let packets_sent = self.stats.packets_sent.load(Ordering::Relaxed);
        let bytes_sent = self.stats.bytes_sent.load(Ordering::Relaxed);
        let packets_lost = self.stats.packets_lost.load(Ordering::Relaxed);

        // Calculate bitrate (over last second)
        let bitrate_kbps = if let Some(last_time) = *self.stats.last_packet_time.read() {
            let elapsed = Instant::now().duration_since(last_time);
            if elapsed < Duration::from_secs(1) {
                ((bytes_received * 8) as f64 / elapsed.as_secs_f64() / 1000.0) as u32
            } else {
                0
            }
        } else {
            0
        };

        TrackStats {
            track_id: self.id.as_str().to_string(),
            kind: self.kind,
            packets_received,
            bytes_received,
            packets_sent,
            bytes_sent,
            packets_lost,
            bitrate_kbps,
            quality_layer: self.quality_layer(),
        }
    }

    /// Update sent packet statistics
    pub fn record_sent_packet(&self, packet_size: usize) {
        self.stats.packets_sent.fetch_add(1, Ordering::Relaxed);
        self.stats.bytes_sent.fetch_add(packet_size as u64, Ordering::Relaxed);
    }

    /// Record packet loss
    pub fn record_packet_loss(&self, count: u64) {
        self.stats.packets_lost.fetch_add(count, Ordering::Relaxed);
    }
}

/// Track statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackStats {
    pub track_id: String,
    pub kind: TrackKind,
    pub packets_received: u64,
    pub bytes_received: u64,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub packets_lost: u64,
    pub bitrate_kbps: u32,
    pub quality_layer: Option<QualityLayer>,
}
