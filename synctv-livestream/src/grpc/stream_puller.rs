use std::sync::Arc;
use bytes::BytesMut;
use synctv_xiu::streamhub::{
    define::{
        FrameData, FrameDataSender, NotifyInfo, PublishType, PublisherInfo, StreamHubEvent,
        StreamHubEventSender
    },
    stream::StreamIdentifier,
    utils::{RandomDigitCount, Uuid},
};
use tokio::sync::oneshot;
use tonic::Request;
use tracing::{error, info, warn};
use synctv_xiu::rtmp::session::common::RtmpStreamHandler;

use super::proto::{stream_relay_service_client::StreamRelayServiceClient, PullRtmpStreamRequest};
use crate::relay::StreamRegistryTrait;

/// gRPC Stream Puller
/// Pulls RTMP stream from remote Publisher node via gRPC and publishes to local `StreamHub`
pub struct GrpcStreamPuller {
    room_id: String,
    media_id: String,
    publisher_node_addr: String,
    stream_hub_event_sender: StreamHubEventSender,
}

impl GrpcStreamPuller {
    pub fn new(
        room_id: String,
        media_id: String,
        publisher_node_addr: String,
        _node_id: String,
        stream_hub_event_sender: StreamHubEventSender,
        _registry: Arc<dyn StreamRegistryTrait>,
    ) -> Self {
        Self {
            room_id,
            media_id,
            publisher_node_addr,
            stream_hub_event_sender,
        }
    }

    /// Run the puller: connect to remote, pull stream, publish to local `StreamHub`
    pub async fn run(mut self) -> anyhow::Result<()> {
        info!(
            room_id = %self.room_id,
            media_id = %self.media_id,
            publisher = %self.publisher_node_addr,
            "Starting gRPC stream puller"
        );

        // 1. Publish to local StreamHub first (similar to xiu ClientSession::publish_to_stream_hub)
        let data_sender = self.publish_to_local_stream_hub().await?;

        // 2. Connect to remote Publisher via gRPC
        let publisher_url = format!("http://{}", self.publisher_node_addr);
        let mut client = StreamRelayServiceClient::connect(publisher_url)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to publisher: {e}"))?;

        // 3. Pull RTMP stream
        let request = Request::new(PullRtmpStreamRequest {
            room_id: self.room_id.clone(),
            media_id: self.media_id.clone(),
        });

        let mut stream = client
            .pull_rtmp_stream(request)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to pull stream: {e}"))?
            .into_inner();

        info!("Connected to remote publisher, receiving stream data");

        // 4. Receive packets and forward to local StreamHub
        while let Some(packet_result) = stream.message().await? {
            let packet = packet_result;

            // Convert RtmpPacket to FrameData
            let frame_data = match packet.frame_type {
                1 => FrameData::Video {
                    timestamp: packet.timestamp,
                    data: BytesMut::from(&packet.data[..]),
                },
                2 => FrameData::Audio {
                    timestamp: packet.timestamp,
                    data: BytesMut::from(&packet.data[..]),
                },
                3 => FrameData::MetaData {
                    timestamp: packet.timestamp,
                    data: BytesMut::from(&packet.data[..]),
                },
                _ => {
                    warn!("Unknown frame type: {}", packet.frame_type);
                    continue;
                }
            };

            // Send to local StreamHub
            if let Err(e) = data_sender.send(frame_data) {
                error!("Failed to send frame to StreamHub: {}", e);
                break;
            }
        }

        info!("Stream ended, unpublishing from local StreamHub");
        self.unpublish_from_local_stream_hub().await?;

        Ok(())
    }

    /// Publish to local `StreamHub` (similar to xiu `ClientSession::publish_to_stream_hub`)
    async fn publish_to_local_stream_hub(&mut self) -> anyhow::Result<FrameDataSender> {
        let publisher_id = Uuid::new(RandomDigitCount::Four);

        let publisher_info = PublisherInfo {
            id: publisher_id,
            pub_type: PublishType::RtmpRelay,  // Using RtmpRelay for inter-node streaming
            pub_data_type: synctv_xiu::streamhub::define::PubDataType::Frame,
            notify_info: NotifyInfo {
                request_url: format!("grpc://{}/{}/{}", self.publisher_node_addr, self.room_id, self.media_id),
                remote_addr: self.publisher_node_addr.clone(),
            },
        };

        // Stream name format: room_id/media_id
        let stream_name = format!("{}/{}", self.room_id, self.media_id);
        let identifier = StreamIdentifier::Rtmp {
            app_name: "live".to_string(),
            stream_name: stream_name.clone(),
        };

        let stream_handler = Arc::new(RtmpStreamHandler::new());

        let (event_result_sender, event_result_receiver) = oneshot::channel();
        let publish_event = StreamHubEvent::Publish {
            identifier,
            info: publisher_info,
            stream_handler,
            result_sender: event_result_sender,
        };

        self.stream_hub_event_sender
            .send(publish_event)
            .map_err(|_| anyhow::anyhow!("Failed to send publish event"))?;

        let result = event_result_receiver
            .await
            .map_err(|_| anyhow::anyhow!("Publish result channel closed"))?
            .map_err(|e| anyhow::anyhow!("Publish failed: {e}"))?;

        let data_sender = result
            .0
            .ok_or_else(|| anyhow::anyhow!("No data sender from publish result"))?;

        info!("Successfully published to local StreamHub");
        Ok(data_sender)
    }

    /// Unpublish from local `StreamHub`
    async fn unpublish_from_local_stream_hub(&mut self) -> anyhow::Result<()> {
        let stream_name = format!("{}/{}", self.room_id, self.media_id);
        let identifier = StreamIdentifier::Rtmp {
            app_name: "live".to_string(),
            stream_name,
        };

        let unpublish_event = StreamHubEvent::UnPublish { identifier };

        if let Err(e) = self.stream_hub_event_sender.send(unpublish_event) {
            warn!("Failed to send unpublish event: {}", e);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::MockStreamRegistry;

    #[tokio::test]
    async fn test_puller_creation() {
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let puller = GrpcStreamPuller::new(
            "room123".to_string(),
            "media456".to_string(),
            "publisher-node:50051".to_string(),
            "node-1".to_string(),
            stream_hub_event_sender,
            std::sync::Arc::new(MockStreamRegistry::new()),
        );

        assert_eq!(puller.room_id, "room123");
        assert_eq!(puller.media_id, "media456");
        assert_eq!(puller.publisher_node_addr, "publisher-node:50051");
    }

    #[tokio::test]
    async fn test_puller_run() {
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::unbounded_channel();

        let puller = GrpcStreamPuller::new(
            "room123".to_string(),
            "media456".to_string(),
            "localhost:50051".to_string(),
            "node-1".to_string(),
            stream_hub_event_sender,
            std::sync::Arc::new(MockStreamRegistry::new()),
        );

        // This will fail to connect since there's no actual publisher
        let result = puller.run().await;
        assert!(result.is_err());
    }
}
