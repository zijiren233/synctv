//! Prometheus metrics for `SyncTV`
//!
//! This module re-exports metrics from synctv-core's unified registry.
//! All metrics are registered in a single global registry to ensure
//! the /metrics endpoint exposes everything.

// Re-export HTTP metrics from core
pub use synctv_core::metrics::http::{
    HTTP_REQUESTS_TOTAL,
    HTTP_REQUEST_DURATION_SECONDS,
    HTTP_REQUESTS_IN_FLIGHT,
    WEBSOCKET_CONNECTIONS_ACTIVE,
    WEBSOCKET_CONNECTIONS_TOTAL,
    ROOMS_ACTIVE,
    USERS_ONLINE,
    STREAMS_ACTIVE,
    WEBRTC_PEERS_ACTIVE,
};

// Re-export gather and normalize from core
pub use synctv_core::metrics::{gather_metrics, normalize_path};
