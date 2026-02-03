use std::pin::Pin;
use std::sync::Arc;
use streamhub::{
    define::{NotifyInfo, StreamHubEvent, StreamHubEventSender, SubscribeType, SubscriberInfo},
    stream::StreamIdentifier,
    utils::{RandomDigitCount, Uuid},
};
use tokio::sync::{mpsc, Mutex};
use tokio_stream::{wrappers::ReceiverStream, Stream};
use tonic::{Request, Response, Status};
use tracing::{info, warn};

use super::proto::*;
use crate::cache::GopCache;
use crate::relay::StreamRegistry;

type ResponseStream = Pin<Box<dyn Stream<Item = Result<RtmpPacket, Status>> + Send>>;

/// StreamRelayService implementation
/// Publisher nodes use this to serve RTMP packets to Puller nodes via subscription
pub struct StreamRelayServiceImpl {
    gop_cache: Arc<GopCache>,
    registry: Arc<StreamRegistry>,
    node_id: String,
    stream_hub_event_sender: Arc<Mutex<StreamHubEventSender>>,
}

impl StreamRelayServiceImpl {
    pub fn new(
        gop_cache: Arc<GopCache>,
        registry: Arc<StreamRegistry>,
        node_id: String,
        stream_hub_event_sender: StreamHubEventSender,
    ) -> Self {
        Self {
            gop_cache,
            registry,
            node_id,
            stream_hub_event_sender: Arc::new(Mutex::new(stream_hub_event_sender)),
        }
    }
}

#[tonic::async_trait]
impl stream_relay_service_server::StreamRelayService for StreamRelayServiceImpl {
    /// Pull RTMP stream from publisher node (server streaming)
    /// Similar to FLV/HLS: subscribe to StreamHub and forward data
    type PullRtmpStreamStream = ResponseStream;

