//! Load balancing for cluster requests
//!
//! Distributes requests across available cluster nodes.
//! Integrates with `HealthMonitor` to exclude unhealthy nodes.

use rand::seq::SliceRandom;
use std::sync::Arc;

use super::health_monitor::{HealthMonitor, NodeHealth};
use super::node_registry::{NodeInfo, NodeRegistry};
use crate::error::{Error, Result};

/// Load balancing strategy
#[derive(Debug, Clone, Copy)]
pub enum LoadBalancingStrategy {
    /// Random selection
    Random,
    /// Round-robin
    RoundRobin,
    /// Least connections (select node with fewest active connections)
    /// Nodes must report connection count in metadata["connections"] via heartbeat.
    LeastConnections,
}

/// Load balancer for cluster node selection
pub struct LoadBalancer {
    node_registry: Arc<NodeRegistry>,
    health_monitor: Option<Arc<HealthMonitor>>,
    strategy: LoadBalancingStrategy,
    round_robin_index: std::sync::atomic::AtomicUsize,
}

impl LoadBalancer {
    /// Create a new load balancer
    #[must_use]
    pub const fn new(node_registry: Arc<NodeRegistry>, strategy: LoadBalancingStrategy) -> Self {
        Self {
            node_registry,
            health_monitor: None,
            strategy,
            round_robin_index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Attach a health monitor to filter out unhealthy nodes
    #[must_use]
    pub fn with_health_monitor(mut self, monitor: Arc<HealthMonitor>) -> Self {
        self.health_monitor = Some(monitor);
        self
    }

    /// Get healthy nodes, filtering by health monitor if available
    async fn get_healthy_nodes(&self) -> Result<Vec<NodeInfo>> {
        let nodes = self.node_registry.get_all_nodes().await?;

        // If no health monitor, return all nodes (stale nodes already filtered by registry)
        let Some(ref monitor) = self.health_monitor else {
            return Ok(nodes);
        };

        let statuses = monitor.get_all_status().await;

        let healthy: Vec<NodeInfo> = nodes
            .into_iter()
            .filter(|n| {
                statuses
                    .get(&n.node_id)
                    .is_none_or(|s| *s != NodeHealth::Unhealthy)
            })
            .collect();

        Ok(healthy)
    }

    /// Select a node for the next request
    pub async fn select_node(&self) -> Result<String> {
        let nodes = self.get_healthy_nodes().await?;

        if nodes.is_empty() {
            return Err(Error::NotFound("No available healthy nodes".to_string()));
        }

        let selected_node = match self.strategy {
            LoadBalancingStrategy::Random => {
                nodes
                    .choose(&mut rand::thread_rng())
                    .ok_or_else(|| Error::NotFound("No nodes available".to_string()))?
                    .node_id
                    .clone()
            }
            LoadBalancingStrategy::RoundRobin => {
                // Sort by node_id for stable ordering across calls
                let mut sorted = nodes;
                sorted.sort_by(|a, b| a.node_id.cmp(&b.node_id));
                let index = self
                    .round_robin_index
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
                    % sorted.len();
                sorted[index].node_id.clone()
            }
            LoadBalancingStrategy::LeastConnections => {
                // Select node with fewest connections based on metadata.
                // Nodes report connection count in metadata["connections"] via heartbeat.
                //
                // For nodes without connection metadata (newly joined), we use a
                // warmup penalty to avoid immediately routing traffic to them.
                // The penalty is based on registered_at timestamp - nodes registered
                // within the last 60 seconds get a higher effective connection count
                // to reduce the "thundering herd" problem on new nodes.
                const WARMUP_PERIOD_SECS: i64 = 60;
                const WARMUP_PENALTY: usize = 1000; // High value to deprioritize new nodes

                let now = chrono::Utc::now();

                nodes
                    .iter()
                    .min_by_key(|n| {
                        let connections = n.metadata
                            .get("connections")
                            .and_then(|v| v.parse::<usize>().ok());

                        match connections {
                            Some(conn) => conn,
                            None => {
                                // Node hasn't reported connections - check if in warmup
                                let registered_at = n.metadata
                                    .get("registered_at")
                                    .and_then(|v| v.parse::<i64>().ok())
                                    .unwrap_or(0);

                                let age_secs = now.timestamp() - registered_at;
                                if age_secs < WARMUP_PERIOD_SECS {
                                    // In warmup period - apply penalty that decreases over time
                                    let warmup_progress = age_secs as f64 / WARMUP_PERIOD_SECS as f64;
                                    let penalty = (WARMUP_PENALTY as f64 * (1.0 - warmup_progress)) as usize;
                                    tracing::trace!(
                                        node_id = %n.node_id,
                                        age_secs = age_secs,
                                        effective_connections = penalty,
                                        "Node in warmup period"
                                    );
                                    penalty
                                } else {
                                    // Past warmup but still no connection data - use moderate value
                                    tracing::debug!(
                                        node_id = %n.node_id,
                                        "Node has no connection metadata, using default"
                                    );
                                    500 // Moderate default
                                }
                            }
                        }
                    })
                    .ok_or_else(|| Error::NotFound("No nodes available".to_string()))?
                    .node_id
                    .clone()
            }
        };

        Ok(selected_node)
    }

    /// Select a specific node by ID (returns error if node not available)
    pub async fn select_node_by_id(&self, node_id: &str) -> Result<String> {
        let node = self
            .node_registry
            .get_node(node_id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Node {node_id} not found")))?;

        Ok(node.node_id)
    }

    /// Get all available healthy nodes
    pub async fn get_available_nodes(&self) -> Result<Vec<String>> {
        let nodes = self.get_healthy_nodes().await?;
        Ok(nodes.into_iter().map(|n| n.node_id).collect())
    }

    /// Get count of available healthy nodes
    pub async fn available_count(&self) -> Result<usize> {
        let nodes = self.get_healthy_nodes().await?;
        Ok(nodes.len())
    }
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    #[ignore = "Requires Redis"]
    async fn test_load_balancer() {
        // Integration test placeholder
    }
}
