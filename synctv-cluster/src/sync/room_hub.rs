use dashmap::DashMap;
use std::sync::Arc;
use synctv_core::models::id::{RoomId, UserId};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::events::ClusterEvent;

/// Handle for a client connection subscription
pub type ConnectionId = String;

/// Message sender for a client connection
pub type MessageSender = mpsc::UnboundedSender<ClusterEvent>;

/// Subscriber information
#[derive(Debug, Clone)]
pub struct Subscriber {
    pub connection_id: ConnectionId,
    pub user_id: UserId,
    pub sender: MessageSender,
}

/// In-memory hub for routing messages to connected clients in rooms
/// This handles local message distribution (single node)
#[derive(Clone)]
pub struct RoomMessageHub {
    /// Map of room_id -> list of subscribers
    rooms: Arc<DashMap<RoomId, Vec<Subscriber>>>,

    /// Map of connection_id -> (room_id, user_id) for cleanup
    connections: Arc<DashMap<ConnectionId, (RoomId, UserId)>>,
}

impl RoomMessageHub {
    /// Create a new RoomMessageHub
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(DashMap::new()),
            connections: Arc::new(DashMap::new()),
        }
    }

    /// Subscribe a client to room events
    /// Returns a receiver for messages
    pub fn subscribe(
        &self,
        room_id: RoomId,
        user_id: UserId,
        connection_id: ConnectionId,
    ) -> mpsc::UnboundedReceiver<ClusterEvent> {
        let (tx, rx) = mpsc::unbounded_channel();

        let subscriber = Subscriber {
            connection_id: connection_id.clone(),
            user_id: user_id.clone(),
            sender: tx,
        };

        // Add to room subscribers
        self.rooms
            .entry(room_id.clone())
            .or_insert_with(Vec::new)
            .push(subscriber);

        // Track connection for cleanup
        self.connections
            .insert(connection_id.clone(), (room_id.clone(), user_id.clone()));

        info!(
            room_id = %room_id.as_str(),
            user_id = %user_id.as_str(),
            connection_id = %connection_id,
            "Client subscribed to room"
        );

        rx
    }

    /// Unsubscribe a client from room events
    pub fn unsubscribe(&self, connection_id: &str) {
        if let Some((_, (room_id, user_id))) = self.connections.remove(connection_id) {
            // Remove from room subscribers
            if let Some(mut subscribers) = self.rooms.get_mut(&room_id) {
                subscribers.retain(|sub| sub.connection_id != connection_id);

                // Remove room entry if no more subscribers
                if subscribers.is_empty() {
                    drop(subscribers); // Drop the RefMut before removing
                    self.rooms.remove(&room_id);
                    debug!(room_id = %room_id.as_str(), "Room has no more subscribers, removed");
                }
            }

            info!(
                room_id = %room_id.as_str(),
                user_id = %user_id.as_str(),
                connection_id = %connection_id,
                "Client unsubscribed from room"
            );
        } else {
            warn!(
                connection_id = %connection_id,
                "Attempted to unsubscribe unknown connection"
            );
        }
    }

    /// Broadcast an event to all subscribers in a room
    pub fn broadcast(&self, room_id: &RoomId, event: ClusterEvent) -> usize {
        let mut sent_count = 0;
        let mut failed_connections = Vec::new();

        if let Some(subscribers) = self.rooms.get(room_id) {
            for subscriber in subscribers.iter() {
                match subscriber.sender.send(event.clone()) {
                    Ok(_) => {
                        sent_count += 1;
                        debug!(
                            room_id = %room_id.as_str(),
                            user_id = %subscriber.user_id.as_str(),
                            connection_id = %subscriber.connection_id,
                            event_type = %event.event_type(),
                            "Event sent to client"
                        );
                    }
                    Err(err) => {
                        warn!(
                            room_id = %room_id.as_str(),
                            user_id = %subscriber.user_id.as_str(),
                            connection_id = %subscriber.connection_id,
                            error = %err,
                            "Failed to send event to client, marking for cleanup"
                        );
                        failed_connections.push(subscriber.connection_id.clone());
                    }
                }
            }
        }

        // Clean up failed connections
        for conn_id in failed_connections {
            self.unsubscribe(&conn_id);
        }

        if sent_count > 0 {
            debug!(
                room_id = %room_id.as_str(),
                sent_count = sent_count,
                event_type = %event.event_type(),
                "Event broadcast complete"
            );
        }

        sent_count
    }

    /// Broadcast an event to a specific user in a room
    pub fn broadcast_to_user(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        event: ClusterEvent,
    ) -> usize {
        let mut sent_count = 0;
        let mut failed_connections = Vec::new();

        if let Some(subscribers) = self.rooms.get(room_id) {
            for subscriber in subscribers.iter() {
                if subscriber.user_id == *user_id {
                    match subscriber.sender.send(event.clone()) {
                        Ok(_) => {
                            sent_count += 1;
                            debug!(
                                room_id = %room_id.as_str(),
                                user_id = %subscriber.user_id.as_str(),
                                connection_id = %subscriber.connection_id,
                                event_type = %event.event_type(),
                                "Event sent to specific user"
                            );
                        }
                        Err(err) => {
                            warn!(
                                room_id = %room_id.as_str(),
                                user_id = %subscriber.user_id.as_str(),
                                connection_id = %subscriber.connection_id,
                                error = %err,
                                "Failed to send event to user"
                            );
                            failed_connections.push(subscriber.connection_id.clone());
                        }
                    }
                }
            }
        }

        // Clean up failed connections
        for conn_id in failed_connections {
            self.unsubscribe(&conn_id);
        }

        sent_count
    }

    /// Get the number of subscribers in a room
    pub fn subscriber_count(&self, room_id: &RoomId) -> usize {
        self.rooms
            .get(room_id)
            .map(|subscribers| subscribers.len())
            .unwrap_or(0)
    }

    /// Get the number of active rooms
    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }

    /// Get total number of active connections
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Get all subscribers in a room (for debugging/monitoring)
    pub fn get_room_subscribers(&self, room_id: &RoomId) -> Vec<(UserId, ConnectionId)> {
        self.rooms
            .get(room_id)
            .map(|subscribers| {
                subscribers
                    .iter()
                    .map(|sub| (sub.user_id.clone(), sub.connection_id.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Default for RoomMessageHub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn test_subscribe_and_broadcast() {
        let hub = RoomMessageHub::new();
        let room_id = RoomId::from_string("test_room".to_string());
        let user_id = UserId::from_string("test_user".to_string());

        // Subscribe
        let mut rx = hub.subscribe(room_id.clone(), user_id.clone(), "conn1".to_string());

        assert_eq!(hub.subscriber_count(&room_id), 1);
        assert_eq!(hub.connection_count(), 1);

        // Broadcast event
        let event = ClusterEvent::ChatMessage {
            room_id: room_id.clone(),
            user_id: user_id.clone(),
            username: "testuser".to_string(),
            message: "Hello!".to_string(),
            timestamp: Utc::now(),
        };

        let sent_count = hub.broadcast(&room_id, event.clone());
        assert_eq!(sent_count, 1);

        // Receive event
        let received = rx.recv().await.unwrap();
        assert_eq!(received.event_type(), "chat_message");
    }

    #[tokio::test]
    async fn test_unsubscribe() {
        let hub = RoomMessageHub::new();
        let room_id = RoomId::from_string("test_room".to_string());
        let user_id = UserId::from_string("test_user".to_string());

        // Subscribe
        let _rx = hub.subscribe(room_id.clone(), user_id.clone(), "conn1".to_string());
        assert_eq!(hub.subscriber_count(&room_id), 1);

        // Unsubscribe
        hub.unsubscribe("conn1");
        assert_eq!(hub.subscriber_count(&room_id), 0);
        assert_eq!(hub.connection_count(), 0);
        assert_eq!(hub.room_count(), 0);
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let hub = RoomMessageHub::new();
        let room_id = RoomId::from_string("test_room".to_string());
        let user1 = UserId::from_string("user1".to_string());
        let user2 = UserId::from_string("user2".to_string());

        // Subscribe two clients
        let mut rx1 = hub.subscribe(room_id.clone(), user1.clone(), "conn1".to_string());
        let mut rx2 = hub.subscribe(room_id.clone(), user2.clone(), "conn2".to_string());

        assert_eq!(hub.subscriber_count(&room_id), 2);

        // Broadcast event
        let event = ClusterEvent::ChatMessage {
            room_id: room_id.clone(),
            user_id: user1.clone(),
            username: "user1".to_string(),
            message: "Hello!".to_string(),
            timestamp: Utc::now(),
        };

        let sent_count = hub.broadcast(&room_id, event.clone());
        assert_eq!(sent_count, 2);

        // Both should receive
        let received1 = rx1.recv().await.unwrap();
        let received2 = rx2.recv().await.unwrap();

        assert_eq!(received1.event_type(), "chat_message");
        assert_eq!(received2.event_type(), "chat_message");
    }

    #[tokio::test]
    async fn test_broadcast_to_specific_user() {
        let hub = RoomMessageHub::new();
        let room_id = RoomId::from_string("test_room".to_string());
        let user1 = UserId::from_string("user1".to_string());
        let user2 = UserId::from_string("user2".to_string());

        // Subscribe two clients
        let mut rx1 = hub.subscribe(room_id.clone(), user1.clone(), "conn1".to_string());
        let mut rx2 = hub.subscribe(room_id.clone(), user2.clone(), "conn2".to_string());

        // Broadcast to user1 only
        let event = ClusterEvent::SystemNotification {
            message: "Private message".to_string(),
            level: crate::sync::NotificationLevel::Info,
            timestamp: Utc::now(),
        };

        let sent_count = hub.broadcast_to_user(&room_id, &user1, event.clone());
        assert_eq!(sent_count, 1);

        // Only user1 should receive
        let received1 = tokio::time::timeout(std::time::Duration::from_millis(100), rx1.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(received1.event_type(), "system_notification");

        // User2 should not receive
        let received2 =
            tokio::time::timeout(std::time::Duration::from_millis(100), rx2.recv()).await;

        assert!(
            received2.is_err(),
            "User2 should not have received the message"
        );
    }
}
