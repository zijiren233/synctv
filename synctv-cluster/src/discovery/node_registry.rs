//! Node registry for cluster member discovery
//!
//! Uses Redis to track active nodes in the cluster.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};

use crate::error::{Error, Result};

/// Timeout for Redis operations in seconds
const REDIS_TIMEOUT_SECS: u64 = 5;

/// Node information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub node_id: String,
    pub grpc_address: String,
    pub http_address: String,
    pub last_heartbeat: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
    /// Fencing token (epoch) for split-brain protection
    /// Increments on each registration to prevent stale updates
    #[serde(default)]
    pub epoch: u64,
}

impl NodeInfo {
    #[must_use]
    pub fn new(node_id: String, grpc_address: String, http_address: String) -> Self {
        Self {
            node_id,
            grpc_address,
            http_address,
            last_heartbeat: Utc::now(),
            metadata: HashMap::new(),
            epoch: 1, // Start at epoch 1
        }
    }

    /// Create with a specific epoch (for re-registration)
    #[must_use]
    pub fn with_epoch(mut self, epoch: u64) -> Self {
        self.epoch = epoch;
        self
    }

    /// Check if node is stale (no recent heartbeat)
    #[must_use]
    pub fn is_stale(&self, timeout_secs: i64) -> bool {
        let now = Utc::now();
        let elapsed = now.signed_duration_since(self.last_heartbeat);
        elapsed.num_seconds() > timeout_secs
    }

    /// Get the fencing token for this node
    #[must_use]
    pub fn fencing_token(&self) -> FencingToken {
        FencingToken::new(self.node_id.clone(), self.epoch)
    }
}

/// Fencing token for split-brain protection
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FencingToken {
    pub node_id: String,
    pub epoch: u64,
}

impl FencingToken {
    /// Create a new fencing token
    #[must_use]
    pub fn new(node_id: String, epoch: u64) -> Self {
        Self { node_id, epoch }
    }

    /// Check if this token is newer than another (same node, higher epoch)
    #[must_use]
    pub fn is_newer_than(&self, other: &FencingToken) -> bool {
        self.node_id == other.node_id && self.epoch > other.epoch
    }
}

/// Redis-based node registry
///
/// Tracks active nodes in the cluster using Redis key expiration.
/// Uses epoch-based fencing tokens to prevent split-brain scenarios.
pub struct NodeRegistry {
    redis_client: Option<redis::Client>,
    node_id: String,
    pub heartbeat_timeout_secs: i64,
    local_nodes: Arc<RwLock<HashMap<String, NodeInfo>>>,
    /// Current epoch for this node (incremented on each registration)
    current_epoch: Arc<std::sync::atomic::AtomicU64>,
}

impl NodeRegistry {
    /// Create a new node registry
    ///
    /// If Redis URL is None, operates in local-only mode (useful for single-node deployments).
    pub fn new(redis_url: Option<String>, node_id: String, heartbeat_timeout_secs: i64) -> Result<Self> {
        let redis_client = if let Some(url) = redis_url {
            Some(
                redis::Client::open(url)
                    .map_err(|e| Error::Configuration(format!("Failed to connect to Redis: {e}")))?,
            )
        } else {
            None
        };

        Ok(Self {
            redis_client,
            node_id,
            heartbeat_timeout_secs,
            local_nodes: Arc::new(RwLock::new(HashMap::new())),
            current_epoch: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        })
    }

    /// Get the current fencing token for this node
    #[must_use]
    pub fn current_fencing_token(&self) -> FencingToken {
        FencingToken::new(
            self.node_id.clone(),
            self.current_epoch.load(std::sync::atomic::Ordering::SeqCst),
        )
    }

    /// Register this node in the registry with epoch-based fencing
    ///
    /// This operation is atomic - it uses a Lua script to atomically:
    /// 1. Read existing epoch
    /// 2. Increment epoch
    /// 3. Write new registration with TTL
    ///
    /// This prevents race conditions when multiple instances register concurrently.
    pub async fn register(&self, grpc_address: String, http_address: String) -> Result<()> {
        if let Some(ref client) = self.redis_client {
            let mut conn = timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                client.get_multiplexed_async_connection(),
            )
            .await
            .map_err(|_| Error::Timeout("Redis connection timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis connection failed: {e}")))?;

