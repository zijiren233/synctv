use anyhow::Result;
use std::sync::Arc;
use tracing::{info, warn};
use bytes::Bytes;
use tokio::sync::mpsc;
use streamhub::define::{StreamHubEventSender};
use crate::libraries::{GopCache, GopFrame};
use crate::relay::StreamRegistryTrait;
use crate::grpc::{FrameType, RtmpPacket, PullRtmpStreamRequest, StreamRelayService};
use tonic::{Request, Response, Status};
use tokio_stream::wrappers::UnboundedReceiverStream;
use std::collections::HashMap;

/// Publisher node - accepts RTMP push and serves to Pullers
///
/// Note: In the xiu architecture, frame data distribution to local subscribers
/// is handled automatically by the `StreamHub` when a session publishes.
/// The Publisher here primarily handles:
/// 1. Caching frames in GOP cache for fast startup
/// 2. Broadcasting to gRPC relay clients (Puller nodes on other servers)
pub struct Publisher {
    room_id: String,
    media_id: String,
    node_id: String,
    gop_cache: Arc<GopCache>,
    registry: Arc<dyn StreamRegistryTrait>,
    /// Event sender for `StreamHub` (for Publish/UnPublish events)
    stream_hub_sender: Option<StreamHubEventSender>,
    /// Channel for gRPC relay to Puller nodes
    relay_senders: Arc<tokio::sync::Mutex<HashMap<String, mpsc::UnboundedSender<Result<RtmpPacket, Status>>>>>,
}

