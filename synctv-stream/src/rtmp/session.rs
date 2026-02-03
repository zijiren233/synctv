use crate::{
    cache::gop_cache::GopCache,
    relay::registry::StreamRegistry,
    rtmp::auth::RtmpAuthCallback,
    error::StreamResult,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpStream;

pub struct SyncTvRtmpSession {
    remote_addr: SocketAddr,
    registry: StreamRegistry,
    room_id: Option<String>,
    is_publisher: bool,
}

impl SyncTvRtmpSession {
    pub fn new(
        _tcp_stream: TcpStream,
        remote_addr: SocketAddr,
        _gop_cache: Arc<GopCache>,
        registry: StreamRegistry,
        _node_id: String,
        _auth_callback: Arc<dyn RtmpAuthCallback>,
    ) -> Self {
        Self {
            remote_addr,
            registry,
            room_id: None,
            is_publisher: false,
        }
    }

    pub async fn run(&mut self) -> StreamResult<()> {
        // TODO: Integrate xiu's RTMP protocol handling
        // For now, this is a placeholder
        tracing::info!("RTMP session started for {}", self.remote_addr);

        // Parse stream name to extract room_id
        // Format: room_{room_id} or just use stream_name as room_id

        // Validate room exists and user has permission

        // If publishing:
        //   - Register as publisher in Redis
        //   - Start receiving audio/video frames
        //   - Add frames to GOP cache
        //
        // If playing:
        //   - Check if publisher exists
        //   - Subscribe to stream
        //   - Send cached GOPs first
        //   - Forward live frames

        Ok(())
    }
}

impl Drop for SyncTvRtmpSession {
    fn drop(&mut self) {
        if let Some(room_id) = &self.room_id {
            if self.is_publisher {
                // Unregister publisher (use fixed media_id "live" for RTMP streams)
                let room_id = room_id.clone();
                let mut registry = self.registry.clone();
                tokio::spawn(async move {
                    if let Err(e) = registry.unregister_publisher(&room_id, "live").await {
                        tracing::error!("Failed to unregister publisher: {}", e);
                    }
                });
            }
        }
        tracing::info!("RTMP session closed for {}", self.remote_addr);
    }
}
