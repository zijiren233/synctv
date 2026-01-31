// HTTP-FLV server wrapper for xiu
//
// xiu's HTTP-FLV functionality uses httpflv::server::run() to serve FLV streams
// over HTTP. The server subscribes to streams from the StreamHub.
//
// Based on:
// - /tmp/xiu/protocol/httpflv/src/server.rs
// - Design doc: 02-整体架构.md § 2.2.2 "FLV拉流流程 (懒加载)"

use crate::{
    relay::registry::StreamRegistry,
    streaming::pull_manager::PullStreamManager,
    error::{StreamResult, StreamError},
};
use tracing as log;
use std::sync::Arc;
use tokio::sync::Mutex;
use streamhub::StreamsHub;

pub struct HttpFlvServer {
    port: usize,
    registry: StreamRegistry,
    pull_manager: Arc<PullStreamManager>,
    stream_hub: Arc<Mutex<StreamsHub>>,
}

impl HttpFlvServer {
    pub fn new(
        address: String,
        pull_manager: Arc<PullStreamManager>,
        registry: StreamRegistry,
        stream_hub: Arc<Mutex<StreamsHub>>,
    ) -> Self {
        // Parse port from address (e.g., "0.0.0.0:8080" -> 8080)
        let port = address
            .split(':')
            .nth(1)
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080);

        Self {
            port,
            registry,
            pull_manager,
            stream_hub,
        }
    }

    pub async fn start(self) -> StreamResult<()> {
        log::info!("HTTP-FLV server starting on http://0.0.0.0:{}", self.port);

        // Get event sender from StreamHub
        let hub_event_sender = {
            let mut hub = self.stream_hub.lock().await;
            hub.get_hub_event_sender()
        };

        // Start xiu HTTP-FLV server
        let port = self.port;
        tokio::spawn(async move {
            if let Err(e) = httpflv::server::run(hub_event_sender, port, None).await {
                log::error!("HTTP-FLV server error: {}", e);
            }
        });

        // TODO: Implement lazy-load pattern
        // Currently xiu creates pull streams immediately when publisher connects
        // We need to modify to create pull streams only when FLV viewers connect

        log::info!("HTTP-FLV server started successfully");

        Ok(())
    }
}
