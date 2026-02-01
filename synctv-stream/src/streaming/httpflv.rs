// HTTP-FLV streaming implementation
// Provides router for synctv-api integration

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Extension, Router,
};
use bytes::BytesMut;
use std::sync::Arc;
use streamhub::{
    define::{
        FrameData, FrameDataReceiver, NotifyInfo, StreamHubEvent, StreamHubEventSender,
        SubDataType, SubscribeType, SubscriberInfo,
    },
    stream::StreamIdentifier,
    utils::{RandomDigitCount, Uuid},
};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info, warn};
use xflv::amf0::amf0_writer::Amf0Writer;
use xflv::muxer::{FlvMuxer, HEADER_LENGTH};

use crate::relay::StreamRegistry;

#[derive(Clone)]
pub struct HttpFlvState {
    registry: Arc<StreamRegistry>,
    stream_hub_event_sender: StreamHubEventSender,
}

impl HttpFlvState {
    pub fn new(registry: Arc<StreamRegistry>, stream_hub_event_sender: StreamHubEventSender) -> Self {
        Self {
            registry,
            stream_hub_event_sender,
        }
    }
}

/// Create HTTP-FLV router
/// Routes:
/// - GET /live/flv/:media_id - FLV streaming (requires auth with room_id in Extension)
pub fn create_flv_router(state: HttpFlvState) -> Router {
    Router::new()
        .route("/live/flv/:media_id", get(handle_flv_stream))
        .with_state(state)
}

/// Handle FLV streaming request
/// Path: GET /live/flv/:media_id
/// Requires: Extension<RoomId> from auth middleware
async fn handle_flv_stream(
    Path(media_id): Path<String>,
    Extension(room_id): Extension<String>,
    State(state): State<HttpFlvState>,
) -> Result<Response, StatusCode> {
    // Remove .flv suffix if present
    let media_id = media_id.trim_end_matches(".flv");

    info!(
        room_id = %room_id,
        media_id = %media_id,
        "FLV streaming request"
    );

    // Check if stream exists (publisher registered)
    let mut registry = (*state.registry).clone();
    match registry.get_publisher(&room_id, media_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            warn!("No publisher for room {} / media {}", room_id, media_id);
            return Err(StatusCode::NOT_FOUND);
        }
        Err(e) => {
            error!("Failed to query publisher: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    // Create channel for HTTP response data
    let (tx, rx) = mpsc::unbounded_channel::<Result<bytes::Bytes, std::io::Error>>();

    // Spawn FLV session
    let stream_name = format!("{}/{}", room_id, media_id);
    let mut flv_session = HttpFlvSession::new(
        "live".to_string(),
        stream_name,
        state.stream_hub_event_sender,
        tx,
    );

    tokio::spawn(async move {
        if let Err(e) = flv_session.run().await {
            error!("FLV session error: {}", e);
        }
    });

    // Return streaming response
    let body = Body::from_stream(tokio_stream::wrappers::UnboundedReceiverStream::new(rx));

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "video/x-flv")
        .header(header::CACHE_CONTROL, "no-cache, no-store")
        .header(header::CONNECTION, "close")
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_response())
}

/// HTTP-FLV session (per-client connection)
pub struct HttpFlvSession {
    app_name: String,
    stream_name: String,
    event_producer: StreamHubEventSender,
    data_receiver: FrameDataReceiver,
    response_producer: mpsc::UnboundedSender<Result<bytes::Bytes, std::io::Error>>,
    subscriber_id: Uuid,
    muxer: FlvMuxer,
    has_audio: bool,
    has_video: bool,
    has_send_header: bool,
}

impl HttpFlvSession {
    pub fn new(
        app_name: String,
        stream_name: String,
        event_producer: StreamHubEventSender,
        response_producer: mpsc::UnboundedSender<Result<bytes::Bytes, std::io::Error>>,
    ) -> Self {
        let (_, data_receiver) = mpsc::unbounded_channel();
        let subscriber_id = Uuid::new(RandomDigitCount::Four);

        Self {
            app_name,
            stream_name,
            event_producer,
            data_receiver,
            response_producer,
            subscriber_id,
            muxer: FlvMuxer::new(),
            has_audio: false,
            has_video: false,
            has_send_header: false,
        }
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        // Subscribe to StreamHub
        self.subscribe_from_stream_hub().await?;

        // Send media stream
        self.send_media_stream().await?;

        Ok(())
    }

