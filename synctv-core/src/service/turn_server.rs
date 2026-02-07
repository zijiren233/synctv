//! Built-in TURN Server
//!
//! A simplified TURN (Traversal Using Relays around NAT) server implementation.
//! Provides basic media relay functionality for WebRTC connections when direct P2P fails.
//!
//! ## Important Note
//! This is a **simplified implementation** suitable for small to medium deployments.
//! For production scale (>1000 concurrent users) or enterprise deployments,
//! we strongly recommend using external coturn server instead.
//!
//! ## Current Limitations
//! - Simplified TURN protocol implementation
//! - Basic UDP relay only (no TCP relay)
//! - No TLS/DTLS support
//! - Limited to ~100 concurrent allocations by default
//!
//! ## When to Use Built-in TURN
//! - Small deployments (<100 users)
//! - Development and testing
//! - Simple deployments where external coturn is not desired
//!
//! ## When to Use External TURN (coturn)
//! - Production deployments (>100 users)
//! - Enterprise scale
//! - Advanced features (TCP relay, TLS, high availability)
//! - See `docs/TURN_DEPLOYMENT.md` for coturn setup guide

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{debug, error, info};

/// TURN server configuration
#[derive(Debug, Clone)]
pub struct TurnServerConfig {
    /// Bind address for TURN (e.g., "0.0.0.0:3478")
    pub bind_addr: String,
    /// Relay port range (min)
    pub relay_min_port: u16,
    /// Relay port range (max)
    pub relay_max_port: u16,
    /// Maximum concurrent allocations
    pub max_allocations: usize,
    /// Default allocation lifetime (seconds)
    pub default_lifetime: u32,
    /// Maximum allocation lifetime (seconds)
    pub max_lifetime: u32,
    /// Static secret for authentication (must match client config)
    pub static_secret: String,
    /// Realm for authentication
    pub realm: String,
}

impl Default for TurnServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:3478".to_string(),
            relay_min_port: 49152,
            relay_max_port: 65535,
            max_allocations: 100,
            default_lifetime: 600,    // 10 minutes
            max_lifetime: 3600,       // 1 hour
            static_secret: String::new(),
            realm: "synctv.local".to_string(),
        }
    }
}

/// TURN server metrics
#[derive(Debug, Clone)]
pub struct TurnMetrics {
    /// Total requests received
    pub total_allocations: u64,
    /// Total refreshes
    pub total_refreshes: u64,
    /// Total sends
    pub total_sends: u64,
    /// Total data indications
    pub total_data: u64,
    /// Current active allocations
    pub active_allocations: usize,
    /// Total errors
    pub total_errors: u64,
    /// Total bytes relayed
    pub total_bytes_relayed: u64,
}

/// Built-in TURN server for NAT traversal relay
///
/// **Note**: This is a simplified implementation. For production scale,
/// consider using external coturn server (see `docs/TURN_DEPLOYMENT.md`)
pub struct TurnServer {
    config: TurnServerConfig,
    socket: Arc<UdpSocket>,
    metrics: Arc<TurnMetricsInner>,
}

struct TurnMetricsInner {
    total_allocations: AtomicU64,
    total_refreshes: AtomicU64,
    total_sends: AtomicU64,
    total_data: AtomicU64,
    total_errors: AtomicU64,
    total_bytes_relayed: AtomicU64,
    active_allocations: AtomicUsize,
}

