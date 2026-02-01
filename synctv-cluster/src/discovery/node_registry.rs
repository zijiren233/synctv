//! Node registry for cluster member discovery
//!
//! Uses Redis to track active nodes in the cluster.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::{Error, Result};

/// Node information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub node_id: String,
    pub grpc_address: String,
    pub http_address: String,
    pub last_heartbeat: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
}

impl NodeInfo {
    pub fn new(node_id: String, grpc_address: String, http_address: String) -> Self {
        Self {
            node_id,
            grpc_address,
            http_address,
            last_heartbeat: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    /// Check if node is stale (no recent heartbeat)
    pub fn is_stale(&self, timeout_secs: i64) -> bool {
        let now = Utc::now();
        let elapsed = now.signed_duration_since(self.last_heartbeat);
        elapsed.num_seconds() > timeout_secs
    }
}

/// Redis-based node registry
///
/// Tracks active nodes in the cluster using Redis key expiration.
pub struct NodeRegistry {
    redis_client: Option<redis::Client>,
    node_id: String,
    pub heartbeat_timeout_secs: i64,
    local_nodes: Arc<RwLock<HashMap<String, NodeInfo>>>,
}

impl NodeRegistry {
    /// Create a new node registry
    ///
    /// If Redis URL is None, operates in local-only mode (useful for single-node deployments).
    pub fn new(redis_url: Option<String>, node_id: String, heartbeat_timeout_secs: i64) -> Result<Self> {
        let redis_client = if let Some(url) = redis_url {
            Some(
                redis::Client::open(url)
                    .map_err(|e| Error::Configuration(format!("Failed to connect to Redis: {}", e)))?,
            )
        } else {
            None
        };

        Ok(Self {
            redis_client,
            node_id,
            heartbeat_timeout_secs,
            local_nodes: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Register this node in the registry
    pub async fn register(&self, grpc_address: String, http_address: String) -> Result<()> {
        let node_info = NodeInfo::new(self.node_id.clone(), grpc_address, http_address);

        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| Error::Database(format!("Redis connection failed: {}", e)))?;

            let key = Self::node_key(&self.node_id);
            let value = serde_json::to_string(&node_info)
                .map_err(|e| Error::Serialization(format!("Failed to serialize node info: {}", e)))?;

            // Set with expiration (heartbeat_timeout * 2 for safety)
            redis::cmd("SETEX")
                .arg(&key)
                .arg(self.heartbeat_timeout_secs * 2)
                .arg(&value)
                .query_async::<()>(&mut conn)
                .await
                .map_err(|e| Error::Database(format!("Redis SETEX failed: {}", e)))?;
        }

        // Also update local cache
        let mut nodes = self.local_nodes.write().await;
        nodes.insert(self.node_id.clone(), node_info);

        Ok(())
    }

    /// Send heartbeat to keep this node alive
    pub async fn heartbeat(&self) -> Result<()> {
        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| Error::Database(format!("Redis connection failed: {}", e)))?;

            let key = Self::node_key(&self.node_id);

            // Update TTL (keep alive)
            redis::cmd("EXPIRE")
                .arg(&key)
                .arg(self.heartbeat_timeout_secs * 2)
                .query_async::<()>(&mut conn)
                .await
                .map_err(|e| Error::Database(format!("Redis EXPIRE failed: {}", e)))?;
        }

        // Update local heartbeat time
        let mut nodes = self.local_nodes.write().await;
        if let Some(node) = nodes.get_mut(&self.node_id) {
            node.last_heartbeat = Utc::now();
        }

        Ok(())
    }

    /// Unregister this node
    pub async fn unregister(&self) -> Result<()> {
        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| Error::Database(format!("Redis connection failed: {}", e)))?;

            let key = Self::node_key(&self.node_id);

            redis::cmd("DEL")
                .arg(&key)
                .query_async::<()>(&mut conn)
                .await
                .map_err(|e| Error::Database(format!("Redis DEL failed: {}", e)))?;
        }

        // Remove from local cache
        let mut nodes = self.local_nodes.write().await;
        nodes.remove(&self.node_id);

        Ok(())
    }

    /// Get all active nodes
    pub async fn get_all_nodes(&self) -> Result<Vec<NodeInfo>> {
        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| Error::Database(format!("Redis connection failed: {}", e)))?;

            // Get all node keys
            let pattern = format!("{}:*", Self::KEY_PREFIX);
            let keys: Vec<String> = redis::cmd("KEYS")
                .arg(&pattern)
                .query_async(&mut conn)
                .await
                .map_err(|e| Error::Database(format!("Redis KEYS failed: {}", e)))?;

            let mut nodes = Vec::new();
            for key in keys {
                let value: Option<String> = redis::cmd("GET")
                    .arg(&key)
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| Error::Database(format!("Redis GET failed: {}", e)))?;

                if let Some(value) = value {
                    if let Ok(node_info) = serde_json::from_str::<NodeInfo>(&value) {
                        if !node_info.is_stale(self.heartbeat_timeout_secs) {
                            nodes.push(node_info);
                        }
                    }
                }
            }

            // Update local cache
            let mut local_nodes = self.local_nodes.write().await;
            local_nodes.clear();
            for node in &nodes {
                local_nodes.insert(node.node_id.clone(), node.clone());
            }

            Ok(nodes)
        } else {
            // Local mode: return cached nodes
            let nodes = self.local_nodes.read().await;
            Ok(nodes.values().cloned().collect())
        }
    }

    /// Get a specific node by ID
    pub async fn get_node(&self, node_id: &str) -> Result<Option<NodeInfo>> {
        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| Error::Database(format!("Redis connection failed: {}", e)))?;

            let key = Self::node_key(node_id);
            let value: Option<String> = redis::cmd("GET")
                .arg(&key)
                .query_async(&mut conn)
                .await
                .map_err(|e| Error::Database(format!("Redis GET failed: {}", e)))?;

            if let Some(value) = value {
                let node_info: NodeInfo = serde_json::from_str(&value)
                    .map_err(|e| Error::Serialization(format!("Failed to deserialize node info: {}", e)))?;

                if node_info.is_stale(self.heartbeat_timeout_secs) {
                    return Ok(None);
                }

                Ok(Some(node_info))
            } else {
                Ok(None)
            }
        } else {
            // Local mode: check cache
            let nodes = self.local_nodes.read().await;
            Ok(nodes.get(node_id).cloned())
        }
    }

    /// Redis key prefix for nodes
    const KEY_PREFIX: &'static str = "synctv:cluster:nodes";

    fn node_key(node_id: &str) -> String {
        format!("{}:{}", Self::KEY_PREFIX, node_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_info_stale() {
        let mut node = NodeInfo::new(
            "test".to_string(),
            "localhost:50051".to_string(),
            "localhost:8080".to_string(),
        );

        // Fresh node should not be stale
        assert!(!node.is_stale(30));

        // Simulate old heartbeat
        node.last_heartbeat = Utc::now() - Duration::seconds(60);
        assert!(node.is_stale(30));
    }
}