            let key = Self::node_key(&self.node_id);
            let local_epoch = self.current_epoch.load(std::sync::atomic::Ordering::SeqCst);
            let ttl = self.heartbeat_timeout_secs * 2;

            // Create node info template
            let mut node_info = NodeInfo::new(self.node_id.clone(), grpc_address, http_address);
            node_info.metadata.insert("local_epoch".to_string(), local_epoch.to_string());
            let node_json = serde_json::to_string(&node_info)
                .map_err(|e| Error::Serialization(format!("Failed to serialize node info: {e}")))?;

            // Atomic Lua script: read epoch, increment, write with TTL
            // Returns the new epoch assigned
            let script = redis::Script::new(
                r#"
                local key = KEYS[1]
                local new_node_json = ARGV[1]
                local ttl = tonumber(ARGV[2])
                local local_epoch = tonumber(ARGV[3])
                local node_id = ARGV[4]

                -- Parse incoming node info
                local new_node = cjson.decode(new_node_json)

                -- Read existing value
                local existing = redis.call('GET', key)
                local existing_epoch = 0

                if existing then
                    local existing_info = cjson.decode(existing)
                    -- Only use existing epoch if it's the same node
                    if existing_info.node_id == node_id then
                        existing_epoch = existing_info.epoch or 0
                    end
                end

                -- Calculate new epoch: max(existing + 1, local_epoch + 1, 1)
                local new_epoch = math.max(existing_epoch + 1, local_epoch + 1, 1)

                -- Update node info with new epoch and current timestamp
                new_node['epoch'] = new_epoch
                new_node['last_heartbeat'] = nil  -- Let the Go side set this

                -- Write with TTL
                local final_json = cjson.encode(new_node)
                redis.call('SETEX', key, ttl, final_json)

                return new_epoch
                "#,
            );

