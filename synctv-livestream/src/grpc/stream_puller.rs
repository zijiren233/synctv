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

    /// Run the puller with retry logic: connect to remote, pull stream, publish to local `StreamHub`.
    ///
    /// On transient failures (connection refused, stream interrupted), retries with exponential
    /// backoff (1s initial, 30s max, with jitter). Gives up after 10 consecutive failures.
    /// The `PullStreamManager` can recreate the puller on the next viewer request.
    pub async fn run(mut self) -> anyhow::Result<()> {
        const MAX_RETRIES: u32 = 10;
        const INITIAL_BACKOFF_MS: u64 = 1000;
        const MAX_BACKOFF_MS: u64 = 30_000;

        info!(
            room_id = %self.room_id,
            media_id = %self.media_id,
            publisher = %self.publisher_node_addr,
            "Starting gRPC stream puller"
        );

        let mut attempt: u32 = 0;

        loop {
            attempt += 1;

            // 1. Publish to local StreamHub (re-publish on each retry to get a fresh sender)
            let data_sender = match self.publish_to_local_stream_hub().await {
                Ok(sender) => sender,
                Err(e) => {
                    error!(
                        room_id = %self.room_id,
                        attempt = attempt,
                        "Failed to publish to local StreamHub: {e}"
                    );
                    if attempt > MAX_RETRIES {
                        return Err(anyhow::anyhow!(
                            "Gave up after {MAX_RETRIES} retries (last error: publish to StreamHub: {e})"
                        ));
                    }
                    Self::backoff(attempt, INITIAL_BACKOFF_MS, MAX_BACKOFF_MS).await;
                    continue;
                }
            };

            let result = self.connect_and_stream(&data_sender).await;

            // Always clean up local StreamHub before retry or exit
            if let Err(e) = self.unpublish_from_local_stream_hub().await {
                warn!("Failed to unpublish from local StreamHub: {e}");
            }

            match result {
                Ok(()) => {
                    info!(
                        room_id = %self.room_id,
                        media_id = %self.media_id,
                        "Stream ended normally"
                    );
                    return Ok(());
                }
                Err(e) => {
                    if attempt >= MAX_RETRIES {
                        error!(
                            room_id = %self.room_id,
                            media_id = %self.media_id,
                            attempt = attempt,
                            "Gave up after {MAX_RETRIES} retries: {e}"
                        );
                        return Err(anyhow::anyhow!(
                            "Gave up after {MAX_RETRIES} retries (last error: {e})"
                        ));
                    }

                    warn!(
                        room_id = %self.room_id,
                        media_id = %self.media_id,
                        attempt = attempt,
                        max_retries = MAX_RETRIES,
                        "Stream pull failed, retrying: {e}"
                    );

                    Self::backoff(attempt, INITIAL_BACKOFF_MS, MAX_BACKOFF_MS).await;
                }
            }
        }
    }

    /// Connect to remote publisher and stream data to the local `StreamHub`.
    /// Returns `Ok(())` when the stream ends normally, `Err` on connection or protocol failure.
    async fn connect_and_stream(&self, data_sender: &FrameDataSender) -> anyhow::Result<()> {
        let publisher_url = format!("http://{}", self.publisher_node_addr);
        let mut client = StreamRelayServiceClient::connect(publisher_url)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to publisher: {e}"))?;

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

        while let Some(packet_result) = stream.message().await? {
            let packet = packet_result;

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

            if let Err(e) = data_sender.send(frame_data) {
                error!("Failed to send frame to StreamHub: {}", e);
                break;
            }
        }

        Ok(())
    }

    /// Exponential backoff with jitter.
    async fn backoff(attempt: u32, initial_ms: u64, max_ms: u64) {
        let base = initial_ms.saturating_mul(1u64 << attempt.min(16).saturating_sub(1));
        let capped = base.min(max_ms);
        // Add jitter: ±25%
        let jitter = capped / 4;
        let random_offset = u64::from(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos())
            % (jitter * 2 + 1);
        let delay = capped.saturating_sub(jitter) + random_offset;
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
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
            .try_send(publish_event)
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

        if let Err(e) = self.stream_hub_event_sender.try_send(unpublish_event) {
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
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::channel(64);

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
        let (stream_hub_event_sender, _) = tokio::sync::mpsc::channel(64);

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