    async fn send_media_stream(&mut self) -> anyhow::Result<()> {
        let mut retry_count = 0;
        let mut max_av_frame_num_to_guess_av = 0;
        let mut cached_frames = Vec::new();

        loop {
            if let Some(data) = self.data_receiver.recv().await {
                // Detect audio/video before sending header
                if !self.has_send_header {
                    max_av_frame_num_to_guess_av += 1;

                    match data {
                        FrameData::Audio { .. } => {
                            self.has_audio = true;
                            cached_frames.push(data);
                        }
                        FrameData::Video { .. } => {
                            self.has_video = true;
                            cached_frames.push(data);
                        }
                        FrameData::MetaData { .. } => cached_frames.push(data),
                        _ => {}
                    }

                    // Send header after detecting A/V or after 10 frames
                    if (self.has_audio && self.has_video) || max_av_frame_num_to_guess_av > 10 {
                        self.has_send_header = true;

                        // Write FLV header
                        self.muxer
                            .write_flv_header(self.has_audio, self.has_video)
                            .map_err(|e| anyhow::anyhow!("Failed to write FLV header: {:?}", e))?;
                        self.muxer
                            .write_previous_tag_size(0)
                            .map_err(|e| anyhow::anyhow!("Failed to write tag size: {:?}", e))?;
                        self.flush_response_data()?;

                        // Write cached frames
                        for frame in &cached_frames {
                            self.write_flv_tag(frame.clone())?;
                        }
                        cached_frames.clear();
                    }

                    continue;
                }

                // Write FLV tag
                if let Err(e) = self.write_flv_tag(data) {
                    error!("Failed to write FLV tag: {}", e);
                    retry_count += 1;
                } else {
                    retry_count = 0;
                }
            } else {
                retry_count += 1;
            }

            if retry_count > 10 {
                break;
            }
        }

        self.unsubscribe_from_stream_hub().await?;
        Ok(())
    }

    fn write_flv_tag(&mut self, frame_data: FrameData) -> anyhow::Result<()> {
        let (data, timestamp, tag_type) = match frame_data {
            FrameData::Audio { timestamp, data } => (data, timestamp, 8), // AUDIO
            FrameData::Video { timestamp, data } => (data, timestamp, 9), // VIDEO
            FrameData::MetaData { timestamp, data } => {
                // Remove @setDataFrame from RTMP's metadata
                let mut amf_writer = Amf0Writer::new();
                amf_writer
                    .write_string(&String::from("@setDataFrame"))
                    .map_err(|e| anyhow::anyhow!("Failed to write AMF string: {:?}", e))?;
                let (_, right) = data.split_at(amf_writer.len());
                (BytesMut::from(right), timestamp, 18) // SCRIPT_DATA_AMF
            }
            _ => return Ok(()),
        };

        let data_len = data.len() as u32;

        self.muxer
            .write_flv_tag_header(tag_type, data_len, timestamp)
            .map_err(|e| anyhow::anyhow!("Failed to write FLV tag header: {:?}", e))?;
        self.muxer
            .write_flv_tag_body(data)
            .map_err(|e| anyhow::anyhow!("Failed to write FLV tag body: {:?}", e))?;
        self.muxer
            .write_previous_tag_size(data_len + HEADER_LENGTH)
            .map_err(|e| anyhow::anyhow!("Failed to write tag size: {:?}", e))?;

        self.flush_response_data()?;

        Ok(())
    }

    fn flush_response_data(&mut self) -> anyhow::Result<()> {
        let data = self.muxer.writer.extract_current_bytes();
        let bytes = bytes::Bytes::from(data.to_vec());

        self.response_producer
            .send(Ok(bytes))
            .map_err(|_| anyhow::anyhow!("Response channel closed"))?;

        Ok(())
    }

    async fn subscribe_from_stream_hub(&mut self) -> anyhow::Result<()> {
        let sub_info = SubscriberInfo {
            id: self.subscriber_id,
            sub_type: SubscribeType::RtmpRemux2HttpFlv,
            sub_data_type: SubDataType::Frame,
            notify_info: NotifyInfo {
                request_url: format!("/live/{}.flv", self.stream_name),
                remote_addr: String::new(),
            },
        };

        let identifier = StreamIdentifier::Rtmp {
            app_name: self.app_name.clone(),
            stream_name: self.stream_name.clone(),
        };

        let (event_result_sender, event_result_receiver) = oneshot::channel();

        let subscribe_event = StreamHubEvent::Subscribe {
            identifier,
            info: sub_info,
            result_sender: event_result_sender,
        };

        self.event_producer
            .send(subscribe_event)
            .map_err(|_| anyhow::anyhow!("Failed to send subscribe event"))?;

        let result = event_result_receiver
            .await
            .map_err(|e| anyhow::anyhow!("Event result channel error: {}", e))?
            .map_err(|e| anyhow::anyhow!("Subscribe failed: {:?}", e))?;
        self.data_receiver = result
            .0
            .frame_receiver
            .ok_or_else(|| anyhow::anyhow!("No frame receiver"))?;

        info!(
            subscriber_id = %self.subscriber_id,
            stream = %self.stream_name,
            "Subscribed to StreamHub"
        );

        Ok(())
    }

    async fn unsubscribe_from_stream_hub(&mut self) -> anyhow::Result<()> {
        let sub_info = SubscriberInfo {
            id: self.subscriber_id,
            sub_type: SubscribeType::RtmpRemux2HttpFlv,
            sub_data_type: SubDataType::Frame,
            notify_info: NotifyInfo {
                request_url: format!("/live/{}.flv", self.stream_name),
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
            warn!("Failed to send unsubscribe event: {}", e);
        }

        info!(
            subscriber_id = %self.subscriber_id,
            stream = %self.stream_name,
            "Unsubscribed from StreamHub"
        );

        Ok(())
    }
}
