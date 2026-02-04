//! Prometheus metrics collection for production monitoring
//!
//! This module provides production-grade metrics collection using prometheus crate.
//! All metrics are automatically exposed via the /metrics endpoint for Prometheus scraping.

use prometheus::{CounterVec, HistogramVec, Registry, IntGauge, TextEncoder, Encoder, register_counter_vec_with_registry, register_histogram_vec_with_registry, register_int_gauge_with_registry};

/// Global metrics registry
pub static REGISTRY: std::sync::LazyLock<Registry> = std::sync::LazyLock::new(Registry::new);

/// HTTP request duration histogram
pub static HTTP_REQUEST_DURATION: std::sync::LazyLock<HistogramVec> = std::sync::LazyLock::new(|| {
    register_histogram_vec_with_registry!(
        "http_request_duration_seconds",
        "HTTP request duration in seconds",
        &["endpoint", "method", "status"],
        REGISTRY.clone()
    ).expect("Failed to register HTTP_REQUEST_DURATION")
});

/// HTTP request counter
pub static HTTP_REQUESTS_TOTAL: std::sync::LazyLock<CounterVec> = std::sync::LazyLock::new(|| {
    register_counter_vec_with_registry!(
        "http_requests_total",
        "Total number of HTTP requests",
        &["endpoint", "method", "status"],
        REGISTRY.clone()
    ).expect("Failed to register HTTP_REQUESTS_TOTAL")
});

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
    ($endpoint:expr, $method:expr, $status:expr, $duration:expr) => {
        let status_str = $status.as_str().to_lowercase();
        let endpoint_str = $endpoint.replace('/', "__");
        let method_str = $method.to_lowercase();

        $crate::metrics::HTTP_REQUEST_DURATION
            .with_label_values(&[&endpoint_str, &method_str, &status_str])
            .observe($duration.as_secs_f64());

        $crate::metrics::HTTP_REQUESTS_TOTAL
            .with_label_values(&[&endpoint_str, &method_str, &status_str])
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

/// Expose metrics in Prometheus format
pub fn gather_metrics() -> Result<String, prometheus::Error> {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer)?;
    String::from_utf8(buffer).map_err(|_| prometheus::Error::Msg("Invalid UTF-8".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_registration() {
        // Verify all metrics are registered
        HTTP_REQUEST_DURATION.with_label_values(&["test", "get", "200"]).observe(0.1);
        HTTP_REQUESTS_TOTAL.with_label_values(&["test", "get", "200"]).inc();

        // Should be able to encode metrics
        let encoder = TextEncoder::new();
        let metric_families = REGISTRY.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer).unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert!(output.contains("http_request_duration_seconds"));
    }
}
