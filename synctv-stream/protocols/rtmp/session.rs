use crate::{
    libraries::gop_cache::{GopCache, GopFrame, FrameType as GopFrameType},
    relay::{StreamRegistry, registry_trait::StreamRegistryTrait},
    protocols::rtmp::auth::RtmpAuthCallback,
    error::{StreamResult, StreamError},
};
use streamhub::{
    define::{
        FrameData, NotifyInfo, PublishType, PublisherInfo, StreamHubEvent, StreamHubEventSender,
        SubscribeType, SubscriberInfo,
    },
    stream::StreamIdentifier,
    utils::{RandomDigitCount, Uuid},
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tracing::{debug, error, info, warn};

pub struct SyncTvRtmpSession {
    remote_addr: SocketAddr,
    registry: Arc<dyn StreamRegistryTrait>,
    node_id: String,
    auth_callback: Arc<dyn RtmpAuthCallback>,
    stream_hub_event_sender: StreamHubEventSender,
    room_id: Option<String>,
    media_id: Option<String>,
    is_publisher: bool,
}

impl SyncTvRtmpSession {
    pub fn new(
        _tcp_stream: TcpStream,
        remote_addr: SocketAddr,
        _gop_cache: Arc<GopCache>,
        registry: Arc<dyn StreamRegistryTrait>,
        node_id: String,
        auth_callback: Arc<dyn RtmpAuthCallback>,
        stream_hub_event_sender: StreamHubEventSender,
    ) -> Self {
        Self {
            remote_addr,
            registry,
            node_id,
            auth_callback,
            stream_hub_event_sender,
            room_id: None,
            media_id: None,
            is_publisher: false,
        }
    }

    pub async fn run(&mut self) -> StreamResult<()> {
        info!("RTMP session started for {}", self.remote_addr);

        // The actual RTMP protocol handling is done by xiu's rtmp::rtmp::RtmpServer
        // This session is a wrapper for SyncTV-specific logic (auth, registration, etc.)
        // The xiu server will call into this for authentication and event handling

        Ok(())
    }

    /// Authenticate and register publisher
    pub async fn handle_publish(
        &mut self,
        app_name: &str,
        stream_key: &str,
    ) -> StreamResult<()> {
        info!(
            app_name = app_name,
            stream_key = stream_key,
            "RTMP publish request from {}",
            self.remote_addr
        );

        // Authenticate
        let channel = self
            .auth_callback
            .authenticate(app_name, stream_key, true)
            .await
            .map_err(|e| {
                error!("Authentication failed: {}", e);
                StreamError::AuthenticationFailed(e.to_string())
            })?;

        self.room_id = Some(channel.room_id.clone());
        self.media_id = Some(channel.channel_name.clone());
        self.is_publisher = true;

        // Parse stream_key as room_id/media_id
        let (room_id, media_id) = if let Some((r, m)) = stream_key.split_once('/') {
            (r.to_string(), m.to_string())
        } else {
            return Err(StreamError::InvalidStreamKey(
                "Expected format: room_id/media_id".to_string(),
            ));
        };

        // Register as publisher in Redis (atomic HSETNX)
        match self
            .registry
            .try_register_publisher(&room_id, &media_id, &self.node_id)
            .await
        {
            Ok(true) => {
                info!(
                    "Registered as publisher for room {} / media {}",
                    room_id, media_id
                );
            }
            Ok(false) => {
                return Err(StreamError::PublisherExists(format!(
                    "Publisher already exists for room {} / media {}",
                    room_id, media_id
                )));
            }
            Err(e) => {
                error!("Failed to register publisher: {}", e);
                return Err(StreamError::RedisError(format!(
                    "Publisher registration failed: {}",
                    e
                )));
            }
        }

        // Publish to StreamHub
        let _data_sender = self
            .publish_to_stream_hub(&room_id, &media_id)
            .await
            .map_err(|e| {
                error!("Failed to publish to StreamHub: {}", e);
                StreamError::StreamHubError(format!("Publish failed: {}", e))
            })?;

        Ok(())
    }

    /// Handle unpublish
    pub async fn handle_unpublish(&mut self) -> StreamResult<()> {
        let (room_id, media_id) = if let (Some(r), Some(m)) = (self.room_id.take(), self.media_id.take()) {
            (r, m)
        } else {
            return Ok(());
        };

        info!(
            "RTMP unpublish for room {} / media {} from {}",
            room_id, media_id, self.remote_addr
        );

        // Unregister publisher
        if let Err(e) = self.registry.unregister_publisher(&room_id, &media_id).await {
            error!("Failed to unregister publisher: {}", e);
        }

        // Unpublish from StreamHub
        let _ = self.unpublish_from_stream_hub(&room_id, &media_id).await;

        Ok(())
    }

    /// Handle play request
    pub async fn handle_play(
        &mut self,
        app_name: &str,
        stream_key: &str,
    ) -> StreamResult<()> {
        info!(
            app_name = app_name,
            stream_key = stream_key,
            "RTMP play request from {}",
            self.remote_addr
        );

        // Authenticate
        let channel = self
            .auth_callback
            .authenticate(app_name, stream_key, false)
            .await
            .map_err(|e| {
                error!("Authentication failed: {}", e);
                StreamError::AuthenticationFailed(e.to_string())
            })?;

        self.room_id = Some(channel.room_id.clone());
        self.media_id = Some(channel.channel_name.clone());
        self.is_publisher = false;

        // Check if publisher exists
        let publisher_info = self
            .registry
            .get_publisher(&channel.room_id, &channel.channel_name)
            .await
            .map_err(|e| {
                error!("Failed to get publisher info: {}", e);
                StreamError::RedisError(format!("Failed to check publisher: {}", e))
            })?
            .ok_or_else(|| StreamError::NoPublisher(format!(
                "No active publisher for room {} / media {}",
                channel.room_id, channel.channel_name
            )))?;

        info!(
            room_id = channel.room_id,
            media_id = channel.channel_name,
            publisher_node = publisher_info.node_id,
            "Found publisher for play request"
        );

        Ok(())
    }

    /// Publish to StreamHub
    async fn publish_to_stream_hub(
        &mut self,
        room_id: &str,
        media_id: &str,
    ) -> Result<streamhub::define::FrameDataSender, anyhow::Error> {
        let publisher_id = Uuid::new(RandomDigitCount::Four);

        let publisher_info = PublisherInfo {
            id: publisher_id,
            pub_type: PublishType::RtmpPush,
            pub_data_type: streamhub::define::PubDataType::Frame,
            notify_info: NotifyInfo {
                request_url: format!("rtmp://{}/{}/{}", self.remote_addr, room_id, media_id),
                remote_addr: self.remote_addr.to_string(),
            },
        };

        let stream_name = format!("{}/{}", room_id, media_id);
        let identifier = StreamIdentifier::Rtmp {
            app_name: "live".to_string(),
            stream_name: stream_name.clone(),
        };

        let stream_handler = Arc::new(rtmp::session::common::RtmpStreamHandler::new());

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
            .map_err(|e| anyhow::anyhow!("Publish failed: {}", e))?;

        let data_sender = result
            .0
            .ok_or_else(|| anyhow::anyhow!("No data sender from publish result"))?;

        info!(
            room_id = room_id,
            media_id = media_id,
            publisher_id = publisher_id.to_string(),
            "Published to StreamHub successfully"
        );

        Ok(data_sender)
    }

    /// Unpublish from StreamHub
    async fn unpublish_from_stream_hub(
        &mut self,
        room_id: &str,
        media_id: &str,
    ) -> anyhow::Result<()> {
        let stream_name = format!("{}/{}", room_id, media_id);
        let identifier = StreamIdentifier::Rtmp {
            app_name: "live".to_string(),
            stream_name,
        };

        let publisher_info = PublisherInfo {
            id: Uuid::new(RandomDigitCount::Four),
            pub_type: PublishType::RtmpPush,
            pub_data_type: streamhub::define::PubDataType::Frame,
            notify_info: NotifyInfo {
                request_url: format!(
                    "rtmp://{}/{}/{}",
                    self.remote_addr, room_id, media_id
                ),
                remote_addr: self.remote_addr.to_string(),
            },
        };

        let unpublish_event = StreamHubEvent::UnPublish {
            identifier,
            info: publisher_info,
        };

        self.stream_hub_event_sender
            .send(unpublish_event)
            .map_err(|_| anyhow::anyhow!("Failed to send unpublish event"))?;

        Ok(())
    }
}

impl Drop for SyncTvRtmpSession {
    fn drop(&mut self) {
        if self.is_publisher {
            if let (Some(room_id), Some(media_id)) = (self.room_id.take(), self.media_id.take()) {
                let registry = self.registry.clone();
                tokio::spawn(async move {
                    if let Err(e) = registry.unregister_publisher(&room_id, &media_id).await {
                        error!("Failed to unregister publisher: {}", e);
                    }
                });
            }
        }
        info!("RTMP session closed for {}", self.remote_addr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::rtmp::auth::NoAuthCallback;
    use crate::relay::MockStreamRegistry;
    use std::net::SocketAddr;

    // Helper function to create test session without Redis dependency
    async fn create_test_session_async(
        remote_addr: SocketAddr,
        stream_hub_event_sender: StreamHubEventSender,
    ) -> SyncTvRtmpSession {
        // Create a pair of connected sockets for testing
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        let tcp_stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
        drop(listener);

        // Use MockStreamRegistry - no Redis required!
        let registry = Arc::new(MockStreamRegistry::new());

        let gop_cache = Arc::new(GopCache::new(Default::default()));
        let node_id = "test_node".to_string();
        let auth_callback = Arc::new(NoAuthCallback);

        SyncTvRtmpSession::new(
            tcp_stream,
            remote_addr,
            gop_cache,
            registry,
            node_id,
            auth_callback,
            stream_hub_event_sender,
        )
    }

    #[tokio::test]
    async fn test_session_creation() {
        let remote_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let session = create_test_session_async(remote_addr, tx).await;

        assert_eq!(session.remote_addr, remote_addr);
        assert!(!session.is_publisher);
        assert!(session.room_id.is_none());
        assert!(session.media_id.is_none());
    }

    #[tokio::test]
    async fn test_session_run_returns_ok() {
        let remote_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let mut session = create_test_session_async(remote_addr, tx).await;

        // run() should return Ok since it's just a wrapper
        let result = session.run().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_channel_struct_from_auth() {
        use crate::protocols::rtmp::auth::Channel;
        let channel = Channel {
            room_id: "test_room".to_string(),
            channel_name: "test_movie".to_string(),
            is_publisher: true,
        };

        assert_eq!(channel.room_id, "test_room");
        assert_eq!(channel.channel_name, "test_movie");
        assert!(channel.is_publisher);
    }
}
