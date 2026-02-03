// SyncTV stream handler for GOP cache integration
//
// Implements xiu's TStreamHandler trait to provide GOP cache functionality
// When new subscribers join, cached GOP frames are sent for instant playback

use crate::cache::gop_cache::{GopCache, GopFrame, FrameType};
use bytes::BytesMut;
use bytes::Bytes;
use std::sync::Arc;
use streamhub::utils::{FrameData, SubscriberInfo};
use anyhow::Result;

pub struct SyncTvStreamHandler {
    room_id: String,
    gop_cache: Arc<GopCache>,
}

impl SyncTvStreamHandler {
    pub fn new(room_id: String, gop_cache: Arc<GopCache>) -> Self {
        Self { room_id, gop_cache }
    }

    /// Save frame data to GOP cache
    ///
    /// Detects keyframes from H.264 video data and manages GOP boundaries
    pub fn save_frame(&self, timestamp: u32, data: BytesMut, is_video: bool) {
        let is_keyframe = if is_video && !data.is_empty() {
            // H.264 keyframe detection
            let frame_type = (data[0] >> 4) & 0x0F;
            frame_type == 1
        } else {
            false
        };

        let gop_frame = GopFrame {
            timestamp,
            is_keyframe,
            data: data.freeze(),
            frame_type: if is_video { FrameType::Video } else { FrameType::Audio },
        };

        self.gop_cache.add_frame(&self.room_id, gop_frame);
    }

    /// Get cached GOP frames for instant playback
    pub fn get_cached_frames(&self) -> Vec<GopFrame> {
        self.gop_cache.get_frames(&self.room_id)
    }

    /// Send prior data to new subscriber (GOP cache frames)
    ///
    /// This is called by xiu's StreamHub when a new subscriber joins.
    /// We send cached GOP frames to provide instant playback without waiting
    /// for the next keyframe.
    ///
    /// # Arguments
    /// * `subscriber_info` - Information about the new subscriber
    /// * `data_sender` - Channel to send prior data to
    ///
    /// # Returns
    /// Result indicating success or failure
    pub fn send_prior_data(
        &self,
        _subscriber_info: &SubscriberInfo,
        data_sender: &tokio::sync::mpsc::UnboundedSender<FrameData>,
    ) -> Result<()> {
        let frames = self.get_cached_frames();

        tracing::debug!(
            room_id = %self.room_id,
            frame_count = frames.len(),
            "Sending GOP cache frames to new subscriber"
        );

        // Send cached frames to new subscriber
        for frame in frames {
            let frame_data = if frame.frame_type == FrameType::Video {
                FrameData::Video {
                    timestamp: frame.timestamp,
                    data: Bytes::copy_from_slice(&frame.data),
                }
            } else {
                FrameData::Audio {
                    timestamp: frame.timestamp,
                    data: Bytes::copy_from_slice(&frame.data),
                }
            };

            data_sender
                .send(frame_data)
                .map_err(|e| anyhow::anyhow!("Failed to send cached frame: {}", e))?;
        }

        tracing::debug!(
            room_id = %self.room_id,
            "Successfully sent GOP cache frames to new subscriber"
        );

        Ok(())
    }
}
