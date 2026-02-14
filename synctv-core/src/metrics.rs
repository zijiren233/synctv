//! Prometheus metrics collection for production monitoring
//!
//! This module provides production-grade metrics collection using prometheus crate.
//! All metrics are automatically exposed via the /metrics endpoint for Prometheus scraping.

use prometheus::{CounterVec, HistogramVec, Registry, IntGauge, IntCounterVec, IntGaugeVec, TextEncoder, Encoder, register_counter_vec_with_registry, register_histogram_vec_with_registry, register_int_gauge_with_registry};

/// Global metrics registry
pub static REGISTRY: std::sync::LazyLock<Registry> = std::sync::LazyLock::new(Registry::new);

/// HTTP metrics
pub mod http {
    use super::{IntCounterVec, REGISTRY, HistogramVec, IntGauge, IntGaugeVec};
    use prometheus::{HistogramOpts, Opts, register_int_counter_vec_with_registry, register_int_gauge_with_registry, register_int_gauge_vec_with_registry};

    /// Total HTTP requests, labeled by method, path, and status code.
    pub static HTTP_REQUESTS_TOTAL: std::sync::LazyLock<IntCounterVec> = std::sync::LazyLock::new(|| {
        register_int_counter_vec_with_registry!(
            Opts::new("http_requests_total", "Total number of HTTP requests"),
            &["method", "path", "status"],
            REGISTRY.clone()
        ).expect("Failed to register HTTP_REQUESTS_TOTAL")
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
        .and_then(|m| { REGISTRY.register(Box::new(m.clone()))?; Ok(m) })
        .expect("Failed to register HTTP_REQUEST_DURATION_SECONDS")
    });

    /// Number of in-flight HTTP requests.
    pub static HTTP_REQUESTS_IN_FLIGHT: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
        register_int_gauge_with_registry!(
            "http_requests_in_flight",
            "Number of HTTP requests currently being processed",
            REGISTRY.clone()
        ).expect("Failed to register HTTP_REQUESTS_IN_FLIGHT")
    });

    /// Active WebSocket connections, labeled by `room_id`.
    pub static WEBSOCKET_CONNECTIONS_ACTIVE: std::sync::LazyLock<IntGaugeVec> = std::sync::LazyLock::new(|| {
        register_int_gauge_vec_with_registry!(
            Opts::new("websocket_connections_active", "Number of active WebSocket connections"),
            &["room_id"],
            REGISTRY.clone()
        ).expect("Failed to register WEBSOCKET_CONNECTIONS_ACTIVE")
    });

    /// Total WebSocket connections opened.
    pub static WEBSOCKET_CONNECTIONS_TOTAL: std::sync::LazyLock<IntCounterVec> = std::sync::LazyLock::new(|| {
        register_int_counter_vec_with_registry!(
            Opts::new("websocket_connections_total", "Total number of WebSocket connections opened"),
            &["room_id"],
            REGISTRY.clone()
        ).expect("Failed to register WEBSOCKET_CONNECTIONS_TOTAL")
    });

    /// Number of active rooms.
    pub static ROOMS_ACTIVE: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
        register_int_gauge_with_registry!(
            "rooms_active",
            "Number of currently active rooms",
            REGISTRY.clone()
        ).expect("Failed to register ROOMS_ACTIVE")
    });

    /// Number of online users.
    pub static USERS_ONLINE: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
        register_int_gauge_with_registry!(
            "users_online",
            "Number of currently online users",
            REGISTRY.clone()
        ).expect("Failed to register USERS_ONLINE")
    });

    /// Number of active live streams.
    pub static STREAMS_ACTIVE: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
        register_int_gauge_with_registry!(
            "streams_active",
            "Number of active live streams",
            REGISTRY.clone()
        ).expect("Failed to register STREAMS_ACTIVE")
    });

    /// Number of active WebRTC peer connections.
    pub static WEBRTC_PEERS_ACTIVE: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
        register_int_gauge_with_registry!(
            "webrtc_peers_active",
            "Number of active WebRTC peer connections",
            REGISTRY.clone()
        ).expect("Failed to register WEBRTC_PEERS_ACTIVE")
    });
}

/// Active connections gauge
pub static ACTIVE_CONNECTIONS: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
    register_int_gauge_with_registry!(
        "active_connections",
        "Current number of active connections",
        REGISTRY.clone()
    ).expect("Failed to register ACTIVE_CONNECTIONS")
});

/// Cache operations
pub mod cache {
    use super::{register_counter_vec_with_registry, CounterVec, REGISTRY};

    /// Cache hit counter
    pub static CACHE_HITS: std::sync::LazyLock<CounterVec> = std::sync::LazyLock::new(|| {
        register_counter_vec_with_registry!(
            "cache_hits_total",
            "Total number of cache hits",
            &["cache_type", "level"],
            REGISTRY.clone()
        ).expect("Failed to register CACHE_HITS")
    });

