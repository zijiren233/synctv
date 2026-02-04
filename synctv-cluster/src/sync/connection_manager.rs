use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use synctv_core::models::id::{RoomId, UserId};
use tracing::{debug, info, warn};

/// Connection information
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub connection_id: String,
    pub user_id: UserId,
    pub room_id: Option<RoomId>,
    pub connected_at: Instant,
    pub last_activity: Instant,
    pub message_count: u64,
    pub rtc_joined: bool,
}

impl ConnectionInfo {
    #[must_use]
    pub fn new(connection_id: String, user_id: UserId) -> Self {
        let now = Instant::now();
        Self {
            connection_id,
            user_id,
            room_id: None,
            connected_at: now,
            last_activity: now,
            message_count: 0,
            rtc_joined: false,
        }
    }

    #[must_use] 
    pub fn duration(&self) -> Duration {
        self.connected_at.elapsed()
    }

    #[must_use] 
    pub fn idle_duration(&self) -> Duration {
        self.last_activity.elapsed()
    }
}

/// Connection limits configuration
#[derive(Debug, Clone)]
pub struct ConnectionLimits {
    /// Maximum connections per user
    pub max_per_user: usize,

    /// Maximum connections per room
    pub max_per_room: usize,

    /// Maximum total connections
    pub max_total: usize,

    /// Idle timeout (disconnect if no activity)
    pub idle_timeout: Duration,

    /// Maximum connection duration
    pub max_duration: Duration,
}

impl Default for ConnectionLimits {
    fn default() -> Self {
        Self {
            max_per_user: 5,
            max_per_room: 200,
            max_total: 10000,
            idle_timeout: Duration::from_secs(300), // 5 minutes
            max_duration: Duration::from_secs(86400), // 24 hours
        }
    }
}

/// Connection manager for tracking active gRPC streaming connections
#[derive(Clone)]
pub struct ConnectionManager {
    /// All active connections by `connection_id`
    connections: Arc<DashMap<String, ConnectionInfo>>,

    /// Connections by `user_id`
    user_connections: Arc<DashMap<UserId, Vec<String>>>,

    /// Connections by `room_id`
    room_connections: Arc<DashMap<RoomId, Vec<String>>>,

    /// Connection limits
    limits: Arc<ConnectionLimits>,

    /// Metrics
    total_connections: Arc<AtomicU64>,
    total_messages: Arc<AtomicU64>,
}

