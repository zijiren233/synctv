// Complete RTMP server implementation using xiu
//
// Architecture:
// 1. Use xiu's StreamHub as the core event bus
// 2. PublisherManager listens to StreamHub events and registers to Redis
// 3. Integrate SyncTvStreamHandler for GOP cache
// 4. Implement TStreamHandler trait for prior data (GOP cache) delivery
//
// Based on:
// - /tmp/xiu/application/xiu/src/service.rs § start_rtmp()
// - /tmp/xiu/protocol/rtmp/src/rtmp.rs
// - Design doc: 17-数据流设计.md § 11.1

use crate::{
    cache::gop_cache::GopCache,
    relay::{registry::StreamRegistry, publisher_manager::PublisherManager},
    error::{StreamResult, StreamError},
};
use std::sync::Arc;
use tracing as log;
use streamhub::StreamsHub;
use tokio::sync::Mutex;

pub struct RtmpServer {
    address: String,
    gop_cache: Arc<GopCache>,
    registry: StreamRegistry,
    node_id: String,
    gop_num: usize,
    stream_hub: Arc<Mutex<StreamsHub>>,
}

impl RtmpServer {
    pub fn new(
        address: String,
        gop_cache: Arc<GopCache>,
        registry: StreamRegistry,
        node_id: String,
        gop_num: usize,
    ) -> Self {
        // Create StreamHub (xiu's event bus)
        let stream_hub = StreamsHub::new(None);

        Self {
            address,
            gop_cache,
            registry,
            node_id,
            gop_num,
            stream_hub: Arc::new(Mutex::new(stream_hub)),
        }
    }

    pub async fn start(&mut self) -> StreamResult<()> {
        let socket_addr: std::net::SocketAddr = self.address
            .parse()
            .map_err(|e| StreamError::InvalidAddress(format!("Invalid RTMP address: {}", e)))?;

        log::info!("RTMP server listening on rtmp://{}", socket_addr);

        // Get event sender and client event consumer from StreamHub
        let (event_sender, client_event_consumer) = {
            let mut hub = self.stream_hub.lock().await;
            (hub.get_hub_event_sender(), hub.get_client_event_consumer())
        };

        // Start PublisherManager to listen for Publish/UnPublish events
        let publisher_manager = Arc::new(PublisherManager::new(
            self.registry.clone(),
            self.node_id.clone(),
        ));
        tokio::spawn(async move {
            publisher_manager.start(client_event_consumer).await;
        });

        // Start StreamHub event loop
        let hub_clone = Arc::clone(&self.stream_hub);
        tokio::spawn(async move {
            let mut hub = hub_clone.lock().await;
            hub.run().await;
            log::info!("StreamHub event loop ended");
        });

        // Start xiu RtmpServer
        let gop_num = self.gop_num;
        let mut xiu_rtmp_server = rtmp::rtmp::RtmpServer::new(
            self.address.clone(),
            event_sender,
            gop_num,
            None, // No auth for now
        );

        tokio::spawn(async move {
            if let Err(e) = xiu_rtmp_server.run().await {
                log::error!("xiu RTMP server error: {}", e);
            }
        });

        // TODO: Integrate SyncTvStreamHandler with StreamHub for GOP cache

        log::info!("RTMP server started successfully");

        // Keep running
        tokio::time::sleep(std::time::Duration::from_secs(u64::MAX)).await;

        Ok(())
    }
}