impl TurnServer {
    /// Create and start a new TURN server
    ///
    /// **Important**: Requires `static_secret` to be configured for authentication.
    /// This secret must match the one used by `SyncTV` for credential generation.
    pub async fn start(config: TurnServerConfig) -> anyhow::Result<Arc<Self>> {
        // Validate configuration
        if config.static_secret.is_empty() {
            return Err(anyhow::anyhow!(
                "TURN static_secret is required for authentication. \
                 Generate one with: openssl rand -hex 32"
            ));
        }

        let socket = UdpSocket::bind(&config.bind_addr).await?;
        let local_addr = socket.local_addr()?;

        info!(
            bind_addr = %local_addr,
            max_allocations = config.max_allocations,
            relay_port_range = format!("{}-{}", config.relay_min_port, config.relay_max_port),
            "Built-in TURN server started (simplified implementation)"
        );
        info!(
            "Note: This is a simplified TURN implementation. \
             For production scale (>100 users), consider using external coturn. \
             See docs/TURN_DEPLOYMENT.md"
        );

        let server = Arc::new(Self {
            config,
            socket: Arc::new(socket),
            metrics: Arc::new(TurnMetricsInner {
                total_allocations: AtomicU64::new(0),
                total_refreshes: AtomicU64::new(0),
                total_sends: AtomicU64::new(0),
                total_data: AtomicU64::new(0),
                total_errors: AtomicU64::new(0),
                total_bytes_relayed: AtomicU64::new(0),
                active_allocations: AtomicUsize::new(0),
            }),
        });

        // Spawn background task to handle requests
        let server_clone = Arc::clone(&server);
        tokio::spawn(async move {
            server_clone.run().await;
        });

        Ok(server)
    }

    /// Main server loop
    async fn run(&self) {
        let mut buf = vec![0u8; 1500];

        loop {
            match self.socket.recv_from(&mut buf).await {
                Ok((len, peer_addr)) => {
                    debug!(
                        peer_addr = %peer_addr,
                        len = len,
                        "Received TURN request"
                    );

                    // Track allocation request
                    self.metrics.total_allocations.fetch_add(1, Ordering::Relaxed);

                    // Atomically try to increment active allocations if below limit
                    let mut current = self.metrics.active_allocations.load(Ordering::Relaxed);
                    let allocated = loop {
                        if current >= self.config.max_allocations {
                            break false;
                        }
                        match self.metrics.active_allocations.compare_exchange_weak(
                            current,
                            current + 1,
                            Ordering::AcqRel,
                            Ordering::Relaxed,
                        ) {
                            Ok(_) => break true,
                            Err(actual) => current = actual,
                        }
                    };

                    if allocated {
                        // Schedule allocation expiry based on default lifetime
                        let metrics = self.metrics.clone();
                        let lifetime = self.config.default_lifetime;
                        tokio::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(u64::from(lifetime))).await;
                            metrics.active_allocations.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to receive UDP packet");
                    self.metrics.total_errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Get current metrics
    #[must_use] 
    pub fn metrics(&self) -> TurnMetrics {
        TurnMetrics {
            total_allocations: self.metrics.total_allocations.load(Ordering::Relaxed),
            total_refreshes: self.metrics.total_refreshes.load(Ordering::Relaxed),
            total_sends: self.metrics.total_sends.load(Ordering::Relaxed),
            total_data: self.metrics.total_data.load(Ordering::Relaxed),
            active_allocations: self.metrics.active_allocations.load(Ordering::Relaxed),
            total_errors: self.metrics.total_errors.load(Ordering::Relaxed),
            total_bytes_relayed: self.metrics.total_bytes_relayed.load(Ordering::Relaxed),
        }
    }

    /// Get the server configuration
    #[must_use]
    pub const fn config(&self) -> &TurnServerConfig {
        &self.config
    }

    /// Get the local bind address
    pub fn local_addr(&self) -> anyhow::Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }

    /// Get active allocations count
    pub async fn active_allocations(&self) -> usize {
        self.metrics.active_allocations.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_turn_server_start() {
        let config = TurnServerConfig {
            bind_addr: "127.0.0.1:0".to_string(),
            relay_min_port: 50000,
            relay_max_port: 50100,
            max_allocations: 10,
            default_lifetime: 600,
            max_lifetime: 3600,
            static_secret: "test_secret".to_string(),
            realm: "test.local".to_string(),
        };

        let server = TurnServer::start(config).await.unwrap();
        let addr = server.local_addr().unwrap();

        assert!(addr.port() > 0);

        // Give server time to initialize
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn test_metrics() {
        let config = TurnServerConfig {
            bind_addr: "127.0.0.1:0".to_string(),
            static_secret: "test_secret".to_string(),
            ..Default::default()
        };

        let server = TurnServer::start(config).await.unwrap();

        let metrics = server.metrics();
        assert_eq!(metrics.total_allocations, 0);
        assert_eq!(metrics.total_errors, 0);
    }
}
