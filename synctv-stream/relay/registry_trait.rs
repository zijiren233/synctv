// StreamRegistry trait for abstraction and testing
//
// This trait allows mocking StreamRegistry in tests without requiring Redis

use async_trait::async_trait;
use anyhow::Result;
use super::registry::{PublisherInfo, StreamRegistry};

/// `StreamRegistry` trait for publisher registration
#[async_trait]
pub trait StreamRegistryTrait: Send + Sync {
    /// Register a publisher for a media in a room (atomic operation)
    /// Returns true if registered successfully, false if already exists
    async fn register_publisher(
        &mut self,
        room_id: &str,
        media_id: &str,
        node_id: &str,
        app_name: &str,
    ) -> Result<bool>;

    /// Try to register as publisher (atomic operation)
    /// Returns true if registered successfully, false if already exists
    async fn try_register_publisher(
        &self,
        room_id: &str,
        media_id: &str,
        node_id: &str,
    ) -> Result<bool>;

    /// Refresh TTL for a publisher (called by heartbeat)
    async fn refresh_publisher_ttl(&self, room_id: &str, media_id: &str) -> Result<()>;

    /// Unregister a publisher
    async fn unregister_publisher(&self, room_id: &str, media_id: &str) -> Result<()>;

    /// Get publisher info for a media in a room
    async fn get_publisher(&self, room_id: &str, media_id: &str) -> Result<Option<PublisherInfo>>;

    /// Check if a stream is active (has a publisher)
    async fn is_stream_active(&self, room_id: &str, media_id: &str) -> Result<bool>;

    /// List all active streams (returns tuples of (`room_id`, `media_id`))
    async fn list_active_streams(&self) -> Result<Vec<(String, String)>>;
}

// Implement StreamRegistryTrait for StreamRegistry
#[async_trait]
impl StreamRegistryTrait for StreamRegistry {
    async fn register_publisher(
        &mut self,
        room_id: &str,
        media_id: &str,
        node_id: &str,
        app_name: &str,
    ) -> Result<bool> {
        Self::register_publisher(self, room_id, media_id, node_id, app_name).await
    }

    async fn try_register_publisher(
        &self,
        room_id: &str,
        media_id: &str,
        node_id: &str,
    ) -> Result<bool> {
        Self::try_register_publisher(self, room_id, media_id, node_id).await
    }

    async fn refresh_publisher_ttl(&self, room_id: &str, media_id: &str) -> Result<()> {
        Self::refresh_publisher_ttl(self, room_id, media_id).await
    }

    async fn unregister_publisher(&self, room_id: &str, media_id: &str) -> Result<()> {
        Self::unregister_publisher_immut(self, room_id, media_id).await
    }

    async fn get_publisher(&self, room_id: &str, media_id: &str) -> Result<Option<PublisherInfo>> {
        Self::get_publisher_immut(self, room_id, media_id).await
    }

    async fn is_stream_active(&self, room_id: &str, media_id: &str) -> Result<bool> {
        Self::is_stream_active_immut(self, room_id, media_id).await
    }

    async fn list_active_streams(&self) -> Result<Vec<(String, String)>> {
        Self::list_active_streams_immut(self).await
    }
}

/// Mock StreamRegistry for testing without Redis
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct MockStreamRegistry {
    publishers: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<(String, String), PublisherInfo>>>,
}