    /// Cache miss counter
    pub static CACHE_MISSES: std::sync::LazyLock<CounterVec> = std::sync::LazyLock::new(|| {
        register_counter_vec_with_registry!(
            "cache_misses_total",
            "Total number of cache misses",
            &["cache_type", "level"],
            REGISTRY.clone()
        ).expect("Failed to register CACHE_MISSES")
    });

    /// Cache evictions counter
    pub static CACHE_EVICTIONS: std::sync::LazyLock<CounterVec> = std::sync::LazyLock::new(|| {
        register_counter_vec_with_registry!(
            "cache_evictions_total",
            "Total number of cache evictions",
            &["cache_type"],
            REGISTRY.clone()
        ).expect("Failed to register CACHE_EVICTIONS")
    });
}

/// Database operations
pub mod database {
    use super::{register_histogram_vec_with_registry, register_int_gauge_with_registry, register_counter_vec_with_registry, HistogramVec, REGISTRY, IntGauge, CounterVec};
    use prometheus::{GaugeVec, register_gauge_vec_with_registry};

    /// Query duration histogram
    pub static DB_QUERY_DURATION: std::sync::LazyLock<HistogramVec> = std::sync::LazyLock::new(|| {
        register_histogram_vec_with_registry!(
            "db_query_duration_seconds",
            "Database query duration in seconds",
            &["operation", "table"],
            REGISTRY.clone()
        ).expect("Failed to register DB_QUERY_DURATION")
    });

    /// Active connections gauge
    pub static DB_CONNECTIONS_ACTIVE: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
        register_int_gauge_with_registry!(
            "db_connections_active",
            "Current number of active database connections",
            REGISTRY.clone()
        ).expect("Failed to register DB_CONNECTIONS_ACTIVE")
    });

    /// Query error counter
    pub static DB_QUERY_ERRORS: std::sync::LazyLock<CounterVec> = std::sync::LazyLock::new(|| {
        register_counter_vec_with_registry!(
            "db_query_errors_total",
            "Total number of database query errors",
            &["operation", "error_type"],
            REGISTRY.clone()
        ).expect("Failed to register DB_QUERY_ERRORS")
    });

    /// Pool utilization percentage (0.0 to 1.0)
    pub static DB_POOL_UTILIZATION: std::sync::LazyLock<GaugeVec> = std::sync::LazyLock::new(|| {
        register_gauge_vec_with_registry!(
            "db_pool_utilization_ratio",
            "Database connection pool utilization ratio (active/max)",
            &["pool"],
            REGISTRY.clone()
        ).expect("Failed to register DB_POOL_UTILIZATION")
    });

    /// Connections waiting for a connection from the pool
    pub static DB_CONNECTIONS_WAITING: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
        register_int_gauge_with_registry!(
            "db_connections_waiting",
            "Number of connections waiting for a connection from the pool",
            REGISTRY.clone()
        ).expect("Failed to register DB_CONNECTIONS_WAITING")
    });

    /// Connection acquire duration histogram
    pub static DB_CONNECTION_ACQUIRE_DURATION: std::sync::LazyLock<HistogramVec> = std::sync::LazyLock::new(|| {
        register_histogram_vec_with_registry!(
            "db_connection_acquire_duration_seconds",
            "Time taken to acquire a connection from the pool",
            &["pool"],
            REGISTRY.clone()
        ).expect("Failed to register DB_CONNECTION_ACQUIRE_DURATION")
    });

    /// Transaction rollback counter
    pub static DB_TRANSACTION_ROLLBACKS: std::sync::LazyLock<CounterVec> = std::sync::LazyLock::new(|| {
        register_counter_vec_with_registry!(
            "db_transaction_rollbacks_total",
            "Total number of database transaction rollbacks",
            &["reason"],
            REGISTRY.clone()
        ).expect("Failed to register DB_TRANSACTION_ROLLBACKS")
    });

    /// Total connections in the pool (max pool size)
    pub static DB_POOL_SIZE_MAX: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
        register_int_gauge_with_registry!(
            "db_pool_size_max",
            "Maximum number of connections in the pool",
            REGISTRY.clone()
        ).expect("Failed to register DB_POOL_SIZE_MAX")
    });

    /// Idle connections in the pool
    pub static DB_CONNECTIONS_IDLE: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
        register_int_gauge_with_registry!(
            "db_connections_idle",
            "Number of idle connections in the pool",
            REGISTRY.clone()
        ).expect("Failed to register DB_CONNECTIONS_IDLE")
    });
}

/// gRPC operations
pub mod grpc {
    use super::{register_histogram_vec_with_registry, register_int_gauge_with_registry, HistogramVec, REGISTRY, IntGauge};

