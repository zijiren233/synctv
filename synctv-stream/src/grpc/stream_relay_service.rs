use std::pin::Pin;
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream};
use tracing::{info, warn, error};

use crate::cache::{GopCache, FrameType};
use crate::relay::StreamRegistry;
use super::proto::*;

type ResponseStream = Pin<Box<dyn Stream<Item = Result<RtmpPacket, Status>> + Send>>;

/// StreamRelayService implementation
/// Publisher nodes use this to serve RTMP packets to Puller nodes
pub struct StreamRelayServiceImpl {
    gop_cache: Arc<GopCache>,
    registry: Arc<StreamRegistry>,
    node_id: String,
}

impl StreamRelayServiceImpl {
    pub fn new(
        gop_cache: Arc<GopCache>,
        registry: Arc<StreamRegistry>,
        node_id: String,
    ) -> Self {
        Self {
            gop_cache,
            registry,
            node_id,
        }
    }
}

#[tonic::async_trait]
impl stream_relay_service_server::StreamRelayService for StreamRelayServiceImpl {
    /// Get publisher information for a room
    async fn get_publisher(
        &self,
        request: Request<GetPublisherRequest>,
    ) -> Result<Response<GetPublisherResponse>, Status> {
        let req = request.into_inner();
        info!(room_id = req.room_id, "GetPublisher request");

        // Query Redis for publisher info
        let mut registry = self.registry.as_ref().clone();
        match registry.get_publisher(&req.room_id).await {
            Ok(Some(pub_info)) => {
                let response = GetPublisherResponse {
                    publisher: Some(PublisherInfo {
                        room_id: req.room_id.clone(),
                        node_id: pub_info.node_id.clone(),
                        stream_key: format!("{}?token=xxx", req.room_id),
                        metadata: None, // TODO: Parse from metadata JSON
                        started_at: pub_info.started_at.timestamp(),
                        stats: None, // TODO: Get live stats
                    }),
                    exists: true,
                };
                Ok(Response::new(response))
            }
            Ok(None) => {
                let response = GetPublisherResponse {
                    publisher: None,
                    exists: false,
                };
                Ok(Response::new(response))
            }
            Err(e) => {
                error!(error = %e, "Failed to get publisher");
                Err(Status::internal(format!("Failed to get publisher: {}", e)))
            }
        }
    }

    /// Pull RTMP stream from publisher node (server streaming)
    type PullRtmpStreamStream = ResponseStream;

