// gRPC services for stream relay

pub mod proto {
    tonic::include_proto!("synctv.stream");
}

mod stream_relay_service;
mod stream_puller;

pub use stream_relay_service::StreamRelayServiceImpl;
pub use stream_puller::GrpcStreamPuller;
pub use proto::stream_relay_service_server::{StreamRelayService, StreamRelayServiceServer};
pub use proto::stream_relay_service_client::StreamRelayServiceClient;
// Export proto message types
pub use proto::{RtmpPacket, PullRtmpStreamRequest, FrameType};
