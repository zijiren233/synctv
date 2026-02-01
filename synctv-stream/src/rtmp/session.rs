use crate::{
    cache::gop_cache::{GopCache, GopFrame},
    relay::registry::StreamRegistry,
    error::{StreamResult, StreamError},
    rtmp::auth::RtmpAuthCallback,
};
use bytes::BytesMut;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpStream;

pub struct SyncTvRtmpSession {
    tcp_stream: TcpStream,
    remote_addr: SocketAddr,
    gop_cache: Arc<GopCache>,
    registry: StreamRegistry,
    node_id: String,
    auth_callback: Arc<dyn RtmpAuthCallback>,
    room_id: Option<String>,
    is_publisher: bool,
}

impl SyncTvRtmpSession {
    pub fn new(
        tcp_stream: TcpStream,
        remote_addr: SocketAddr,
        gop_cache: Arc<GopCache>,
        registry: StreamRegistry,
        node_id: String,
        auth_callback: Arc<dyn RtmpAuthCallback>,
    ) -> Self {
        Self {
            tcp_stream,
            remote_addr,
            gop_cache,
            registry,
            node_id,
            auth_callback,
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

    async fn handle_publish(&mut self, room_id: String, stream_key: String) -> StreamResult<()> {
        tracing::info!("Handling publish for room {}", room_id);

        // Authenticate publisher
        let channel = self.auth_callback
            .authenticate(&room_id, &stream_key, true)
            .await?;

        tracing::info!(
            "Publisher authenticated for room {}, channel {}",
            channel.room_id,
            channel.channel_name
        );

        // Try to register as publisher
        let registered = self.registry
            .register_publisher(&room_id, &self.node_id, "live", &room_id)
            .await
            .map_err(|e| StreamError::RegistryError(e.to_string()))?;

        if !registered {
            return Err(StreamError::AlreadyPublishing(format!(
                "Room {} already has a publisher",
                room_id
            )));
        }

        self.room_id = Some(room_id.clone());
        self.is_publisher = true;

        tracing::info!("Successfully registered as publisher for room {}", room_id);

        // TODO: Start receiving RTMP frames and add to GOP cache

        Ok(())
    }

    async fn handle_play(&mut self, room_id: String, channel_name: String) -> StreamResult<()> {
        tracing::info!("Handling play for room {}", room_id);

        // Authenticate player
        let channel = self.auth_callback
            .authenticate(&room_id, &channel_name, false)
            .await?;

        tracing::info!(
            "Player authenticated for room {}, channel {}",
            channel.room_id,
            channel.channel_name
        );

        // Check if publisher exists (use fixed media_id "live" for RTMP streams)
        let publisher_info = self.registry
            .get_publisher(&room_id, "live")
            .await
            .map_err(|e| StreamError::RegistryError(e.to_string()))?;

        let publisher_info = publisher_info.ok_or_else(|| {
            StreamError::NoPublisher(format!("No publisher found for room {}", room_id))
        })?;

        self.room_id = Some(room_id.clone());

        tracing::info!(
            "Found publisher for room {} on node {}",
            room_id,
            publisher_info.node_id
        );

        // Send cached GOP frames first for instant playback
        let cached_frames = self.gop_cache.get_frames(&room_id);
        for frame in cached_frames {
            // TODO: Convert GopFrame to RTMP packets and send
            tracing::debug!("Sending cached frame: keyframe={}", frame.is_keyframe);
        }

        // TODO: Subscribe to live frames and forward

        Ok(())
    }

    async fn on_audio_frame(&mut self, timestamp: u32, data: BytesMut) -> StreamResult<()> {
        if let Some(room_id) = &self.room_id {
            if self.is_publisher {
                let frame = GopFrame {
                    timestamp,
                    is_keyframe: false,
                    frame_type: crate::cache::gop_cache::FrameType::Audio,
                    data: data.freeze(),
                };
                self.gop_cache.add_frame(room_id, frame);
            }
        }
        Ok(())
    }

    async fn on_video_frame(&mut self, timestamp: u32, data: BytesMut, is_keyframe: bool) -> StreamResult<()> {
        if let Some(room_id) = &self.room_id {
            if self.is_publisher {
                let frame = GopFrame {
                    timestamp,
                    is_keyframe,
                    frame_type: crate::cache::gop_cache::FrameType::Video,
                    data: data.freeze(),
                };
                self.gop_cache.add_frame(room_id, frame);
            }
        }
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
