//! Load balancing for cluster requests
//!
//! Distributes requests across available cluster nodes.

use rand::seq::SliceRandom;
use std::sync::Arc;

use super::node_registry::NodeRegistry;
use crate::error::{Error, Result};

/// Load balancing strategy
#[derive(Debug, Clone, Copy)]
pub enum LoadBalancingStrategy {
    /// Random selection
    Random,
    /// Round-robin
    RoundRobin,
    /// Least connections (select node with fewest active connections)
    LeastConnections,
}

/// Load balancer for cluster node selection
pub struct LoadBalancer {
    node_registry: Arc<NodeRegistry>,
    strategy: LoadBalancingStrategy,
    round_robin_index: std::sync::atomic::AtomicUsize,
}

impl LoadBalancer {
    /// Create a new load balancer
    #[must_use] 
    pub const fn new(node_registry: Arc<NodeRegistry>, strategy: LoadBalancingStrategy) -> Self {
        Self {
            node_registry,
            strategy,
            round_robin_index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Select a node for the next request
    pub async fn select_node(&self) -> Result<String> {
        let nodes = self.node_registry.get_all_nodes().await?;

        if nodes.is_empty() {
            return Err(Error::NotFound("No available nodes".to_string()));
        }

        // Filter out this node (if we want to avoid self-selection)
        // For now, include all nodes

        let selected_node = match self.strategy {
            LoadBalancingStrategy::Random => {
                nodes
                    .choose(&mut rand::thread_rng())
                    .ok_or_else(|| Error::NotFound("No nodes available".to_string()))?
                    .node_id
                    .clone()
            }
            LoadBalancingStrategy::RoundRobin => {
                let index = self
                    .round_robin_index
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
                    % nodes.len();
                nodes[index].node_id.clone()
            }
            LoadBalancingStrategy::LeastConnections => {
                // Select node with fewest connections based on metadata
                // Nodes report connection count in metadata["connections"]
                nodes
                    .iter()
                    .min_by_key(|n| {
                        n.metadata
                            .get("connections")
                            .and_then(|v| v.parse::<usize>().ok())
                            .unwrap_or(0)
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

    /// Get all available nodes
    pub async fn get_available_nodes(&self) -> Result<Vec<String>> {
        let nodes = self.node_registry.get_all_nodes().await?;
        Ok(nodes.into_iter().map(|n| n.node_id).collect())
    }

    /// Get count of available nodes
    pub async fn available_count(&self) -> Result<usize> {
        let nodes = self.node_registry.get_all_nodes().await?;
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