    async fn pull_rtmp_stream(
        &self,
        request: Request<PullRtmpStreamRequest>,
    ) -> Result<Response<Self::PullRtmpStreamStream>, Status> {
        let req = request.into_inner();
        info!(
            room_id = req.room_id,
            stream_key = req.stream_key,
            "PullRtmpStream request"
        );

        // Check if this node is the publisher
        let mut registry = (*self.registry).clone();
        let publisher_info = registry
            .get_publisher(&req.room_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get publisher: {}", e)))?
            .ok_or_else(|| Status::not_found("No active publisher for this room"))?;

        if publisher_info.node_id != self.node_id {
            return Err(Status::failed_precondition(format!(
                "This node ({}) is not the publisher (publisher is {})",
                self.node_id, publisher_info.node_id
            )));
        }

        // Get GOP cache frames
        let cached_frames = self.gop_cache.get_frames(&req.room_id);
        info!(
            room_id = req.room_id,
            cached_frame_count = cached_frames.len(),
            "Sending cached frames to puller"
        );

        // Create a channel for streaming packets
        let (tx, rx) = mpsc::channel(128);

        // Send cached frames first (for fast start)
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            for frame in cached_frames {
                let packet = RtmpPacket {
                    r#type: match frame.frame_type {
                        FrameType::Video => rtmp_packet::PacketType::Video,
                        FrameType::Audio => rtmp_packet::PacketType::Audio,
                    } as i32,
                    timestamp: frame.timestamp as i64,
                    pts: frame.timestamp as i64,
                    payload: frame.data.to_vec(),
                    is_keyframe: frame.is_keyframe,
                };

                if tx_clone.send(Ok(packet)).await.is_err() {
                    warn!("Client disconnected while sending cached frames");
                    break;
                }
            }

            // TODO: Subscribe to live frame updates and continue streaming
            // For now, close the stream after sending cached frames
            info!("Cached frames sent, ending stream (live streaming not yet implemented)");
        });

        let output_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(output_stream) as Self::PullRtmpStreamStream
        ))
    }

    /// Register as publisher for a stream
    async fn register_publisher(
        &self,
        request: Request<RegisterPublisherRequest>,
    ) -> Result<Response<RegisterPublisherResponse>, Status> {
        let req = request.into_inner();
        info!(
            room_id = req.room_id,
            node_id = req.node_id,
            "RegisterPublisher request"
        );

        // Attempt atomic registration
        let mut registry = (*self.registry).clone();
        match registry
            .register_publisher(&req.room_id, &req.node_id, "live")
            .await
        {
            Ok(true) => {
                // Registration successful
                let response = RegisterPublisherResponse {
                    success: true,
                    publisher: Some(PublisherInfo {
                        room_id: req.room_id.clone(),
                        node_id: req.node_id.clone(),
                        stream_key: req.stream_key.clone(),
                        metadata: req.metadata,
                        started_at: chrono::Utc::now().timestamp(),
                        stats: None,
                    }),
                    error: String::new(),
                };
                Ok(Response::new(response))
            }
            Ok(false) => {
                // Already registered by another node
                let response = RegisterPublisherResponse {
                    success: false,
                    publisher: None,
                    error: "Stream already being published by another node".to_string(),
                };
                Ok(Response::new(response))
            }
            Err(e) => {
                error!(error = %e, "Failed to register publisher");
                Err(Status::internal(format!("Failed to register publisher: {}", e)))
            }
        }
    }

    /// Unregister publisher
    async fn unregister_publisher(
        &self,
        request: Request<UnregisterPublisherRequest>,
    ) -> Result<Response<UnregisterPublisherResponse>, Status> {
        let req = request.into_inner();
        info!(room_id = req.room_id, node_id = req.node_id, "UnregisterPublisher request");

        let mut registry = (*self.registry).clone();
        match registry.unregister_publisher(&req.room_id).await {
            Ok(_) => {
                let response = UnregisterPublisherResponse { success: true };
                Ok(Response::new(response))
            }
            Err(e) => {
                error!(error = %e, "Failed to unregister publisher");
                Err(Status::internal(format!("Failed to unregister publisher: {}", e)))
            }
        }
    }

    /// Get stream statistics
    async fn get_stream_stats(
        &self,
        request: Request<GetStreamStatsRequest>,
    ) -> Result<Response<GetStreamStatsResponse>, Status> {
        let req = request.into_inner();

        let mut registry = (*self.registry).clone();
        let viewer_count = registry
            .get_viewer_count(&req.room_id)
            .await
            .unwrap_or(0);

        let cache_stats = self.gop_cache.get_stats(&req.room_id);

        if cache_stats.is_some() {
            let stats = StreamStats {
                bytes_sent: 0, // TODO: Track in Publisher
                bytes_received: 0,
                viewer_count: viewer_count as i32,
                bitrate_kbps: 0.0, // TODO: Calculate
                frame_count: cache_stats.as_ref().map(|s| s.total_frames as i32).unwrap_or(0),
                dropped_frames: 0,
                latency_ms: 0.0,
            };

            let response = GetStreamStatsResponse {
                stats: Some(stats),
                exists: true,
            };
            Ok(Response::new(response))
        } else {
            let response = GetStreamStatsResponse {
                stats: None,
                exists: false,
            };
            Ok(Response::new(response))
        }
    }

    /// List active streams
    async fn list_streams(
        &self,
        request: Request<ListStreamsRequest>,
    ) -> Result<Response<ListStreamsResponse>, Status> {
        let req = request.into_inner();
        info!(node_filter = req.node_id, "ListStreams request");

        let mut registry = (*self.registry).clone();
        let stream_ids = registry
            .list_active_streams()
            .await
            .map_err(|e| Status::internal(format!("Failed to list streams: {}", e)))?;

        // Build publisher info for each stream
        let mut streams = Vec::new();
        for room_id in stream_ids {
            if let Ok(Some(pub_info)) = registry.get_publisher(&room_id).await {
                // Filter by node if requested
                if !req.node_id.is_empty() && pub_info.node_id != req.node_id {
                    continue;
                }

                streams.push(PublisherInfo {
                    room_id: room_id.clone(),
                    node_id: pub_info.node_id,
                    stream_key: format!("{}?token=xxx", room_id),
                    metadata: None,
                    started_at: pub_info.started_at.timestamp(),
                    stats: None,
                });
            }
        }

        let total = streams.len() as i32;
        let response = ListStreamsResponse {
            streams,
            total,
        };

        Ok(Response::new(response))
    }
}

// Tests are integration tests that require Redis running
// Run with: cargo test --package synctv-stream -- --ignored
