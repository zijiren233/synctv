// Integration layer between xiu's StreamHub and SyncTV's GOP cache

use crate::libraries::gop_cache::{GopCache, GopFrame};
use std::sync::Arc;
use streamhub::define::{FrameData, TStreamHandler, DataSender, SubscribeType};
use streamhub::errors::StreamHubError;
use streamhub::statistics::StatisticsStream;
use streamhub::define::InformationSender;
use async_trait::async_trait;

pub struct SyncTvStreamHandler {
    room_id: String,
    gop_cache: Arc<GopCache>,
}

impl SyncTvStreamHandler {
    pub fn new(room_id: String, gop_cache: Arc<GopCache>) -> Self {
        Self { room_id, gop_cache }
    }

    pub fn save_frame(&self, frame_data: &FrameData) {
        let (timestamp, data, is_keyframe, frame_type) = match frame_data {
            FrameData::Video { timestamp, data } => {
                // Detect keyframe from video data
                // For H.264: check if frame type is 1 (keyframe)
                let is_keyframe = if !data.is_empty() {
                    let frame_type = (data[0] >> 4) & 0x0F;
                    frame_type == 1
                } else {
                    false
                };
                (*timestamp, data.clone().freeze(), is_keyframe, crate::libraries::gop_cache::FrameType::Video)
            }
            FrameData::Audio { timestamp, data } => {
                (*timestamp, data.clone().freeze(), false, crate::libraries::gop_cache::FrameType::Audio)
            }
            FrameData::MetaData { timestamp, data } => {
                // Metadata frames treated as video for now
                (*timestamp, data.clone().freeze(), false, crate::libraries::gop_cache::FrameType::Video)
            }
            _ => return,
        };

        let gop_frame = GopFrame {
            timestamp,
            is_keyframe,
            frame_type,
            data,
        };

        self.gop_cache.add_frame(&self.room_id, gop_frame);
    }
}

#[async_trait]
impl TStreamHandler for SyncTvStreamHandler {
    async fn send_prior_data(
        &self,
        sender: DataSender,
        _sub_type: SubscribeType,
    ) -> Result<(), StreamHubError> {
        let frame_sender = match sender {
            DataSender::Frame { sender } => sender,
            DataSender::Packet { .. } => {
                return Err(StreamHubError {
                    value: streamhub::errors::StreamHubErrorValue::NotCorrectDataSenderType,
                });
            }
        };

        // Send cached GOP frames to new subscriber
        let frames = self.gop_cache.get_frames(&self.room_id);

        tracing::info!(
            "Sending {} cached frames to new subscriber for room {}",
            frames.len(),
            self.room_id
        );

        for gop_frame in frames {
            // Convert GopFrame back to FrameData
            // Determine if audio or video based on frame_type
            let frame_data = match gop_frame.frame_type {
                crate::libraries::gop_cache::FrameType::Video => FrameData::Video {
                    timestamp: gop_frame.timestamp,
                    data: gop_frame.data.into(),
                },
                crate::libraries::gop_cache::FrameType::Audio => FrameData::Audio {
                    timestamp: gop_frame.timestamp,
                    data: gop_frame.data.into(),
                },
            };

            frame_sender.send(frame_data).map_err(|_| StreamHubError {
                value: streamhub::errors::StreamHubErrorValue::SendError,
            })?;
        }

        Ok(())
    }

    async fn get_statistic_data(&self) -> Option<StatisticsStream> {
        None
    }

    async fn send_information(&self, _sender: InformationSender) {
        // No-op for now
    }
}
