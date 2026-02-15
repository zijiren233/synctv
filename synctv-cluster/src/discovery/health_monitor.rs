//! Health monitoring for cluster nodes
//!
//! Tracks node health via periodic heartbeats and active TCP health probes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;

use super::node_registry::NodeRegistry;
#[allow(unused_imports)]
use futures::future::join_all;
use crate::error::Result;

/// Health status of a node
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeHealth {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Configuration for active health probes
#[derive(Debug, Clone)]
pub struct HealthProbeConfig {
    /// TCP connection timeout for health probes
    pub probe_timeout_secs: u64,
    /// Number of consecutive probe failures before marking unhealthy
    pub failure_threshold: u32,
    /// Number of consecutive probe successes before marking healthy
    pub success_threshold: u32,
    /// Interval between active probes (separate from heartbeat checks)
    pub probe_interval_secs: u64,
}

impl Default for HealthProbeConfig {
    fn default() -> Self {
        Self {
            probe_timeout_secs: 3,
            failure_threshold: 2,
            success_threshold: 1,
            probe_interval_secs: 15,
        }
    }
}

/// Probe health state for a single node
#[derive(Debug, Default)]
struct ProbeState {
    /// Consecutive successful probes
    success_count: AtomicU32,
    /// Consecutive failed probes
    failure_count: AtomicU32,
}

/// Health monitor for cluster nodes
///
/// Periodically checks node health via:
/// 1. Passive heartbeat monitoring (based on last_heartbeat timestamp)
/// 2. Active TCP health probes (connect to gRPC port)
pub struct HealthMonitor {
    node_registry: Arc<NodeRegistry>,
    check_interval_secs: u64,
    health_status: Arc<RwLock<std::collections::HashMap<String, NodeHealth>>>,
    cancel_token: CancellationToken,
    /// Active probe configuration
    probe_config: HealthProbeConfig,
    /// Probe state per node
    probe_states: Arc<RwLock<std::collections::HashMap<String, ProbeState>>>,
}

impl HealthMonitor {
    /// Create a new health monitor with default probe config
    #[must_use]
    pub fn new(node_registry: Arc<NodeRegistry>, check_interval_secs: u64) -> Self {
        Self {
            node_registry,
            check_interval_secs,
            health_status: Arc::new(RwLock::new(std::collections::HashMap::new())),
            cancel_token: CancellationToken::new(),
            probe_config: HealthProbeConfig::default(),
            probe_states: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Create a new health monitor with custom probe configuration
    #[must_use]
    pub fn with_probe_config(node_registry: Arc<NodeRegistry>, check_interval_secs: u64, probe_config: HealthProbeConfig) -> Self {
        Self {
            node_registry,
            check_interval_secs,
            health_status: Arc::new(RwLock::new(std::collections::HashMap::new())),
            cancel_token: CancellationToken::new(),
            probe_config,
            probe_states: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Start health monitoring loop
    ///
    /// Returns the `JoinHandle` so the caller can detect panics or task completion.
    /// Use `shutdown()` to gracefully stop the monitoring loop.
    pub async fn start(&self) -> Result<tokio::task::JoinHandle<()>> {
        let registry = self.node_registry.clone();
        let health_status = self.health_status.clone();
        let timeout_secs = registry.heartbeat_timeout_secs;
        let cancel_token = self.cancel_token.clone();
        let probe_config = self.probe_config.clone();
        let probe_states = self.probe_states.clone();

        let mut timer = interval(Duration::from_secs(self.check_interval_secs));
        let mut probe_timer = interval(Duration::from_secs(probe_config.probe_interval_secs));

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = cancel_token.cancelled() => {
                        tracing::info!("Health monitor shutting down");
                        return;
                    }
                    _ = timer.tick() => {
                        // Passive heartbeat check
                        Self::check_heartbeats(&registry, &health_status, timeout_secs).await;
                    }
                    _ = probe_timer.tick() => {
                        // Active TCP probe
                        Self::probe_nodes(&registry, &health_status, &probe_config, &probe_states).await;
                    }
                }
            }
        });

        Ok(handle)
    }

    /// Check heartbeats for all nodes (passive check)
    async fn check_heartbeats(
        registry: &Arc<NodeRegistry>,
        health_status: &Arc<RwLock<std::collections::HashMap<String, NodeHealth>>>,
        timeout_secs: i64,
    ) {
        match registry.get_all_nodes().await {
            Ok(nodes) => {
                let mut status = health_status.write().await;
                let node_ids: std::collections::HashSet<String> = nodes.iter().map(|n| n.node_id.clone()).collect();

                for node in nodes {
                    // Only update to unhealthy based on heartbeat; probes may override
                    let is_alive = !node.is_stale(timeout_secs);
                    if !is_alive {
                        // Heartbeat expired - mark unhealthy immediately
                        let old_status = status.get(&node.node_id);
                        if old_status != Some(&NodeHealth::Unhealthy) {
                            tracing::warn!(
                                node_id = %node.node_id,
                                last_heartbeat = ?node.last_heartbeat,
                                "Node marked unhealthy: heartbeat expired"
                            );
                        }
                        status.insert(node.node_id.clone(), NodeHealth::Unhealthy);
                    }
                    // If heartbeat is alive, don't override - let probes decide
                }

                // Remove nodes that are no longer in registry
                status.retain(|node_id, _| node_ids.contains(node_id));
            }
            Err(e) => {
                tracing::error!("Failed to get nodes for health check: {}", e);
            }
        }
    }

    /// Perform active TCP probes on all nodes concurrently
    async fn probe_nodes(
        registry: &Arc<NodeRegistry>,
        health_status: &Arc<RwLock<std::collections::HashMap<String, NodeHealth>>>,
        probe_config: &HealthProbeConfig,
        probe_states: &Arc<RwLock<std::collections::HashMap<String, ProbeState>>>,
    ) {
        let nodes = match registry.get_all_nodes().await {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("Failed to get nodes for probe: {}", e);
                return;
            }
        };

        // Filter nodes to probe (skip those unhealthy with stale heartbeats)
        let heartbeat_timeout = registry.heartbeat_timeout_secs;
        let nodes_to_probe: Vec<_> = {
            let hs = health_status.read().await;
            nodes.into_iter().filter(|node| {
                let current_status = hs.get(&node.node_id).copied();
                if current_status == Some(NodeHealth::Unhealthy) {
                    // Only probe if heartbeat has recovered
                    !node.is_stale(heartbeat_timeout)
                } else {
                    true
                }
            }).collect()
        };

        // Probe all nodes concurrently
        let probe_timeout = probe_config.probe_timeout_secs;
        let probe_results: Vec<_> = futures::future::join_all(
            nodes_to_probe.iter().map(|node| {
                let addr = node.grpc_address.clone();
                async move {
                    Self::probe_node_static(&addr, probe_timeout).await
                }
            })
        ).await;

        // Process results
        let mut states = probe_states.write().await;
        let mut hs = health_status.write().await;
        let active_node_ids: std::collections::HashSet<String> =
            nodes_to_probe.iter().map(|n| n.node_id.clone()).collect();

        for (node, probe_success) in nodes_to_probe.iter().zip(probe_results) {
            let state = states.entry(node.node_id.clone()).or_default();

            let new_status = if probe_success {
                state.failure_count.store(0, Ordering::Relaxed);
                let successes = state.success_count.fetch_add(1, Ordering::Relaxed) + 1;

                if successes >= probe_config.success_threshold {
                    Some(NodeHealth::Healthy)
                } else {
                    None
                }
            } else {
                state.success_count.store(0, Ordering::Relaxed);
                let failures = state.failure_count.fetch_add(1, Ordering::Relaxed) + 1;

                if failures >= probe_config.failure_threshold {
                    if failures == probe_config.failure_threshold {
                        tracing::warn!(
                            node_id = %node.node_id,
                            consecutive_failures = failures,
                            "Node marked as unhealthy after consecutive probe failures"
                        );
                    }
                    Some(NodeHealth::Unhealthy)
                } else {
                    Some(NodeHealth::Degraded)
                }
            };

            if let Some(status) = new_status {
                let old_status = hs.get(&node.node_id);

                if old_status != Some(&status) {
                    match status {
                        NodeHealth::Healthy => {
                            tracing::info!(node_id = %node.node_id, "Node is healthy (probe)");
                        }
                        NodeHealth::Degraded => {
                            tracing::warn!(node_id = %node.node_id, "Node is degraded (probe)");
                        }
                        NodeHealth::Unhealthy => {
                            tracing::warn!(node_id = %node.node_id, "Node is unhealthy (probe)");
                        }
                    }
                }

                hs.insert(node.node_id.clone(), status);
            }
        }

        // Prune probe_states for nodes no longer in the registry
        states.retain(|node_id, _| active_node_ids.contains(node_id));
    }

    /// Static probe function for use in async context.
    /// Supports both IPv4 ("host:port") and IPv6 ("[::1]:port") addresses.
    async fn probe_node_static(grpc_address: &str, timeout_secs: u64) -> bool {
        // Try parsing as a SocketAddr first (handles IPv6 like [::1]:50051)
        // then fall back to rsplit_once for "host:port" format
        let addr = if grpc_address.parse::<std::net::SocketAddr>().is_ok() {
            grpc_address.to_string()
        } else if let Some((host, port_str)) = grpc_address.rsplit_once(':') {
            if port_str.parse::<u16>().is_err() {
                return false;
            }
            format!("{host}:{port_str}")
        } else {
            return false;
        };

        let result = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            tokio::net::TcpStream::connect(&addr),
        )
        .await;

        match result {
            Ok(Ok(_)) => true,
            Ok(Err(_)) | Err(_) => false,
        }
    }

    /// Gracefully shut down the health monitoring loop
    pub fn shutdown(&self) {
        self.cancel_token.cancel();
    }

    /// Get health status of all nodes
    pub async fn get_all_status(&self) -> std::collections::HashMap<String, NodeHealth> {
        let status = self.health_status.read().await;
        status.clone()
    }

    /// Get health status of a specific node
    pub async fn get_node_status(&self, node_id: &str) -> Option<NodeHealth> {
        let status = self.health_status.read().await;
        status.get(node_id).copied()
    }
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    #[ignore = "Requires Redis"]
    async fn test_health_monitor() {
        // Integration test placeholder
    }
}