impl ConnectionManager {
    /// Create a new `ConnectionManager`
    #[must_use] 
    pub fn new(limits: ConnectionLimits) -> Self {
        Self {
            connections: Arc::new(DashMap::new()),
            user_connections: Arc::new(DashMap::new()),
            room_connections: Arc::new(DashMap::new()),
            limits: Arc::new(limits),
            total_connections: Arc::new(AtomicU64::new(0)),
            total_messages: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Register a new connection
    ///
    /// Returns Ok(()) if connection is allowed, or Err with reason if rejected
    pub fn register(&self, connection_id: String, user_id: UserId) -> Result<(), String> {
        // Check total limit
        if self.connections.len() >= self.limits.max_total {
            return Err(format!(
                "Server at capacity ({} connections)",
                self.limits.max_total
            ));
        }

        // Check per-user limit
        if let Some(user_conns) = self.user_connections.get(&user_id) {
            if user_conns.len() >= self.limits.max_per_user {
                return Err(format!(
                    "Too many connections for this user (max {})",
                    self.limits.max_per_user
                ));
            }
        }

        // Create connection info
        let conn_info = ConnectionInfo::new(connection_id.clone(), user_id.clone());

        // Register connection
        self.connections.insert(connection_id.clone(), conn_info);

        // Add to user connections
        self.user_connections
            .entry(user_id.clone())
            .or_default()
            .push(connection_id.clone());

        // Update metrics
        self.total_connections.fetch_add(1, Ordering::Relaxed);

        info!(
            connection_id = %connection_id,
            user_id = %user_id.as_str(),
            total_connections = self.connections.len(),
            "Connection registered"
        );

        Ok(())
    }

    /// Associate a connection with a room
    pub fn join_room(&self, connection_id: &str, room_id: RoomId) -> Result<(), String> {
        // Check per-room limit
        if let Some(room_conns) = self.room_connections.get(&room_id) {
            if room_conns.len() >= self.limits.max_per_room {
                return Err(format!(
                    "Room at capacity ({} connections)",
                    self.limits.max_per_room
                ));
            }
        }

        // Update connection info
        if let Some(mut conn) = self.connections.get_mut(connection_id) {
            conn.room_id = Some(room_id.clone());
            conn.last_activity = Instant::now();
        } else {
            return Err("Connection not found".to_string());
        }

        // Add to room connections
        self.room_connections
            .entry(room_id.clone())
            .or_default()
            .push(connection_id.to_string());

        debug!(
            connection_id = %connection_id,
            room_id = %room_id.as_str(),
            "Connection joined room"
        );

        Ok(())
    }

    /// Record message activity for a connection
    pub fn record_message(&self, connection_id: &str) {
        if let Some(mut conn) = self.connections.get_mut(connection_id) {
            conn.last_activity = Instant::now();
            conn.message_count += 1;
        }
        self.total_messages.fetch_add(1, Ordering::Relaxed);
    }

    /// Unregister a connection
    pub fn unregister(&self, connection_id: &str) {
        if let Some((_, conn_info)) = self.connections.remove(connection_id) {
            // Remove from user connections
            if let Some(mut user_conns) = self.user_connections.get_mut(&conn_info.user_id) {
                user_conns.retain(|id| id != connection_id);
                if user_conns.is_empty() {
                    drop(user_conns);
                    self.user_connections.remove(&conn_info.user_id);
                }
            }

            // Remove from room connections
            if let Some(room_id) = &conn_info.room_id {
                if let Some(mut room_conns) = self.room_connections.get_mut(room_id) {
                    room_conns.retain(|id| id != connection_id);
                    if room_conns.is_empty() {
                        drop(room_conns);
                        self.room_connections.remove(room_id);
                    }
                }
            }

            info!(
                connection_id = %connection_id,
                user_id = %conn_info.user_id.as_str(),
                duration = ?conn_info.duration(),
                message_count = conn_info.message_count,
                "Connection unregistered"
            );
        }
    }

    /// Check for idle or expired connections
    ///
    /// Returns list of connection IDs that should be disconnected
    pub fn check_timeouts(&self) -> Vec<String> {
        let mut to_disconnect = Vec::new();

        for entry in self.connections.iter() {
            let conn = entry.value();

            // Check idle timeout
            if conn.idle_duration() > self.limits.idle_timeout {
                warn!(
                    connection_id = %conn.connection_id,
                    idle_duration = ?conn.idle_duration(),
                    "Connection idle timeout"
                );
                to_disconnect.push(conn.connection_id.clone());
                continue;
            }

            // Check max duration
            if conn.duration() > self.limits.max_duration {
                warn!(
                    connection_id = %conn.connection_id,
                    duration = ?conn.duration(),
                    "Connection max duration reached"
                );
                to_disconnect.push(conn.connection_id.clone());
            }
        }

        to_disconnect
    }

    /// Get connection count
    #[must_use] 
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Get connection count for a user
    #[must_use] 
    pub fn user_connection_count(&self, user_id: &UserId) -> usize {
        self.user_connections
            .get(user_id)
            .map_or(0, |conns| conns.len())
    }

    /// Get connection count for a room
    #[must_use] 
    pub fn room_connection_count(&self, room_id: &RoomId) -> usize {
        self.room_connections
            .get(room_id)
            .map_or(0, |conns| conns.len())
    }

    /// Get total connections ever established
    #[must_use] 
    pub fn total_connections(&self) -> u64 {
        self.total_connections.load(Ordering::Relaxed)
    }

    /// Get total messages processed
    #[must_use] 
    pub fn total_messages(&self) -> u64 {
        self.total_messages.load(Ordering::Relaxed)
    }

    /// Get connection info
    #[must_use] 
    pub fn get_connection(&self, connection_id: &str) -> Option<ConnectionInfo> {
        self.connections.get(connection_id).map(|c| c.clone())
    }

    /// Get all connections for a user
    #[must_use] 
    pub fn get_user_connections(&self, user_id: &UserId) -> Vec<ConnectionInfo> {
        if let Some(conn_ids) = self.user_connections.get(user_id) {
            conn_ids
                .iter()
                .filter_map(|id| self.connections.get(id).map(|c| c.clone()))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get all connections in a room
    #[must_use] 
    pub fn get_room_connections(&self, room_id: &RoomId) -> Vec<ConnectionInfo> {
        if let Some(conn_ids) = self.room_connections.get(room_id) {
            conn_ids
                .iter()
                .filter_map(|id| self.connections.get(id).map(|c| c.clone()))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get metrics summary
    #[must_use]
    pub fn metrics(&self) -> ConnectionMetrics {
        ConnectionMetrics {
            active_connections: self.connection_count(),
            total_connections: self.total_connections(),
            total_messages: self.total_messages(),
            active_users: self.user_connections.len(),
            active_rooms: self.room_connections.len(),
        }
    }

    /// Get connection ID for a user in a specific room
    ///
    /// Returns the first active connection ID found for the user in the room.
    /// For WebRTC, this allows us to identify which connection a user is using in a room.
    #[must_use]
    pub fn get_connection_id(&self, room_id: &RoomId, user_id: &UserId) -> Option<String> {
        // Get all connections for this user
        if let Some(conn_ids) = self.user_connections.get(user_id) {
            // Find the first connection that's in the specified room
            for conn_id in conn_ids.iter() {
                if let Some(conn) = self.connections.get(conn_id) {
                    if conn.room_id.as_ref() == Some(room_id) {
                        return Some(conn.connection_id.clone());
                    }
                }
            }
        }
        None
    }

    /// Mark a connection as joined or left WebRTC session
    ///
    /// This is used to track which connections are actively participating in WebRTC calls.
    pub fn mark_rtc_joined(&self, room_id: &RoomId, user_id: &UserId, conn_id: &str, joined: bool) {
        // Verify the connection belongs to the user and room
        if let Some(mut conn) = self.connections.get_mut(conn_id) {
            if &conn.user_id == user_id && conn.room_id.as_ref() == Some(room_id) {
                conn.rtc_joined = joined;
                debug!(
                    connection_id = %conn_id,
                    user_id = %user_id.as_str(),
                    room_id = %room_id.as_str(),
                    joined = joined,
                    "WebRTC join status updated"
                );
            }
        }
    }

    /// Get all connections in a room that have joined WebRTC
    #[must_use]
    pub fn get_rtc_connections(&self, room_id: &RoomId) -> Vec<ConnectionInfo> {
        if let Some(conn_ids) = self.room_connections.get(room_id) {
            conn_ids
                .iter()
                .filter_map(|id| self.connections.get(id).map(|c| c.clone()))
                .filter(|conn| conn.rtc_joined)
                .collect()
        } else {
            Vec::new()
        }
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new(ConnectionLimits::default())
    }
}

/// Connection metrics
#[derive(Debug, Clone)]
pub struct ConnectionMetrics {
    pub active_connections: usize,
    pub total_connections: u64,
    pub total_messages: u64,
    pub active_users: usize,
    pub active_rooms: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_connection() {
        let manager = ConnectionManager::default();
        let user_id = UserId::from_string("user1".to_string());

        let result = manager.register("conn1".to_string(), user_id.clone());
        assert!(result.is_ok());
        assert_eq!(manager.connection_count(), 1);
        assert_eq!(manager.user_connection_count(&user_id), 1);
    }

    #[test]
    fn test_per_user_limit() {
        let limits = ConnectionLimits {
            max_per_user: 2,
            ..Default::default()
        };
        let manager = ConnectionManager::new(limits);
        let user_id = UserId::from_string("user1".to_string());

        // First two should succeed
        assert!(manager
            .register("conn1".to_string(), user_id.clone())
            .is_ok());
        assert!(manager
            .register("conn2".to_string(), user_id.clone())
            .is_ok());

        // Third should fail
        let result = manager.register("conn3".to_string(), user_id.clone());
        assert!(result.is_err());
        assert_eq!(manager.connection_count(), 2);
    }

    #[test]
    fn test_join_room() {
        let manager = ConnectionManager::default();
        let user_id = UserId::from_string("user1".to_string());
        let room_id = RoomId::from_string("room1".to_string());

        manager
            .register("conn1".to_string(), user_id.clone())
            .unwrap();

        let result = manager.join_room("conn1", room_id.clone());
        assert!(result.is_ok());
        assert_eq!(manager.room_connection_count(&room_id), 1);

        let conn = manager.get_connection("conn1").unwrap();
        assert_eq!(conn.room_id.as_ref().unwrap().as_str(), "room1");
    }

    #[test]
    fn test_per_room_limit() {
        let limits = ConnectionLimits {
            max_per_room: 2,
            ..Default::default()
        };
        let manager = ConnectionManager::new(limits);
        let room_id = RoomId::from_string("room1".to_string());

        // Register two connections and join room
        let user1 = UserId::from_string("user1".to_string());
        let user2 = UserId::from_string("user2".to_string());
        let user3 = UserId::from_string("user3".to_string());

        manager.register("conn1".to_string(), user1).unwrap();
        manager.register("conn2".to_string(), user2).unwrap();
        manager.register("conn3".to_string(), user3).unwrap();

        assert!(manager.join_room("conn1", room_id.clone()).is_ok());
        assert!(manager.join_room("conn2", room_id.clone()).is_ok());

        // Third should fail
        let result = manager.join_room("conn3", room_id.clone());
        assert!(result.is_err());
    }

    #[test]
    fn test_record_message() {
        let manager = ConnectionManager::default();
        let user_id = UserId::from_string("user1".to_string());

        manager.register("conn1".to_string(), user_id).unwrap();

        manager.record_message("conn1");
        manager.record_message("conn1");

        let conn = manager.get_connection("conn1").unwrap();
        assert_eq!(conn.message_count, 2);
        assert_eq!(manager.total_messages(), 2);
    }

    #[test]
    fn test_unregister() {
        let manager = ConnectionManager::default();
        let user_id = UserId::from_string("user1".to_string());
        let room_id = RoomId::from_string("room1".to_string());

        manager
            .register("conn1".to_string(), user_id.clone())
            .unwrap();
        manager.join_room("conn1", room_id.clone()).unwrap();

        assert_eq!(manager.connection_count(), 1);
        assert_eq!(manager.user_connection_count(&user_id), 1);
        assert_eq!(manager.room_connection_count(&room_id), 1);

        manager.unregister("conn1");

        assert_eq!(manager.connection_count(), 0);
        assert_eq!(manager.user_connection_count(&user_id), 0);
        assert_eq!(manager.room_connection_count(&room_id), 0);
    }

    #[test]
    fn test_metrics() {
        let manager = ConnectionManager::default();
        let user1 = UserId::from_string("user1".to_string());
        let user2 = UserId::from_string("user2".to_string());

        manager.register("conn1".to_string(), user1).unwrap();
        manager.register("conn2".to_string(), user2).unwrap();

        manager.record_message("conn1");
        manager.record_message("conn2");

        let metrics = manager.metrics();
        assert_eq!(metrics.active_connections, 2);
        assert_eq!(metrics.total_connections, 2);
        assert_eq!(metrics.total_messages, 2);
        assert_eq!(metrics.active_users, 2);
    }

    #[tokio::test]
    async fn test_idle_timeout() {
        let limits = ConnectionLimits {
            idle_timeout: Duration::from_millis(100),
            ..Default::default()
        };
        let manager = ConnectionManager::new(limits);
        let user_id = UserId::from_string("user1".to_string());

        manager.register("conn1".to_string(), user_id).unwrap();

        // Wait for idle timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        let timeouts = manager.check_timeouts();
        assert_eq!(timeouts.len(), 1);
        assert_eq!(timeouts[0], "conn1");
    }
}
