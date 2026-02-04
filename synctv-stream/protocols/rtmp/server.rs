use crate::{
    libraries::gop_cache::GopCache,
    relay::registry_trait::StreamRegistryTrait,
    error::StreamResult,
    protocols::rtmp::auth::RtmpAuthCallback,
};
use std::sync::Arc;
use streamhub::define::StreamHubEventSender;
use tokio::net::TcpListener;

pub struct RtmpStreamingServer {
    rtmp_address: String,
    gop_cache: Arc<GopCache>,
    registry: Arc<dyn StreamRegistryTrait>,
    node_id: String,
    auth_callback: Arc<dyn RtmpAuthCallback>,
    stream_hub_event_sender: StreamHubEventSender,
}

impl RtmpStreamingServer {
    pub fn new(
        rtmp_address: String,
        gop_cache: Arc<GopCache>,
        registry: Arc<dyn StreamRegistryTrait>,
        node_id: String,
        auth_callback: Arc<dyn RtmpAuthCallback>,
        stream_hub_event_sender: StreamHubEventSender,
    ) -> Self {
        Self {
            rtmp_address,
            gop_cache,
            registry,
            node_id,
            auth_callback,
            stream_hub_event_sender,
        }
    }

    pub async fn start(&mut self) -> StreamResult<()> {
        let socket_addr: std::net::SocketAddr = self.rtmp_address.parse()
            .map_err(|e| crate::error::StreamError::InvalidAddress(format!("Invalid RTMP address: {e}")))?;

        let listener = TcpListener::bind(socket_addr).await
            .map_err(crate::error::StreamError::IoError)?;

        tracing::info!("RTMP server listening on rtmp://{}", socket_addr);

        loop {
            let (tcp_stream, remote_addr) = listener.accept().await
                .map_err(crate::error::StreamError::IoError)?;

            tracing::info!("New RTMP connection from {}", remote_addr);

            let gop_cache = Arc::clone(&self.gop_cache);
            let registry = self.registry.clone();
            let node_id = self.node_id.clone();
            let auth_callback = Arc::clone(&self.auth_callback);
            let stream_hub_event_sender = self.stream_hub_event_sender.clone();

            tokio::spawn(async move {
                let mut session = crate::protocols::rtmp::session::SyncTvRtmpSession::new(
                    tcp_stream,
                    remote_addr,
                    gop_cache,
                    registry,
                    node_id,
                    auth_callback,
                    stream_hub_event_sender,
                );

                if let Err(err) = session.run().await {
                    tracing::error!(
                        "RTMP session error from {}: {}",
                        remote_addr,
                        err
                    );
                }
            });
        }
    }
}
