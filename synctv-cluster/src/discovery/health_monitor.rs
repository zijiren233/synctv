//! Health monitoring for cluster nodes
//!
//! Tracks node health via periodic heartbeats.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;

use super::node_registry::NodeRegistry;
use crate::error::Result;

/// Health status of a node
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeHealth {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Health monitor for cluster nodes
///
/// Periodically checks node health and triggers callbacks on status changes.
pub struct HealthMonitor {
    node_registry: Arc<NodeRegistry>,
    check_interval_secs: u64,
    health_status: Arc<RwLock<std::collections::HashMap<String, NodeHealth>>>,
}

impl HealthMonitor {
    /// Create a new health monitor
    #[must_use] 
    pub fn new(node_registry: Arc<NodeRegistry>, check_interval_secs: u64) -> Self {
        Self {
            node_registry,
            check_interval_secs,
            health_status: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Start health monitoring loop
    ///
    /// Returns the `JoinHandle` so the caller can detect panics or task completion.
    pub async fn start(&self) -> Result<tokio::task::JoinHandle<()>> {
        let registry = self.node_registry.clone();
        let health_status = self.health_status.clone();
        let timeout_secs = registry.heartbeat_timeout_secs;

        let mut timer = interval(Duration::from_secs(self.check_interval_secs));

        let handle = tokio::spawn(async move {
            loop {
                timer.tick().await;

                match registry.get_all_nodes().await {
                    Ok(nodes) => {
                        let mut status = health_status.write().await;
                        let node_ids: std::collections::HashSet<String> = nodes.iter().map(|n| n.node_id.clone()).collect();

                        for node in nodes {
                            let is_alive = !node.is_stale(timeout_secs);
                            let node_health = if is_alive {
                                NodeHealth::Healthy
                            } else {
                                NodeHealth::Unhealthy
                            };

                            let old_status = status.get(&node.node_id);

                            // Log status changes
                            if old_status != Some(&node_health) {
                                match node_health {
                                    NodeHealth::Healthy => {
                                        tracing::info!(
                                            node_id = %node.node_id,
                                            "Node is healthy"
                                        );
                                    }
                                    NodeHealth::Unhealthy => {
                                        tracing::warn!(
                                            node_id = %node.node_id,
                                            "Node is unhealthy (no heartbeat)"
                                        );
                                    }
                                    NodeHealth::Degraded => {
                                        tracing::warn!(
                                            node_id = %node.node_id,
                                            "Node is degraded"
                                        );
                                    }
                                }
                            }

                            status.insert(node.node_id.clone(), node_health);
                        }

                        // Remove nodes that are no longer in registry
                        status.retain(|node_id, _| {
                            node_ids.contains(node_id)
                        });
                    }
                    Err(e) => {
                        tracing::error!("Failed to get nodes for health check: {}", e);
                    }
                }
            }
        });

        Ok(handle)
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
