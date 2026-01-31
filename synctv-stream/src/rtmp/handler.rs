// Integration layer between xiu's StreamHub and SyncTV's GOP cache

use crate::cache::gop_cache::{GopCache, GopFrame};
use bytes::BytesMut;
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
        let (timestamp, data, is_keyframe) = match frame_data {
            FrameData::Video { timestamp, data } => {
                // Detect keyframe from video data
                // For H.264: check if frame type is 1 (keyframe)
                let is_keyframe = if !data.is_empty() {
                    let frame_type = (data[0] >> 4) & 0x0F;
                    frame_type == 1
                } else {
                    false
                };
                (*timestamp, data.clone(), is_keyframe)
            }
            FrameData::Audio { timestamp, data } => {
                (*timestamp, data.clone(), false)
            }
            FrameData::MetaData { timestamp, data } => {
                (*timestamp, data.clone(), false)
            }
            _ => return,
        };

        let gop_frame = GopFrame {
            timestamp,
            is_keyframe,
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

        log::info!(
            "Sending {} cached frames to new subscriber for room {}",
            frames.len(),
            self.room_id
        );

        for gop_frame in frames {
            // Convert GopFrame back to FrameData
            // Determine if audio or video based on content or metadata
            let frame_data = if gop_frame.is_keyframe || is_video_data(&gop_frame.data) {
                FrameData::Video {
                    timestamp: gop_frame.timestamp,
                    data: gop_frame.data,
                }
            } else {
                FrameData::Audio {
                    timestamp: gop_frame.timestamp,
                    data: gop_frame.data,
                }
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

fn is_video_data(data: &BytesMut) -> bool {
    if data.is_empty() {
        return false;
    }

    // FLV video tag detection
    // Video frame type is in the first byte (bits 4-7)
    let frame_type = (data[0] >> 4) & 0x0F;
    // Video codec ID is in the first byte (bits 0-3)
    let codec_id = data[0] & 0x0F;

    // H.264/AVC codec ID is 7
    // Frame types: 1=keyframe, 2=inter frame, 3=disposable, 4=generated, 5=video info/command
    frame_type <= 5 && codec_id == 7
}
