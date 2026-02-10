// Complete RTMP server implementation using xiu
//
// Architecture:
// 1. Use xiu's StreamHub as the core event bus (includes GOP cache)
// 2. PublisherManager listens to StreamHub events and registers to Redis
// 3. StreamHub automatically distributes frames + GOP to all local subscribers

use crate::{
    relay::{registry_trait::StreamRegistryTrait, publisher_manager::PublisherManager},
    error::{StreamResult, StreamError},
};
use rtmp::auth::AuthCallback;
use std::sync::Arc;
use tracing as log;
use streamhub::StreamsHub;
use streamhub::define::StreamHubEventSender;
use tokio::sync::{mpsc, Mutex};

pub struct RtmpServer {
    address: String,
    registry: Arc<dyn StreamRegistryTrait>,
    node_id: String,
    gop_num: usize,
    auth: Option<Arc<dyn AuthCallback>>,
    pub(in crate::livestream) event_sender: StreamHubEventSender,
    pub(in crate::livestream) stream_hub: Arc<Mutex<StreamsHub>>,
}

impl RtmpServer {
    pub fn new(
        address: String,
        registry: Arc<dyn StreamRegistryTrait>,
        node_id: String,
        gop_num: usize,
        auth: Option<Arc<dyn AuthCallback>>,
    ) -> Self {
        // Create StreamHub channels
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        let stream_hub = StreamsHub::new(event_sender.clone(), event_receiver);

        Self {
            address,
            registry,
            node_id,
            gop_num,
            auth,
            event_sender: event_sender.clone(),
            stream_hub: Arc::new(Mutex::new(stream_hub)),
        }
    }

    pub async fn start(&mut self) -> StreamResult<()> {
        let socket_addr: std::net::SocketAddr = self.address
            .parse()
            .map_err(|e| StreamError::InvalidAddress(format!("Invalid RTMP address: {}", e)))?;

        log::info!("RTMP server listening on rtmp://{}", socket_addr);

        // Get client event consumer from StreamHub
        let client_event_consumer = {
            let mut hub = self.stream_hub.lock().await;
            hub.get_client_event_consumer()
        };

        // Start PublisherManager to listen for Publish/UnPublish events
        let registry = Arc::clone(&self.registry);
        let node_id = self.node_id.clone();
        let publisher_manager = Arc::new(PublisherManager::new(
            registry.clone(),
            node_id,
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

        // Start xiu RtmpServer (includes built-in GOP cache with gop_num)
        let event_sender = self.event_sender.clone();
        let gop_num = self.gop_num;
        let auth = self.auth.clone();
        let mut xiu_rtmp_server = rtmp::rtmp::RtmpServer::new(
            self.address.clone(),
            event_sender,
            gop_num,
            auth,
        );

        tokio::spawn(async move {
            if let Err(e) = xiu_rtmp_server.run().await {
                log::error!("xiu RTMP server error: {}", e);
            }
        });

        log::info!(
            "RTMP server started successfully (gop_num={})",
            gop_num
        );

        // Keep running
        tokio::time::sleep(std::time::Duration::from_secs(u64::MAX)).await;

        Ok(())
    }
}
