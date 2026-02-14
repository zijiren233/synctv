// Custom HLS server with storage abstraction
//
// Architecture:
// 1. HLS HTTP server - serves .m3u8 and .ts files from HlsStorage
// 2. Custom HLS remuxer - uses xiu's libs but pluggable storage backend

use crate::hls::{
    segment_manager::SegmentManager,
    remuxer::{CustomHlsRemuxer, StreamRegistry},
};
use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use crate::streamhub::StreamsHub;

pub struct HlsServer {
    port: usize,
    stream_hub: Arc<Mutex<StreamsHub>>,
    segment_manager: Arc<SegmentManager>,
    stream_registry: StreamRegistry,
    shutdown_token: CancellationToken,
}

impl HlsServer {
    pub fn new(
        address: String,
        stream_hub: Arc<Mutex<StreamsHub>>,
        segment_manager: Arc<SegmentManager>,
        stream_registry: StreamRegistry,
    ) -> Self {
        // Parse port from address (e.g., "0.0.0.0:8081" -> 8081)
        let port = address
            .rsplit_once(':')
            .and_then(|(_, p)| p.parse().ok())
            .unwrap_or(8081);

        Self {
            port,
            stream_hub,
            segment_manager,
            stream_registry,
            shutdown_token: CancellationToken::new(),
        }
    }

    /// Returns a `CancellationToken` that can be used to signal graceful shutdown.
    #[must_use]
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown_token.clone()
    }

    pub async fn start(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("HLS server starting on http://0.0.0.0:{}", self.port);

        let shutdown_token = self.shutdown_token.clone();

        // Start custom HLS HTTP server (serves from our storage)
        let port = self.port;
        let segment_manager_clone = Arc::clone(&self.segment_manager);
        let stream_registry_clone = self.stream_registry.clone();
        let http_shutdown = shutdown_token.clone();
        tokio::spawn(async move {
            if let Err(e) = start_http_server(port, segment_manager_clone, stream_registry_clone, http_shutdown).await {
                tracing::error!("HLS HTTP server error: {}", e);
            }
        });

        // Start custom HLS remuxer (uses xiu's libs + our storage)
        let stream_hub_clone = Arc::clone(&self.stream_hub);
        let segment_manager_clone = Arc::clone(&self.segment_manager);
        let stream_registry_clone = self.stream_registry;
        tokio::spawn(async move {
            let (client_event_consumer, hub_event_sender) = {
                let mut hub = stream_hub_clone.lock().await;
                (hub.get_client_event_consumer(), hub.get_hub_event_sender())
            };

            let mut remuxer = CustomHlsRemuxer::new(
                client_event_consumer,
                hub_event_sender,
                segment_manager_clone,
                stream_registry_clone,
            );

            if let Err(e) = remuxer.run().await {
                tracing::error!("HLS remuxer error: {}", e);
            }
        });

        tracing::info!("HLS server started successfully");

        Ok(())
    }
}

/// HTTP server state
#[derive(Clone)]
struct HlsServerState {
    segment_manager: Arc<SegmentManager>,
    stream_registry: StreamRegistry,
}

/// Start HLS HTTP server with axum
async fn start_http_server(
    port: usize,
    segment_manager: Arc<SegmentManager>,
    stream_registry: StreamRegistry,
    shutdown_token: CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = HlsServerState {
        segment_manager,
        stream_registry,
    };

    let app = Router::new()
        .route("/live/:app/:stream/index.m3u8", axum::routing::get(serve_m3u8))
        .route("/live/:app/:stream/:segment", axum::routing::get(serve_segment))
        .with_state(state);

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("HLS HTTP server listening on {}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown_token.cancelled().await })
        .await?;

    tracing::info!("HLS HTTP server shut down gracefully");

    Ok(())
}

/// Serve M3U8 playlist (dynamically generated)
async fn serve_m3u8(
    Path((app, stream)): Path<(String, String)>,
    State(state): State<HlsServerState>,
) -> Response {
    let registry_key = format!("{app}/{stream}");

    // Look up stream in registry
    if let Some(stream_state) = state.stream_registry.get(&registry_key) {
        // Generate M3U8 dynamically from current segment list
        let state_lock = stream_state.read();

        // Use closure to generate TS URLs with custom format
        let app_clone = app;
        let stream_clone = stream;
        let m3u8_content = state_lock.generate_m3u8(move |ts_name| {
            format!("/live/{app_clone}/{stream_clone}/{ts_name}.ts")
        });

        (
            StatusCode::OK,
            [
                ("Content-Type", "application/vnd.apple.mpegurl"),
                ("Cache-Control", "no-cache"),
            ],
            m3u8_content,
        )
            .into_response()
    } else {
        tracing::warn!("Stream not found: {}", registry_key);
        (StatusCode::NOT_FOUND, "Stream not found or ended").into_response()
    }
}

/// Serve TS segment
async fn serve_segment(
    Path((app, stream, segment_filename)): Path<(String, String, String)>,
    State(state): State<HlsServerState>,
) -> Response {
    // Extract TS name from filename (e.g., "a1b2c3d4e5f6.ts" -> "a1b2c3d4e5f6")
    let ts_name = segment_filename
        .strip_suffix(".ts")
        .unwrap_or(&segment_filename);

    // Build storage key: app-stream-ts_name (no prefix, no ext)
    let storage_key = format!("{app}-{stream}-{ts_name}");

    match state.segment_manager.storage().read(&storage_key).await {
        Ok(data) => {
            (
                StatusCode::OK,
                [
                    ("Content-Type", "video/mp2t"),
                    ("Cache-Control", "public, max-age=90"), // Cache segments for 90 seconds
                ],
                data,
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!("Segment not found: {} - {}", storage_key, e);
            (StatusCode::NOT_FOUND, "Segment not found").into_response()
        }
    }
}