impl Publisher {
    /// Create a new Publisher
    pub fn new(
        room_id: String,
        media_id: String,
        node_id: String,
        gop_cache: Arc<GopCache>,
        registry: Arc<dyn StreamRegistryTrait>,
        stream_hub_sender: Option<StreamHubEventSender>,
    ) -> Self {
        Self {
            room_id,
            media_id,
            node_id,
            gop_cache,
            registry,
            stream_hub_sender,
            relay_senders: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Handle incoming RTMP data
    /// This will be called by xiu when data is received
    pub async fn on_rtmp_data(
        &mut self,
        data: Bytes,
        timestamp: u32,
        is_keyframe: bool,
        is_video: bool,
    ) -> Result<()> {
        // Add frame to GOP cache for fast viewer startup
        let frame = GopFrame {
            data: data.clone(),
            timestamp,
            is_keyframe,
            frame_type: if is_video {
                crate::libraries::FrameType::Video
            } else {
                crate::libraries::FrameType::Audio
            },
        };

        // Use composite key "room_id:media_id" for GOP cache
        let stream_key = format!("{}:{}", self.room_id, self.media_id);
        self.gop_cache.add_frame(&stream_key, frame);

        // Note: Local viewer distribution is handled by xiu's StreamHub automatically
        // The RTMP session publishes to StreamHub, which distributes to all subscribers

        // Send to gRPC stream relay service for Puller nodes on other servers
        self.broadcast_to_grpc_relays(data, timestamp, is_keyframe, is_video).await?;

        Ok(())
    }

    /// Broadcast frame to gRPC relay clients (Puller nodes on other servers)
    async fn broadcast_to_grpc_relays(
        &self,
        data: Bytes,
        timestamp: u32,
        is_keyframe: bool,
        is_video: bool,
    ) -> Result<()> {
        let frame_type = if is_video {
            FrameType::Video
        } else if is_keyframe {
            FrameType::Metadata
        } else {
            FrameType::Audio
        };

        let packet = RtmpPacket {
            frame_type: frame_type as i32,
            timestamp,
            data: data.to_vec(),
        };

        let mut senders = self.relay_senders.lock().await;
        let mut dead_clients = Vec::new();

        for (id, sender) in senders.iter() {
            if let Err(_) = sender.send(Ok(packet.clone())) {
                dead_clients.push(id.clone());
            }
        }

        // Remove disconnected clients
        for id in dead_clients {
            senders.remove(&id);
            info!("Removed disconnected gRPC relay client: {}", id);
        }

        Ok(())
    }

    /// Register a new gRPC relay client
    pub async fn register_relay_client(&self, client_id: String) -> mpsc::UnboundedReceiver<Result<RtmpPacket, Status>> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut senders = self.relay_senders.lock().await;
        senders.insert(client_id.clone(), tx);
        info!("Registered gRPC relay client: {}", client_id);
        rx
    }

    /// Unregister a gRPC relay client
    pub async fn unregister_relay_client(&self, client_id: &str) {
        let mut senders = self.relay_senders.lock().await;
        senders.remove(client_id);
        info!("Unregistered gRPC relay client: {}", client_id);
    }

    /// Stop publishing and cleanup
    pub async fn stop(&mut self) -> Result<()> {
        info!(
            room_id = %self.room_id,
            media_id = %self.media_id,
            node_id = %self.node_id,
            "Publisher stopping"
        );

        // Unregister from Redis
        self.registry.unregister_publisher(&self.room_id, &self.media_id).await?;

        // Clear GOP cache
        let stream_key = format!("{}:{}", self.room_id, self.media_id);
        self.gop_cache.clear_stream(&stream_key);

        // Clear all relay clients
        let mut senders = self.relay_senders.lock().await;
        senders.clear();

        Ok(())
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        warn!(
            room_id = %self.room_id,
            "Publisher dropped - may need manual cleanup"
        );
    }
}

/// gRPC service implementation for stream relay
/// This is used by Puller nodes to connect and receive stream data
pub struct PublisherRelayService {
    publishers: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<Publisher>>>>>,
}

impl PublisherRelayService {
    #[must_use] 
    pub fn new() -> Self {
        Self {
            publishers: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    pub async fn register_publisher(&self, key: String, publisher: Arc<tokio::sync::Mutex<Publisher>>) {
        let mut publishers = self.publishers.lock().await;
        publishers.insert(key, publisher);
    }

    pub async fn unregister_publisher(&self, key: &str) {
        let mut publishers = self.publishers.lock().await;
        publishers.remove(key);
    }
}

impl Default for PublisherRelayService {
    fn default() -> Self {
        Self::new()
    }
}

#[tonic::async_trait]
impl StreamRelayService for PublisherRelayService {
    type PullRtmpStreamStream = UnboundedReceiverStream<Result<RtmpPacket, Status>>;

    async fn pull_rtmp_stream(
        &self,
        request: Request<PullRtmpStreamRequest>,
    ) -> Result<Response<Self::PullRtmpStreamStream>, Status> {
        let req = request.into_inner();

        let key = format!("{}:{}", req.room_id, req.media_id);
        let publishers = self.publishers.lock().await;

        let publisher = publishers.get(&key)
            .ok_or_else(|| Status::not_found(format!("No publisher for {key}")))?;

        let publisher = publisher.clone();
        drop(publishers);

        // Create a unique client ID for this connection
        let client_id = nanoid::nanoid!();

        // Register relay client (now using tokio::sync::Mutex which is Send-safe)
        let publisher = publisher.lock().await;
        let receiver = publisher.register_relay_client(client_id.clone()).await;

        let stream = UnboundedReceiverStream::new(receiver);

        Ok(Response::new(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::libraries::GopCacheConfig;
    use crate::relay::MockStreamRegistry;

    #[tokio::test]
    async fn test_publisher_creation() {
        let gop_cache = Arc::new(GopCache::new(GopCacheConfig::default()));
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;

        let publisher = Publisher::new(
            "room123".to_string(),
            "media123".to_string(),
            "node1".to_string(),
            gop_cache,
            registry,
            None,
        );

        assert_eq!(publisher.room_id, "room123");
        assert_eq!(publisher.node_id, "node1");
    }

    #[tokio::test]
    async fn test_publisher_on_rtmp_data() {
        let gop_cache = Arc::new(GopCache::new(GopCacheConfig::default()));
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;

        let mut publisher = Publisher::new(
            "room123".to_string(),
            "media123".to_string(),
            "node1".to_string(),
            gop_cache,
            registry,
            None,
        );

        let test_data = Bytes::from(&b"test video data"[..]);

        // Should not panic
        let result = publisher.on_rtmp_data(test_data, 1000, true, true).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_relay_client_registration() {
        let gop_cache = Arc::new(GopCache::new(GopCacheConfig::default()));
        let registry = Arc::new(MockStreamRegistry::new()) as Arc<dyn StreamRegistryTrait>;

        let publisher = Publisher::new(
            "room123".to_string(),
            "media123".to_string(),
            "node1".to_string(),
            gop_cache,
            registry,
            None,
        );

        // Test client registration
        let client_id = "test_client".to_string();
        let _rx = publisher.register_relay_client(client_id.clone()).await;

        // Verify client is registered
        let senders = publisher.relay_senders.lock().await;
        assert!(senders.contains_key(&client_id));
        drop(senders);

        // Test unregistration
        publisher.unregister_relay_client(&client_id).await;

        // Verify client is removed
        let senders = publisher.relay_senders.lock().await;
        assert!(!senders.contains_key(&client_id));
    }
}
