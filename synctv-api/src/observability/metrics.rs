//! Prometheus metrics for `SyncTV`
//!
//! Provides HTTP request metrics, WebSocket connection tracking,
//! room/user counts, and other operational metrics.

use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
    TextEncoder,
};

/// Global metrics registry
static REGISTRY: std::sync::LazyLock<Registry> = std::sync::LazyLock::new(|| {
    let registry = Registry::new();
    if let Err(e) = register_metrics_safe(&registry) {
        tracing::error!("Failed to register metrics: {}. Metrics will be unavailable.", e);
    }
    registry
});

// --- HTTP Metrics ---

/// Total HTTP requests, labeled by method, path, and status code.
pub static HTTP_REQUESTS_TOTAL: std::sync::LazyLock<IntCounterVec> = std::sync::LazyLock::new(|| {
    IntCounterVec::new(
        Opts::new("http_requests_total", "Total number of HTTP requests"),
        &["method", "path", "status"],
    )
    .unwrap_or_else(|e| {
        tracing::error!("Failed to create http_requests_total metric: {}", e);
        // Create a dummy metric that won't panic
        IntCounterVec::new(
            Opts::new("http_requests_total_fallback", "Fallback metric"),
            &["method", "path", "status"],
        )
        .expect("fallback metric creation failed")
    })
});

/// HTTP request duration in seconds, labeled by method and path.
pub static HTTP_REQUEST_DURATION_SECONDS: std::sync::LazyLock<HistogramVec> = std::sync::LazyLock::new(|| {
    HistogramVec::new(
        HistogramOpts::new(
            "http_request_duration_seconds",
            "HTTP request duration in seconds",
        )
        .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
        &["method", "path"],
    )
    .unwrap_or_else(|e| {
        tracing::error!("Failed to create http_request_duration_seconds metric: {}", e);
        HistogramVec::new(
            HistogramOpts::new("http_request_duration_seconds_fallback", "Fallback metric"),
            &["method", "path"],
        )
        .expect("fallback metric creation failed")
    })
});

/// Number of in-flight HTTP requests.
pub static HTTP_REQUESTS_IN_FLIGHT: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
    IntGauge::new(
        "http_requests_in_flight",
        "Number of HTTP requests currently being processed",
    )
    .unwrap_or_else(|e| {
        tracing::error!("Failed to create http_requests_in_flight metric: {}", e);
        IntGauge::new("http_requests_in_flight_fallback", "Fallback metric")
            .expect("fallback metric creation failed")
    })
});

// --- WebSocket Metrics ---

/// Active WebSocket connections, labeled by `room_id`.
pub static WEBSOCKET_CONNECTIONS_ACTIVE: std::sync::LazyLock<IntGaugeVec> = std::sync::LazyLock::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "websocket_connections_active",
            "Number of active WebSocket connections",
        ),
        &["room_id"],
    )
    .unwrap_or_else(|e| {
        tracing::error!("Failed to create websocket_connections_active metric: {}", e);
        IntGaugeVec::new(
            Opts::new("websocket_connections_active_fallback", "Fallback metric"),
            &["room_id"],
        )
        .expect("fallback metric creation failed")
    })
});

/// Total WebSocket connections opened.
pub static WEBSOCKET_CONNECTIONS_TOTAL: std::sync::LazyLock<IntCounterVec> = std::sync::LazyLock::new(|| {
    IntCounterVec::new(
        Opts::new(
            "websocket_connections_total",
            "Total number of WebSocket connections opened",
        ),
        &["room_id"],
    )
    .unwrap_or_else(|e| {
        tracing::error!("Failed to create websocket_connections_total metric: {}", e);
        IntCounterVec::new(
            Opts::new("websocket_connections_total_fallback", "Fallback metric"),
            &["room_id"],
        )
        .expect("fallback metric creation failed")
    })
});

// --- Room Metrics ---

/// Number of active rooms.
pub static ROOMS_ACTIVE: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
    IntGauge::new("rooms_active", "Number of currently active rooms")
        .unwrap_or_else(|e| {
            tracing::error!("Failed to create rooms_active metric: {}", e);
            IntGauge::new("rooms_active_fallback", "Fallback metric")
                .expect("fallback metric creation failed")
        })
});

// --- User Metrics ---

/// Number of online users.
pub static USERS_ONLINE: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
    IntGauge::new("users_online", "Number of currently online users")
        .unwrap_or_else(|e| {
            tracing::error!("Failed to create users_online metric: {}", e);
            IntGauge::new("users_online_fallback", "Fallback metric")
                .expect("fallback metric creation failed")
        })
});

// --- Streaming Metrics ---

/// Number of active live streams.
pub static STREAMS_ACTIVE: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
    IntGauge::new("streams_active", "Number of active live streams")
        .unwrap_or_else(|e| {
            tracing::error!("Failed to create streams_active metric: {}", e);
            IntGauge::new("streams_active_fallback", "Fallback metric")
                .expect("fallback metric creation failed")
        })
});

// --- WebRTC Metrics ---

/// Number of active WebRTC peer connections.
pub static WEBRTC_PEERS_ACTIVE: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
    IntGauge::new(
        "webrtc_peers_active",
        "Number of active WebRTC peer connections",
    )
    .unwrap_or_else(|e| {
        tracing::error!("Failed to create webrtc_peers_active metric: {}", e);
        IntGauge::new("webrtc_peers_active_fallback", "Fallback metric")
            .expect("fallback metric creation failed")
    })
});

/// Register all metrics with the registry.
/// Register all metrics with the registry, with error handling
fn register_metrics_safe(registry: &Registry) -> Result<(), Box<dyn std::error::Error>> {
    registry.register(Box::new(HTTP_REQUESTS_TOTAL.clone()))?;
    registry.register(Box::new(HTTP_REQUEST_DURATION_SECONDS.clone()))?;
    registry.register(Box::new(HTTP_REQUESTS_IN_FLIGHT.clone()))?;
    registry.register(Box::new(WEBSOCKET_CONNECTIONS_ACTIVE.clone()))?;
    registry.register(Box::new(WEBSOCKET_CONNECTIONS_TOTAL.clone()))?;
    registry.register(Box::new(ROOMS_ACTIVE.clone()))?;
    registry.register(Box::new(USERS_ONLINE.clone()))?;
    registry.register(Box::new(STREAMS_ACTIVE.clone()))?;
    registry.register(Box::new(WEBRTC_PEERS_ACTIVE.clone()))?;
    Ok(())
}

/// Gather all metrics and encode them in Prometheus text format.
pub fn gather_metrics() -> String {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();
    match encoder.encode(&metric_families, &mut buffer) {
        Ok(()) => {},
        Err(e) => {
            tracing::error!("Failed to encode metrics: {}", e);
            return String::from("# Failed to encode metrics\n");
        }
    }
    String::from_utf8(buffer).unwrap_or_else(|e| {
        tracing::error!("Metrics buffer contains invalid UTF-8: {}", e);
        String::from("# Invalid UTF-8 in metrics\n")
    })
}

/// Normalize a request path for metric labels.
///
/// Replaces path parameters (UUIDs, numeric IDs, nanoids) with placeholders
/// to avoid high-cardinality labels.
#[must_use] 
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
            Some(&"rooms" | &"media" | &"chat" | &"playlists") => true,
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
