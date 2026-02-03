//! Provider gRPC Server
//!
//! Standalone gRPC server that exposes provider services (Alist, Bilibili, Emby).
//! Can be deployed as a remote provider instance.

use synctv_providers::grpc::{
    alist::alist_server::AlistServer,
    alist_server::AlistService as AlistGrpcService,
    bilibili::bilibili_server::BilibiliServer,
    bilibili_server::BilibiliService,
    emby::emby_server::EmbyServer,
    emby_server::EmbyService,
};
use tonic::transport::Server;
use tracing::{info, Level};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_target(false)
        .init();

    let addr = "[::1]:50051".parse()?;

    info!("Starting Provider gRPC server on {}", addr);

    // Create service instances
    let alist_service = AlistGrpcService::new();
    let bilibili_service = BilibiliService::new();
    let emby_service = EmbyService::new();

    // Build and start server
    Server::builder()
        .add_service(AlistServer::new(alist_service))
        .add_service(BilibiliServer::new(bilibili_service))
        .add_service(EmbyServer::new(emby_service))
        .serve(addr)
        .await?;

    Ok(())
}
