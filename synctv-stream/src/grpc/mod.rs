// gRPC services for stream relay

pub mod proto {
    tonic::include_proto!("synctv.stream");
}

mod stream_relay_service;

pub use stream_relay_service::StreamRelayServiceImpl;
pub use proto::stream_relay_service_server::{StreamRelayService, StreamRelayServiceServer};
pub use proto::stream_relay_service_client::StreamRelayServiceClient;
