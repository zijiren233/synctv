//! Cluster gRPC server implementation
//!
//! Handles inter-node communication for cluster coordination.

use std::sync::Arc;
use tonic::{Request, Response, Status};

use super::synctv::cluster::cluster_service_server::ClusterService;
use super::synctv::cluster::{NodeInfo, RegisterNodeRequest, RegisterNodeResponse, HeartbeatRequest, HeartbeatResponse, GetNodesRequest, GetNodesResponse, DeregisterNodeRequest, DeregisterNodeResponse, SyncRoomStateRequest, SyncRoomStateResponse, BroadcastEventRequest, BroadcastEventResponse, GetUserOnlineStatusRequest, GetUserOnlineStatusResponse, GetRoomConnectionsRequest, GetRoomConnectionsResponse};
use crate::discovery::{NodeInfo as DiscoveryNodeInfo, NodeRegistry};

/// Cluster gRPC service
///
/// Handles node registration, heartbeats, and state synchronization.
#[derive(Clone)]
pub struct ClusterServer {
    node_registry: Arc<NodeRegistry>,
}

impl ClusterServer {
    /// Create a new cluster server
    #[must_use] 
    pub fn new(node_registry: Arc<NodeRegistry>, _node_id: String) -> Self {
        Self {
            node_registry,
        }
    }

    /// Convert discovery `NodeInfo` to proto `NodeInfo`
    fn discovery_to_proto_node(&self, discovery: &DiscoveryNodeInfo) -> NodeInfo {
        NodeInfo {
            node_id: discovery.node_id.clone(),
            address: discovery.grpc_address.clone(),
            region: String::new(),
            status: 1, // Active
            registered_at: chrono::Utc::now().timestamp(),
            last_heartbeat: discovery.last_heartbeat.timestamp(),
            metrics: None,
        }
    }
}

#[tonic::async_trait]
impl ClusterService for ClusterServer {
    /// Register a new node in the cluster
    async fn register_node(
        &self,
        request: Request<RegisterNodeRequest>,
    ) -> std::result::Result<Response<RegisterNodeResponse>, Status> {
        let req = request.into_inner();

        // Create node info
        let node_info = DiscoveryNodeInfo {
            node_id: req.node_id.clone(),
            grpc_address: req.address.clone(),
            http_address: String::new(),
            last_heartbeat: chrono::Utc::now(),
            metadata: std::collections::HashMap::new(),
        };

        // Store in Redis via registry (simulated - we'd update the registry directly)
        tracing::info!(
            node_id = %req.node_id,
            address = %req.address,
            "Node registered in cluster"
        );

        // Get peer nodes
        let peers = match self.node_registry.get_all_nodes().await {
            Ok(nodes) => nodes
                .into_iter()
                .filter(|n| n.node_id != req.node_id)
                .map(|n| self.discovery_to_proto_node(&n))
                .collect(),
            Err(e) => {
                tracing::warn!("Failed to get peer nodes: {}", e);
                Vec::new()
            }
        };

        Ok(Response::new(RegisterNodeResponse {
            node: Some(self.discovery_to_proto_node(&node_info)),
            peers,
        }))
    }

    /// Handle heartbeat from a node
    async fn heartbeat(
        &self,
        request: Request<HeartbeatRequest>,
    ) -> std::result::Result<Response<HeartbeatResponse>, Status> {
        let req = request.into_inner();

        // Update heartbeat in registry
        // In production, we'd update metrics here
        tracing::trace!(
            node_id = %req.node_id,
            "Heartbeat received"
        );

        Ok(Response::new(HeartbeatResponse {
            success: true,
            timestamp: chrono::Utc::now().timestamp(),
        }))
    }

    /// Get all nodes in the cluster
    async fn get_nodes(
        &self,
        _request: Request<GetNodesRequest>,
    ) -> std::result::Result<Response<GetNodesResponse>, Status> {
        match self.node_registry.get_all_nodes().await {
            Ok(nodes) => {
                let proto_nodes: Vec<NodeInfo> = nodes
                    .iter()
                    .map(|n| self.discovery_to_proto_node(n))
                    .collect();

                Ok(Response::new(GetNodesResponse { nodes: proto_nodes }))
            }
            Err(e) => {
                tracing::error!("Failed to get nodes: {}", e);
                Ok(Response::new(GetNodesResponse { nodes: Vec::new() }))
            }
        }
    }

    /// Deregister a node from the cluster
    async fn deregister_node(
        &self,
        request: Request<DeregisterNodeRequest>,
    ) -> std::result::Result<Response<DeregisterNodeResponse>, Status> {
        let req = request.into_inner();

        tracing::info!(
            node_id = %req.node_id,
            reason = %req.reason,
            "Node deregistered from cluster"
        );

        // In production, we'd clean up the node from registry here
        // For now, let Redis TTL handle expiration

        Ok(Response::new(DeregisterNodeResponse { success: true }))
    }

    /// Synchronize room state between nodes
    async fn sync_room_state(
        &self,
        _request: Request<SyncRoomStateRequest>,
    ) -> std::result::Result<Response<SyncRoomStateResponse>, Status> {
        // Room state synchronization is handled via Redis Pub/Sub in the sync module
        // This is a placeholder for future direct state sync

        Ok(Response::new(SyncRoomStateResponse { state: None }))
    }

    /// Broadcast an event to all nodes
    async fn broadcast_event(
        &self,
        _request: Request<BroadcastEventRequest>,
    ) -> std::result::Result<Response<BroadcastEventResponse>, Status> {
        // Events are broadcast via Redis Pub/Sub
        Ok(Response::new(BroadcastEventResponse {
            success: true,
            nodes_reached: 0,
        }))
    }

    /// Get online status of users
    async fn get_user_online_status(
        &self,
        _request: Request<GetUserOnlineStatusRequest>,
    ) -> std::result::Result<Response<GetUserOnlineStatusResponse>, Status> {
        // User tracking is handled via sync module
        Ok(Response::new(GetUserOnlineStatusResponse {
            statuses: Vec::new(),
        }))
    }

    /// Get connection count for rooms
    async fn get_room_connections(
        &self,
        _request: Request<GetRoomConnectionsRequest>,
    ) -> std::result::Result<Response<GetRoomConnectionsResponse>, Status> {
        // Room connections are tracked via sync module
        let response = GetRoomConnectionsResponse {
            connections: Vec::new(),
        };
        Ok(Response::new(response))
    }
}
