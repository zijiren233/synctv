use crate::{
    cache::gop_cache::GopCache,
    relay::registry::StreamRegistry,
    error::StreamResult,
};
use std::sync::Arc;
use tokio::net::TcpListener;
use xrtmp::rtmp::RtmpServer;

pub struct RtmpStreamingServer {
    rtmp_address: String,
    gop_cache: Arc<GopCache>,
    registry: StreamRegistry,
    node_id: String,
}

impl RtmpStreamingServer {
    pub fn new(
        rtmp_address: String,
        gop_cache: Arc<GopCache>,
        registry: StreamRegistry,
        node_id: String,
    ) -> Self {
        Self {
            rtmp_address,
            gop_cache,
            registry,
            node_id,
        }
    }

    pub async fn start(&mut self) -> StreamResult<()> {
        let socket_addr: std::net::SocketAddr = self.rtmp_address.parse()
            .map_err(|e| crate::error::StreamError::InvalidAddress(format!("Invalid RTMP address: {}", e)))?;

        let listener = TcpListener::bind(socket_addr).await
            .map_err(|e| crate::error::StreamError::IoError(e))?;

        log::info!("RTMP server listening on rtmp://{}", socket_addr);

        loop {
            let (tcp_stream, remote_addr) = listener.accept().await
                .map_err(|e| crate::error::StreamError::IoError(e))?;

            log::info!("New RTMP connection from {}", remote_addr);

            let gop_cache = Arc::clone(&self.gop_cache);
            let registry = self.registry.clone();
            let node_id = self.node_id.clone();

            tokio::spawn(async move {
                let mut session = crate::rtmp::session::SyncTvRtmpSession::new(
                    tcp_stream,
                    remote_addr,
                    gop_cache,
                    registry,
                    node_id,
                );

                if let Err(err) = session.run().await {
                    log::error!(
                        "RTMP session error from {}: {}",
                        remote_addr,
                        err
                    );
                }
            });
        }
    }
}
