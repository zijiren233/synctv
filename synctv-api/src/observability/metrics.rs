//! Prometheus metrics for SyncTV
//!
//! Provides HTTP request metrics, WebSocket connection tracking,
//! room/user counts, and other operational metrics.

use once_cell::sync::Lazy;
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
    TextEncoder,
};

/// Global metrics registry
static REGISTRY: Lazy<Registry> = Lazy::new(|| {
    let registry = Registry::new();
    register_metrics(&registry);
    registry
});

// --- HTTP Metrics ---

/// Total HTTP requests, labeled by method, path, and status code.
pub static HTTP_REQUESTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new("http_requests_total", "Total number of HTTP requests"),
        &["method", "path", "status"],
    )
    .expect("failed to create http_requests_total")
});

/// HTTP request duration in seconds, labeled by method and path.
pub static HTTP_REQUEST_DURATION_SECONDS: Lazy<HistogramVec> = Lazy::new(|| {
    HistogramVec::new(
        HistogramOpts::new(
            "http_request_duration_seconds",
            "HTTP request duration in seconds",
        )
        .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
        &["method", "path"],
    )
    .expect("failed to create http_request_duration_seconds")
});

/// Number of in-flight HTTP requests.
pub static HTTP_REQUESTS_IN_FLIGHT: Lazy<IntGauge> = Lazy::new(|| {
    IntGauge::new(
        "http_requests_in_flight",
        "Number of HTTP requests currently being processed",
    )
    .expect("failed to create http_requests_in_flight")
});

// --- WebSocket Metrics ---

/// Active WebSocket connections, labeled by room_id.
pub static WEBSOCKET_CONNECTIONS_ACTIVE: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "websocket_connections_active",
            "Number of active WebSocket connections",
        ),
        &["room_id"],
    )
    .expect("failed to create websocket_connections_active")
});

/// Total WebSocket connections opened.
pub static WEBSOCKET_CONNECTIONS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "websocket_connections_total",
            "Total number of WebSocket connections opened",
        ),
        &["room_id"],
    )
    .expect("failed to create websocket_connections_total")
});

// --- Room Metrics ---

/// Number of active rooms.
pub static ROOMS_ACTIVE: Lazy<IntGauge> = Lazy::new(|| {
    IntGauge::new("rooms_active", "Number of currently active rooms")
        .expect("failed to create rooms_active")
});

// --- User Metrics ---

/// Number of online users.
pub static USERS_ONLINE: Lazy<IntGauge> = Lazy::new(|| {
    IntGauge::new("users_online", "Number of currently online users")
        .expect("failed to create users_online")
});

// --- Streaming Metrics ---

/// Number of active live streams.
pub static STREAMS_ACTIVE: Lazy<IntGauge> = Lazy::new(|| {
    IntGauge::new("streams_active", "Number of active live streams")
        .expect("failed to create streams_active")
});

// --- WebRTC Metrics ---

/// Number of active WebRTC peer connections.
pub static WEBRTC_PEERS_ACTIVE: Lazy<IntGauge> = Lazy::new(|| {
    IntGauge::new(
        "webrtc_peers_active",
        "Number of active WebRTC peer connections",
    )
    .expect("failed to create webrtc_peers_active")
});

/// Register all metrics with the registry.
fn register_metrics(registry: &Registry) {
    registry
        .register(Box::new(HTTP_REQUESTS_TOTAL.clone()))
        .expect("failed to register http_requests_total");
    registry
        .register(Box::new(HTTP_REQUEST_DURATION_SECONDS.clone()))
        .expect("failed to register http_request_duration_seconds");
    registry
        .register(Box::new(HTTP_REQUESTS_IN_FLIGHT.clone()))
        .expect("failed to register http_requests_in_flight");
    registry
        .register(Box::new(WEBSOCKET_CONNECTIONS_ACTIVE.clone()))
        .expect("failed to register websocket_connections_active");
    registry
        .register(Box::new(WEBSOCKET_CONNECTIONS_TOTAL.clone()))
        .expect("failed to register websocket_connections_total");
    registry
        .register(Box::new(ROOMS_ACTIVE.clone()))
        .expect("failed to register rooms_active");
    registry
        .register(Box::new(USERS_ONLINE.clone()))
        .expect("failed to register users_online");
    registry
        .register(Box::new(STREAMS_ACTIVE.clone()))
        .expect("failed to register streams_active");
    registry
        .register(Box::new(WEBRTC_PEERS_ACTIVE.clone()))
        .expect("failed to register webrtc_peers_active");
}

/// Gather all metrics and encode them in Prometheus text format.
pub fn gather_metrics() -> String {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).expect("failed to encode metrics");
    String::from_utf8(buffer).expect("metrics are valid UTF-8")
}

/// Normalize a request path for metric labels.
///
/// Replaces path parameters (UUIDs, numeric IDs, nanoids) with placeholders
/// to avoid high-cardinality labels.
pub fn normalize_path(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').collect();
    let mut result = Vec::with_capacity(segments.len());

    for (i, segment) in segments.iter().enumerate() {
        if segment.is_empty() {
            result.push(*segment);
            continue;
        }

        // Replace segments that look like IDs (UUIDs, numeric, nanoid-style alphanumeric)
        let prev = if i > 0 { segments.get(i - 1) } else { None };
        let is_id = match prev {
            Some(&"rooms") | Some(&"media") | Some(&"chat") | Some(&"playlists") => true,
            _ => false,
        };

        if is_id {
            result.push(":id");
        } else {
            result.push(segment);
        }
    }

    result.join("/")
}