#[cfg(test)]
impl MockStreamRegistry {
    pub fn new() -> Self {
        Self {
            publishers: std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub fn with_publishers(publishers: std::collections::HashMap<(String, String), PublisherInfo>) -> Self {
        Self {
            publishers: std::sync::Arc::new(tokio::sync::Mutex::new(publishers)),
        }
    }
}

#[cfg(test)]
impl Default for MockStreamRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[async_trait]
impl StreamRegistryTrait for MockStreamRegistry {
    async fn register_publisher(
        &mut self,
        room_id: &str,
        media_id: &str,
        node_id: &str,
        app_name: &str,
    ) -> Result<bool> {
        let mut publishers = self.publishers.lock().await;
        let key = (room_id.to_string(), media_id.to_string());

        if publishers.contains_key(&key) {
            Ok(false)
        } else {
            publishers.insert(key, PublisherInfo {
                node_id: node_id.to_string(),
                app_name: app_name.to_string(),
                started_at: Utc::now(),
            });
            Ok(true)
        }
    }

    async fn try_register_publisher(
        &self,
        room_id: &str,
        media_id: &str,
        node_id: &str,
    ) -> Result<bool> {
        let mut publishers = self.publishers.lock().await;
        let key = (room_id.to_string(), media_id.to_string());

        if publishers.contains_key(&key) {
            Ok(false)
        } else {
            publishers.insert(key, PublisherInfo {
                node_id: node_id.to_string(),
                app_name: "live".to_string(),
                started_at: Utc::now(),
            });
            Ok(true)
        }
    }

    async fn refresh_publisher_ttl(&self, _room_id: &str, _media_id: &str) -> Result<()> {
        // No-op for mock
        Ok(())
    }

    async fn unregister_publisher(&self, room_id: &str, media_id: &str) -> Result<()> {
        let mut publishers = self.publishers.lock().await;
        publishers.remove(&(room_id.to_string(), media_id.to_string()));
        Ok(())
    }

    async fn get_publisher(&self, room_id: &str, media_id: &str) -> Result<Option<PublisherInfo>> {
        let publishers = self.publishers.lock().await;
        Ok(publishers.get(&(room_id.to_string(), media_id.to_string())).cloned())
    }

    async fn is_stream_active(&self, room_id: &str, media_id: &str) -> Result<bool> {
        let publishers = self.publishers.lock().await;
        Ok(publishers.contains_key(&(room_id.to_string(), media_id.to_string())))
    }

    async fn list_active_streams(&self) -> Result<Vec<(String, String)>> {
        let publishers = self.publishers.lock().await;
        Ok(publishers.keys().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit tests using MockStreamRegistry (no Redis required)
    #[tokio::test]
    async fn test_mock_register_publisher_success() {
        let mut registry = MockStreamRegistry::new();

        // First registration should succeed
        let registered = registry
            .register_publisher("room123", "media456", "node1", "live")
            .await
            .unwrap();
        assert!(registered);

        // Verify publisher exists
        let publisher = registry.get_publisher("room123", "media456").await.unwrap();
        assert!(publisher.is_some());

        let pub_info = publisher.unwrap();
        assert_eq!(pub_info.node_id, "node1");
        assert_eq!(pub_info.app_name, "live");
    }

    #[tokio::test]
    async fn test_mock_register_publisher_duplicate() {
        let mut registry = MockStreamRegistry::new();

        // First registration should succeed
        let registered = registry
            .register_publisher("room123", "media456", "node1", "live")
            .await
            .unwrap();
        assert!(registered);

        // Second registration should fail (already exists)
        let registered = registry
            .register_publisher("room123", "media456", "node2", "live")
            .await
            .unwrap();
        assert!(!registered);
    }

    #[tokio::test]
    async fn test_mock_try_register_publisher() {
        let registry = MockStreamRegistry::new();

        // First try_register should succeed
        let result = registry.try_register_publisher("room123", "media456", "node1").await.unwrap();
        assert!(result);

        // Second try_register should return false (already exists)
        let result = registry.try_register_publisher("room123", "media456", "node2").await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_mock_unregister_publisher() {
        let mut registry = MockStreamRegistry::new();

        // Register publisher
        registry
            .register_publisher("room123", "media456", "node1", "live")
            .await
            .unwrap();

        // Verify exists
        assert!(registry.is_stream_active("room123", "media456").await.unwrap());

        // Unregister
        registry.unregister_publisher("room123", "media456").await.unwrap();

        // Verify removed
        assert!(!registry.is_stream_active("room123", "media456").await.unwrap());
    }

    #[tokio::test]
    async fn test_mock_get_publisher_not_found() {
        let registry = MockStreamRegistry::new();

        // Non-existent publisher should return None
        let result = registry.get_publisher("nonexistent", "media").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_mock_list_active_streams() {
        let mut registry = MockStreamRegistry::new();

        // Register multiple publishers
        registry
            .register_publisher("room1", "media1", "node1", "live")
            .await
            .unwrap();
        registry
            .register_publisher("room2", "media2", "node1", "live")
            .await
            .unwrap();

        // List active streams
        let streams = registry.list_active_streams().await.unwrap();
        assert_eq!(streams.len(), 2);
        assert!(streams.contains(&(String::from("room1"), String::from("media1"))));
        assert!(streams.contains(&(String::from("room2"), String::from("media2"))));
    }

    #[tokio::test]
    async fn test_mock_pre_initialized() {
        let mut publishers = std::collections::HashMap::new();
        publishers.insert(
            ("room1".to_string(), "media1".to_string()),
            PublisherInfo {
                node_id: "node1".to_string(),
                app_name: "live".to_string(),
                started_at: Utc::now(),
            }
        );

        let registry = MockStreamRegistry::with_publishers(publishers);

        // Should find the pre-registered publisher
        let result = registry.get_publisher("room1", "media1").await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().node_id, "node1");
    }
}