    /// RPC request duration histogram
    pub static GRPC_REQUEST_DURATION: std::sync::LazyLock<HistogramVec> = std::sync::LazyLock::new(|| {
        register_histogram_vec_with_registry!(
            "grpc_request_duration_seconds",
            "gRPC request duration in seconds",
            &["service", "method", "status"],
            REGISTRY.clone()
        ).expect("Failed to register GRPC_REQUEST_DURATION")
    });

    /// Active RPC streams gauge
    pub static GRPC_ACTIVE_STREAMS: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
        register_int_gauge_with_registry!(
            "grpc_active_streams",
            "Current number of active gRPC streams",
            REGISTRY.clone()
        ).expect("Failed to register GRPC_ACTIVE_STREAMS")
    });
}

/// Stream operations
pub mod stream {
    use super::{register_histogram_vec_with_registry, register_int_gauge_with_registry, register_counter_vec_with_registry, HistogramVec, REGISTRY, IntGauge, CounterVec};

    /// Stream relay duration histogram
    pub static STREAM_RELAY_DURATION: std::sync::LazyLock<HistogramVec> = std::sync::LazyLock::new(|| {
        register_histogram_vec_with_registry!(
            "stream_relay_duration_seconds",
            "Stream relay operation duration in seconds",
            &["stream_id"],
            REGISTRY.clone()
        ).expect("Failed to register STREAM_RELAY_DURATION")
    });

    /// Active relay streams gauge
    pub static ACTIVE_RELAY_STREAMS: std::sync::LazyLock<IntGauge> = std::sync::LazyLock::new(|| {
        register_int_gauge_with_registry!(
            "active_relay_streams",
            "Current number of active relay streams",
            REGISTRY.clone()
        ).expect("Failed to register ACTIVE_RELAY_STREAMS")
    });

    /// Stream error counter
    pub static STREAM_ERRORS: std::sync::LazyLock<CounterVec> = std::sync::LazyLock::new(|| {
        register_counter_vec_with_registry!(
            "stream_errors_total",
            "Total number of stream errors",
            &["error_type", "stream_id"],
            REGISTRY.clone()
        ).expect("Failed to register STREAM_ERRORS")
    });
}

/// Helper macro to record HTTP request metrics
#[macro_export]
macro_rules! record_http_request {
    ($method:expr, $path:expr, $status:expr, $duration:expr) => {
        let status_str = $status.to_string();
        let method_str = $method.to_string();

        $crate::metrics::http::HTTP_REQUEST_DURATION_SECONDS
            .with_label_values(&[&method_str, $path])
            .observe($duration.as_secs_f64());

        $crate::metrics::http::HTTP_REQUESTS_TOTAL
            .with_label_values(&[&method_str, $path, &status_str])
            .inc();
    };
}

/// Helper macro to record cache metrics
#[macro_export]
macro_rules! record_cache_hit {
    ($cache_type:expr, $level:expr) => {
        $crate::metrics::cache::CACHE_HITS
            .with_label_values(&[$cache_type, $level])
            .inc();
    };
}

#[macro_export]
macro_rules! record_cache_miss {
    ($cache_type:expr, $level:expr) => {
        $crate::metrics::cache::CACHE_MISSES
            .with_label_values(&[$cache_type, $level])
            .inc();
    };
}

/// Helper macro to record database query metrics
#[macro_export]
macro_rules! record_db_query {
    ($operation:expr, $table:expr, $duration:expr, $error:expr) => {
        $crate::metrics::database::DB_QUERY_DURATION
            .with_label_values(&[$operation, $table])
            .observe($duration.as_secs_f64());

        if let Err(e) = $error {
            let error_type = if e.to_string().contains("timeout") {
                "timeout"
            } else if e.to_string().contains("connection") {
                "connection"
            } else {
                "other"
            };
            $crate::metrics::database::DB_QUERY_ERRORS
                .with_label_values(&[$operation, error_type])
                .inc();
        }
    };
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

        // Replace segments that look like IDs (after known resource paths)
        let prev = if i > 0 { segments.get(i - 1) } else { None };
        let is_id = matches!(prev, Some(&"rooms" | &"media" | &"chat" | &"playlists"));

        if is_id {
            result.push(":id");
        } else {
            result.push(segment);
        }
    }

    result.join("/")
}

/// Expose metrics in Prometheus format
pub fn gather_metrics() -> String {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();
    match encoder.encode(&metric_families, &mut buffer) {
        Ok(()) => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_registration() {
        // Verify all metrics are registered
        http::HTTP_REQUEST_DURATION_SECONDS.with_label_values(&["GET", "/test"]).observe(0.1);
        http::HTTP_REQUESTS_TOTAL.with_label_values(&["GET", "/test", "200"]).inc();

        // Should be able to encode metrics
        let encoder = TextEncoder::new();
        let metric_families = REGISTRY.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer).unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert!(output.contains("http_request_duration_seconds"));
    }
}