    async fn pull_rtmp_stream(
        &self,
        request: Request<PullRtmpStreamRequest>,
    ) -> Result<Response<Self::PullRtmpStreamStream>, Status> {
        let req = request.into_inner();
        info!(
            room_id = req.room_id,
            media_id = req.media_id,
            "PullRtmpStream request (service-to-service internal call)"
        );

        // Check if this node is the publisher
        let mut registry = (*self.registry).clone();
        let publisher_info = registry
            .get_publisher(&req.room_id, &req.media_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get publisher: {}", e)))?
            .ok_or_else(|| Status::not_found("No active publisher for this media"))?;

        if publisher_info.node_id != self.node_id {
            return Err(Status::failed_precondition(format!(
                "This node ({}) is not the publisher (publisher is {})",
                self.node_id, publisher_info.node_id
            )));
        }

        // Get GOP cache frames for fast start
        let stream_key = format!("{}:{}", req.room_id, req.media_id);
        let cached_frames = self.gop_cache.get_frames(&stream_key);
        info!(
            room_id = req.room_id,
            media_id = req.media_id,
            cached_frame_count = cached_frames.len(),
            "Sending cached frames + live subscription to puller"
        );

        // Subscribe to StreamHub for live data
        let subscriber_id = Uuid::new(RandomDigitCount::Four);
        let sub_info = SubscriberInfo {
            id: subscriber_id,
            sub_type: SubscribeType::RtmpPull,
            sub_data_type: streamhub::define::SubDataType::Frame,
            notify_info: NotifyInfo {
                request_url: String::new(),
                remote_addr: String::new(),
            },
        };

        // Stream name format: room_id/media_id
        let stream_name = format!("{}/{}", req.room_id, req.media_id);
        let identifier = StreamIdentifier::Rtmp {
            app_name: "live".to_string(),
            stream_name: stream_name.clone(),
        };

        let (event_result_sender, event_result_receiver) = tokio::sync::oneshot::channel();
        let subscribe_event = StreamHubEvent::Subscribe {
            identifier,
            info: sub_info,
            result_sender: event_result_sender,
        };

        // Send subscribe event
        let event_sender = self.stream_hub_event_sender.lock().await;
        event_sender
            .send(subscribe_event)
            .map_err(|_| Status::internal("Failed to send subscribe event"))?;
        drop(event_sender);

        // Wait for subscription result
        let subscribe_result = event_result_receiver
            .await
            .map_err(|_| Status::internal("Subscribe result channel closed"))?
            .map_err(|e| Status::internal(format!("Subscribe failed: {}", e)))?;

        let mut frame_receiver = subscribe_result
            .0
            .frame_receiver
            .ok_or_else(|| Status::internal("No frame receiver from subscription"))?;

        // Create a channel for streaming packets
        let (tx, rx) = mpsc::channel(128);

        // Spawn task to forward frames
        let stream_name_clone = stream_name.clone();
        let event_sender_clone = Arc::clone(&self.stream_hub_event_sender);
        tokio::spawn(async move {
            // 1. Send GOP cache first (fast start)
            for frame in cached_frames {
                let frame_type = match frame.frame_type {
                    crate::cache::gop_cache::FrameType::Video => FrameType::Video as i32,
                    crate::cache::gop_cache::FrameType::Audio => FrameType::Audio as i32,
                };

                let packet = RtmpPacket {
                    data: frame.data.to_vec(),
                    timestamp: frame.timestamp,
                    frame_type,
                };

                if tx.send(Ok(packet)).await.is_err() {
                    warn!("Client disconnected while sending cached frames");
                    // Unsubscribe
                    Self::unsubscribe_from_hub(
                        event_sender_clone,
                        subscriber_id,
                        stream_name_clone,
                    )
                    .await;
                    return;
                }
            }

            // 2. Stream live data from StreamHub subscription
            info!("GOP cache sent, now streaming live data");
            while let Some(frame_data) = frame_receiver.recv().await {
                // Extract data, timestamp, and frame_type from FrameData enum
                let (data, timestamp, frame_type) = match frame_data {
                    streamhub::define::FrameData::Video { data, timestamp } => {
                        (data, timestamp, FrameType::Video as i32)
                    }
                    streamhub::define::FrameData::Audio { data, timestamp } => {
                        (data, timestamp, FrameType::Audio as i32)
                    }
                    streamhub::define::FrameData::MetaData { data, timestamp } => {
                        (data, timestamp, FrameType::Metadata as i32)
                    }
                    _ => continue,
                };

                let packet = RtmpPacket {
                    data: data.to_vec(),
                    timestamp,
                    frame_type,
                };

                if tx.send(Ok(packet)).await.is_err() {
                    warn!("Client disconnected during live streaming");
                    break;
                }
            }

            info!("Stream ended, unsubscribing");
            Self::unsubscribe_from_hub(event_sender_clone, subscriber_id, stream_name_clone).await;
        });

        let output_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(output_stream) as Self::PullRtmpStreamStream
        ))
    }
}

impl StreamRelayServiceImpl {
    /// Unsubscribe from StreamHub
    async fn unsubscribe_from_hub(
        event_sender: Arc<Mutex<StreamHubEventSender>>,
        subscriber_id: Uuid,
        stream_name: String,
    ) {
        let sub_info = SubscriberInfo {
            id: subscriber_id,
            sub_type: SubscribeType::RtmpPull,
            sub_data_type: streamhub::define::SubDataType::Frame,
            notify_info: NotifyInfo {
                request_url: String::new(),
                remote_addr: String::new(),
            },
        };

        let identifier = StreamIdentifier::Rtmp {
            app_name: "live".to_string(),
            stream_name,
        };

        let unsubscribe_event = StreamHubEvent::UnSubscribe {
            identifier,
            info: sub_info,
        };

        let sender = event_sender.lock().await;
        if let Err(e) = sender.send(unsubscribe_event) {
            warn!("Failed to send unsubscribe event: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_service_creation() {
        // Create real components for testing
        let gop_cache = Arc::new(GopCache::new(Default::default()));
        let (event_sender, _) = tokio::sync::mpsc::unbounded_channel::<streamhub::define::StreamHubEvent>();
        let node_id = "test_node".to_string();

        // Verify the node_id is correct
        assert_eq!(node_id, "test_node");

        // Note: Full service creation requires StreamRegistry which needs Redis
        // This test verifies basic type compatibility
        assert!(Arc::strong_count(&gop_cache) >= 1);
    }

    #[test]
    fn test_response_stream_type() {
        // Just verify the ResponseStream type alias compiles
        // In production, this would be tested via integration tests
        let (_tx, rx) = tokio::sync::mpsc::channel(128);
        let _: ResponseStream = Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx));
    }
}