            let new_epoch: u64 = timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                script
                    .key(&key)
                    .arg(&node_json)
                    .arg(ttl)
                    .arg(local_epoch)
                    .arg(&self.node_id)
                    .invoke_async(&mut conn),
            )
            .await
            .map_err(|_| Error::Timeout("Redis register script timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis register script failed: {e}")))?;

            // Update local epoch
            self.current_epoch.store(new_epoch, std::sync::atomic::Ordering::SeqCst);

            // Update local cache
            node_info.epoch = new_epoch;
            node_info.last_heartbeat = Utc::now();
            let mut nodes = self.local_nodes.write().await;
            nodes.insert(self.node_id.clone(), node_info);

            tracing::debug!(
                node_id = %self.node_id,
                epoch = new_epoch,
                "Node registered with fencing token (atomic)"
            );
        } else {
            // Local-only mode
            let node_info = NodeInfo::new(self.node_id.clone(), grpc_address, http_address);
            let mut nodes = self.local_nodes.write().await;
            nodes.insert(self.node_id.clone(), node_info);
        }

        Ok(())
    }

    /// Send heartbeat to keep this node alive with fencing token validation
    ///
    /// If the node's epoch in Redis doesn't match our local epoch, this indicates
    /// another registration happened (possibly split-brain). We should re-register.
    pub async fn heartbeat(&self) -> Result<()> {
        if let Some(ref client) = self.redis_client {
            let mut conn = timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                client.get_multiplexed_async_connection(),
            )
            .await
            .map_err(|_| Error::Timeout("Redis connection timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis connection failed: {e}")))?;

            let key = Self::node_key(&self.node_id);
            let current_epoch = self.current_epoch.load(std::sync::atomic::Ordering::SeqCst);

            // Read current value to verify epoch
            let existing: Option<String> = timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                redis::cmd("GET")
                    .arg(&key)
                    .query_async(&mut conn),
            )
            .await
            .map_err(|_| Error::Timeout("Redis GET timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis GET failed: {e}")))?;

            if let Some(existing) = existing {
                if let Ok(existing_info) = serde_json::from_str::<NodeInfo>(&existing) {
                    // Check for epoch mismatch (indicates split-brain or re-registration)
                    if existing_info.epoch != current_epoch {
                        tracing::warn!(
                            node_id = %self.node_id,
                            local_epoch = current_epoch,
                            remote_epoch = existing_info.epoch,
                            "Epoch mismatch during heartbeat, node may need re-registration"
                        );
                        // Update our epoch to match the remote one
                        self.current_epoch.store(
                            existing_info.epoch.max(current_epoch),
                            std::sync::atomic::Ordering::SeqCst
                        );
                    }
                }
            }

            // Update heartbeat timestamp and epoch in Redis
            let node_info = {
                let nodes = self.local_nodes.read().await;
                let mut info = nodes.get(&self.node_id).cloned().unwrap_or_else(|| {
                    NodeInfo::new(self.node_id.clone(), String::new(), String::new())
                });
                info.last_heartbeat = Utc::now();
                info.epoch = current_epoch;
                info
            };

            let value = serde_json::to_string(&node_info)
                .map_err(|e| Error::Serialization(format!("Failed to serialize node info: {e}")))?;

            // Update value with expiration (atomic SETEX)
            timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                redis::cmd("SETEX")
                    .arg(&key)
                    .arg(self.heartbeat_timeout_secs * 2)
                    .arg(&value)
                    .query_async::<()>(&mut conn),
            )
            .await
            .map_err(|_| Error::Timeout("Redis SETEX timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis SETEX failed: {e}")))?;
        }

        // Update local heartbeat time
        let mut nodes = self.local_nodes.write().await;
        if let Some(node) = nodes.get_mut(&self.node_id) {
            node.last_heartbeat = Utc::now();
        }

        Ok(())
    }

    /// Unregister this node with fencing token validation
    ///
    /// Only allows unregistration if our epoch matches Redis epoch.
    /// Prevents stale nodes from unregistering newer registrations.
    pub async fn unregister(&self) -> Result<()> {
        if let Some(ref client) = self.redis_client {
            let mut conn = timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                client.get_multiplexed_async_connection(),
            )
            .await
            .map_err(|_| Error::Timeout("Redis connection timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis connection failed: {e}")))?;

            let key = Self::node_key(&self.node_id);
            let current_epoch = self.current_epoch.load(std::sync::atomic::Ordering::SeqCst);

            // Read and verify epoch before deleting
            let existing: Option<String> = timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                redis::cmd("GET")
                    .arg(&key)
                    .query_async(&mut conn),
            )
            .await
            .map_err(|_| Error::Timeout("Redis GET timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis GET failed: {e}")))?;

            if let Some(existing) = existing {
                if let Ok(existing_info) = serde_json::from_str::<NodeInfo>(&existing) {
                    if existing_info.epoch > current_epoch {
                        // A newer registration exists, don't delete it
                        tracing::warn!(
                            node_id = %self.node_id,
                            local_epoch = current_epoch,
                            remote_epoch = existing_info.epoch,
                            "Skipping unregister: newer registration exists in Redis"
                        );
                        // Still remove from local cache
                        let mut nodes = self.local_nodes.write().await;
                        nodes.remove(&self.node_id);
                        return Ok(());
                    }
                }
            }

            // Safe to delete
            timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                redis::cmd("DEL")
                    .arg(&key)
                    .query_async::<()>(&mut conn),
            )
            .await
            .map_err(|_| Error::Timeout("Redis DEL timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis DEL failed: {e}")))?;
        }

        // Remove from local cache
        let mut nodes = self.local_nodes.write().await;
        nodes.remove(&self.node_id);

        Ok(())
    }

    /// Register a remote node (called by gRPC handler when another node joins)
    pub async fn register_remote(&self, node_info: NodeInfo) -> Result<()> {
        if let Some(ref client) = self.redis_client {
            let mut conn = timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                client.get_multiplexed_async_connection(),
            )
            .await
            .map_err(|_| Error::Timeout("Redis connection timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis connection failed: {e}")))?;

            let key = Self::node_key(&node_info.node_id);
            let value = serde_json::to_string(&node_info)
                .map_err(|e| Error::Serialization(format!("Failed to serialize node info: {e}")))?;

            timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                redis::cmd("SETEX")
                    .arg(&key)
                    .arg(self.heartbeat_timeout_secs * 2)
                    .arg(&value)
                    .query_async::<()>(&mut conn),
            )
            .await
            .map_err(|_| Error::Timeout("Redis SETEX timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis SETEX failed: {e}")))?;
        }

        let mut nodes = self.local_nodes.write().await;
        nodes.insert(node_info.node_id.clone(), node_info);

        Ok(())
    }

    /// Update heartbeat for a remote node (atomic via Lua script)
    pub async fn heartbeat_remote(&self, node_id: &str) -> Result<()> {
        if let Some(ref client) = self.redis_client {
            let mut conn = timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                client.get_multiplexed_async_connection(),
            )
            .await
            .map_err(|_| Error::Timeout("Redis connection timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis connection failed: {e}")))?;

            let key = Self::node_key(node_id);
            let now = Utc::now().to_rfc3339();
            let ttl = self.heartbeat_timeout_secs * 2;

            // Atomic Lua: read → update last_heartbeat → write back with fresh TTL
            let script = redis::Script::new(
                r#"
                local val = redis.call('GET', KEYS[1])
                if not val then return nil end
                local obj = cjson.decode(val)
                obj['last_heartbeat'] = ARGV[1]
                local updated = cjson.encode(obj)
                redis.call('SETEX', KEYS[1], ARGV[2], updated)
                return updated
                "#,
            );

            let result: Option<String> = timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                script.key(&key).arg(&now).arg(ttl).invoke_async(&mut conn),
            )
            .await
            .map_err(|_| Error::Timeout("Redis heartbeat script timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis heartbeat script failed: {e}")))?;

            // Update local cache from the returned value
            if let Some(updated_json) = result {
                if let Ok(node_info) = serde_json::from_str::<NodeInfo>(&updated_json) {
                    let mut nodes = self.local_nodes.write().await;
                    nodes.insert(node_id.to_string(), node_info);
                }
            }
        } else {
            // Local-only mode: update local cache
            let mut nodes = self.local_nodes.write().await;
            if let Some(node) = nodes.get_mut(node_id) {
                node.last_heartbeat = Utc::now();
            }
        }

        Ok(())
    }

    /// Unregister a remote node
    pub async fn unregister_remote(&self, node_id: &str) -> Result<()> {
        if let Some(ref client) = self.redis_client {
            let mut conn = timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                client.get_multiplexed_async_connection(),
            )
            .await
            .map_err(|_| Error::Timeout("Redis connection timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis connection failed: {e}")))?;

            let key = Self::node_key(node_id);

            timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                redis::cmd("DEL")
                    .arg(&key)
                    .query_async::<()>(&mut conn),
            )
            .await
            .map_err(|_| Error::Timeout("Redis DEL timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis DEL failed: {e}")))?;
        }

        let mut nodes = self.local_nodes.write().await;
        nodes.remove(node_id);

        Ok(())
    }

    /// Get all active nodes
    pub async fn get_all_nodes(&self) -> Result<Vec<NodeInfo>> {
        if let Some(ref client) = self.redis_client {
            let mut conn = timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                client.get_multiplexed_async_connection(),
            )
            .await
            .map_err(|_| Error::Timeout("Redis connection timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis connection failed: {e}")))?;

            // Use SCAN instead of KEYS for better performance on large datasets
            // SCAN is non-blocking and returns results incrementally
            let pattern = format!("{}:*", Self::KEY_PREFIX);
            let mut keys = Vec::new();
            let mut cursor: u64 = 0;

            loop {
                let scan_result: (u64, Vec<String>) = timeout(
                    Duration::from_secs(REDIS_TIMEOUT_SECS),
                    redis::cmd("SCAN")
                        .arg(cursor)
                        .arg("MATCH")
                        .arg(&pattern)
                        .arg("COUNT")
                        .arg(100) // Scan 100 keys at a time
                        .query_async(&mut conn),
                )
                .await
                .map_err(|_| Error::Timeout("Redis SCAN timed out".to_string()))?
                .map_err(|e| Error::Database(format!("Redis SCAN failed: {e}")))?;

                cursor = scan_result.0;
                keys.extend(scan_result.1);

                // cursor 0 means iteration complete
                if cursor == 0 {
                    break;
                }
            }

            let mut nodes = Vec::new();
            for key in keys {
                let value: Option<String> = timeout(
                    Duration::from_secs(REDIS_TIMEOUT_SECS),
                    redis::cmd("GET")
                        .arg(&key)
                        .query_async(&mut conn),
                )
                .await
                .map_err(|_| Error::Timeout("Redis GET timed out".to_string()))?
                .map_err(|e| Error::Database(format!("Redis GET failed: {e}")))?;

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
            let mut conn = timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                client.get_multiplexed_async_connection(),
            )
            .await
            .map_err(|_| Error::Timeout("Redis connection timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis connection failed: {e}")))?;

            let key = Self::node_key(node_id);
            let value: Option<String> = timeout(
                Duration::from_secs(REDIS_TIMEOUT_SECS),
                redis::cmd("GET")
                    .arg(&key)
                    .query_async(&mut conn),
            )
            .await
            .map_err(|_| Error::Timeout("Redis GET timed out".to_string()))?
            .map_err(|e| Error::Database(format!("Redis GET failed: {e}")))?;

            if let Some(value) = value {
                let node_info: NodeInfo = serde_json::from_str(&value)
                    .map_err(|e| Error::Serialization(format!("Failed to deserialize node info: {e}")))?;

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

    /// Update metadata for this node in the local cache
    ///
    /// This should be called periodically by the heartbeat loop to include
    /// connection counts and other metrics. The metadata will be persisted
    /// to Redis on the next heartbeat.
    pub async fn update_local_metadata(&self, key: &str, value: String) {
        let mut nodes = self.local_nodes.write().await;
        if let Some(node) = nodes.get_mut(&self.node_id) {
            node.metadata.insert(key.to_string(), value);
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
    use chrono::Duration;

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

    #[test]
    fn test_node_info_epoch_initialization() {
        let node = NodeInfo::new(
            "test".to_string(),
            "localhost:50051".to_string(),
            "localhost:8080".to_string(),
        );

        // New nodes should start with epoch 1
        assert_eq!(node.epoch, 1);
    }

    #[test]
    fn test_node_info_with_epoch() {
        let node = NodeInfo::new(
            "test".to_string(),
            "localhost:50051".to_string(),
            "localhost:8080".to_string(),
        ).with_epoch(5);

        assert_eq!(node.epoch, 5);
    }

    #[test]
    fn test_fencing_token_new() {
        let token = FencingToken::new("node1".to_string(), 3);
        assert_eq!(token.node_id, "node1");
        assert_eq!(token.epoch, 3);
    }

    #[test]
    fn test_fencing_token_is_newer_than() {
        let token1 = FencingToken::new("node1".to_string(), 3);
        let token2 = FencingToken::new("node1".to_string(), 5);
        let token3 = FencingToken::new("node2".to_string(), 5);

        // Same node, higher epoch is newer
        assert!(token2.is_newer_than(&token1));
        assert!(!token1.is_newer_than(&token2));

        // Different nodes - not newer even with higher epoch
        assert!(!token3.is_newer_than(&token1));

        // Same token is not newer than itself
        assert!(!token1.is_newer_than(&token1));
    }

    #[test]
    fn test_node_info_fencing_token() {
        let node = NodeInfo::new(
            "test_node".to_string(),
            "localhost:50051".to_string(),
            "localhost:8080".to_string(),
        ).with_epoch(10);

        let token = node.fencing_token();
        assert_eq!(token.node_id, "test_node");
        assert_eq!(token.epoch, 10);
    }

    #[tokio::test]
    async fn test_node_registry_local_mode() {
        let registry = NodeRegistry::new(None, "test_node".to_string(), 30).unwrap();

        // Get fencing token
        let token = registry.current_fencing_token();
        assert_eq!(token.node_id, "test_node");
        assert_eq!(token.epoch, 1);

        // Register in local mode
        registry
            .register("localhost:50051".to_string(), "localhost:8080".to_string())
            .await
            .unwrap();

        // Check local cache
        let nodes = registry.local_nodes.read().await;
        assert!(nodes.contains_key("test_node"));
    }

    #[test]
    fn test_fencing_token_serialization() {
        let token = FencingToken::new("node1".to_string(), 42);

        // Serialize to JSON
        let json = serde_json::to_string(&token).unwrap();
        assert!(json.contains("node1"));
        assert!(json.contains("42"));

        // Deserialize back
        let deserialized: FencingToken = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.node_id, "node1");
        assert_eq!(deserialized.epoch, 42);
    }

    #[test]
    fn test_node_info_serialization_with_epoch() {
        let node = NodeInfo::new(
            "test".to_string(),
            "localhost:50051".to_string(),
            "localhost:8080".to_string(),
        )
        .with_epoch(7);

        // Serialize to JSON
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains("\"epoch\":7"));

        // Deserialize back
        let deserialized: NodeInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.epoch, 7);
    }
}
