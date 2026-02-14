//! Cluster gRPC server implementation
//!
//! Handles inter-node communication for cluster coordination.

use std::sync::Arc;
use tonic::{Request, Response, Status};

use super::synctv::cluster::cluster_service_server::ClusterService;
use super::synctv::cluster::{NodeInfo, RegisterNodeRequest, RegisterNodeResponse, HeartbeatRequest, HeartbeatResponse, GetNodesRequest, GetNodesResponse, DeregisterNodeRequest, DeregisterNodeResponse, SyncRoomStateRequest, SyncRoomStateResponse, BroadcastEventRequest, BroadcastEventResponse, GetUserOnlineStatusRequest, GetUserOnlineStatusResponse, UserOnlineStatus, GetRoomConnectionsRequest, GetRoomConnectionsResponse, RoomConnection};
use crate::discovery::{NodeInfo as DiscoveryNodeInfo, NodeRegistry};
use crate::sync::connection_manager::ConnectionManager;

/// Cluster gRPC service
///
/// Handles node registration, heartbeats, and state synchronization.
#[derive(Clone)]
pub struct ClusterServer {
    node_registry: Arc<NodeRegistry>,
    connection_manager: Option<Arc<ConnectionManager>>,
    node_id: String,
}

impl ClusterServer {
    /// Create a new cluster server
    #[must_use]
    pub fn new(node_registry: Arc<NodeRegistry>, node_id: String) -> Self {
        Self {
            node_registry,
            connection_manager: None,
            node_id,
        }
    }

    /// Set the connection manager for user/room connection queries
    #[must_use]
    pub fn with_connection_manager(mut self, cm: Arc<ConnectionManager>) -> Self {
        self.connection_manager = Some(cm);
        self
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
            epoch: 1, // Start with epoch 1 for remote nodes
        };

        // Register the remote node in Redis
        if let Err(e) = self.node_registry.register_remote(node_info.clone()).await {
            tracing::error!(
                node_id = %req.node_id,
                error = %e,
                "Failed to register node in cluster"
            );
            return Err(Status::internal(format!("Failed to register node: {e}")));
        }

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

        // Update heartbeat in registry (refreshes TTL and last_heartbeat in Redis)
        if let Err(e) = self.node_registry.heartbeat_remote(&req.node_id).await {
            tracing::warn!(
                node_id = %req.node_id,
                error = %e,
                "Failed to update heartbeat"
            );
            return Err(Status::internal(format!("Failed to update heartbeat: {e}")));
        }

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

        // Remove the node from Redis registry
        if let Err(e) = self.node_registry.unregister_remote(&req.node_id).await {
            tracing::warn!(
                node_id = %req.node_id,
                error = %e,
                "Failed to deregister node from cluster"
            );
            // Don't fail the response — best-effort cleanup, TTL will expire anyway
        }

        tracing::info!(
            node_id = %req.node_id,
            reason = %req.reason,
            "Node deregistered from cluster"
        );

        Ok(Response::new(DeregisterNodeResponse { success: true }))
    }

    /// Synchronize room state between nodes
    ///
    /// Not implemented - room state synchronization is handled via Redis Pub/Sub
    /// in the sync module.
    async fn sync_room_state(
        &self,
        _request: Request<SyncRoomStateRequest>,
    ) -> std::result::Result<Response<SyncRoomStateResponse>, Status> {
        Err(Status::unimplemented(
            "sync_room_state is not implemented; use Redis Pub/Sub for real-time sync",
        ))
    }

    /// Broadcast an event to all nodes
    ///
    /// Not implemented - events are broadcast via Redis Pub/Sub through ClusterManager.
    async fn broadcast_event(
        &self,
        _request: Request<BroadcastEventRequest>,
    ) -> std::result::Result<Response<BroadcastEventResponse>, Status> {
        Err(Status::unimplemented(
            "broadcast_event is not implemented; use ClusterManager for event broadcasting",
        ))
    }

    /// Get online status of users on this node
    ///
    /// Returns the online status for requested users based on this node's
    /// ConnectionManager. In a multi-replica setup, the caller should fan out
    /// this query to all nodes to get the global picture.
    async fn get_user_online_status(
        &self,
        request: Request<GetUserOnlineStatusRequest>,
    ) -> std::result::Result<Response<GetUserOnlineStatusResponse>, Status> {
        let req = request.into_inner();

        let Some(ref cm) = self.connection_manager else {
            return Ok(Response::new(GetUserOnlineStatusResponse {
                statuses: Vec::new(),
            }));
        };

        let statuses: Vec<UserOnlineStatus> = req
            .user_ids
            .iter()
            .map(|uid| {
                let user_id = synctv_core::models::UserId::from_string(uid.clone());
                let connections = cm.get_user_connections(&user_id);
                let is_online = !connections.is_empty();
                let room_ids: Vec<String> = connections
                    .iter()
                    .filter_map(|c| c.room_id.as_ref().map(|r| r.as_str().to_string()))
                    .collect();

                UserOnlineStatus {
                    user_id: uid.clone(),
                    is_online,
                    room_ids,
                    node_id: self.node_id.clone(),
                }
            })
            .collect();

        Ok(Response::new(GetUserOnlineStatusResponse { statuses }))
    }

    /// Get connections for a room on this node
    ///
    /// Returns the active connections in a specific room based on this node's
    /// ConnectionManager. In a multi-replica setup, the caller should fan out
    /// this query to all nodes to get the global room connections.
    async fn get_room_connections(
        &self,
        request: Request<GetRoomConnectionsRequest>,
    ) -> std::result::Result<Response<GetRoomConnectionsResponse>, Status> {
        let req = request.into_inner();

        let Some(ref cm) = self.connection_manager else {
            return Ok(Response::new(GetRoomConnectionsResponse {
                connections: Vec::new(),
            }));
        };

        let room_id = synctv_core::models::RoomId::from_string(req.room_id);
        let room_conns = cm.get_room_connections(&room_id);

        let connections: Vec<RoomConnection> = room_conns
            .iter()
            .map(|conn| {
                // Convert Instant durations to Unix timestamps (approximate)
                let now_unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                let connected_secs_ago = conn.connected_at.elapsed().as_secs() as i64;
                let last_activity_secs_ago = conn.last_activity.elapsed().as_secs() as i64;

                RoomConnection {
                    user_id: conn.user_id.as_str().to_string(),
                    node_id: self.node_id.clone(),
                    connected_at: now_unix - connected_secs_ago,
                    last_activity: now_unix - last_activity_secs_ago,
                }
            })
            .collect();

        Ok(Response::new(GetRoomConnectionsResponse { connections }))
    }
}
