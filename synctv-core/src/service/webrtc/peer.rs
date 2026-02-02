//! WebRTC peer management
//!
//! Manages individual peer connections in a WebRTC session.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use super::{SessionDescription, IceCandidate, MediaType};

use crate::{models::UserId, Error, Result};

/// Peer connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerConnectionState {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}

/// Peer state within a session
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerState {
    /// Peer is joining the session
    Joining,
    /// Peer is active in the session
    Active,
    /// Peer is muted
    Muted,
    /// Peer has video disabled
    VideoOff,
    /// Peer is leaving the session
    Leaving,
    /// Peer has left the session
    Left,
}

/// WebRTC peer (participant in a call)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    /// Unique peer ID
    pub id: String,
    /// User ID
    pub user_id: UserId,
    /// Username
    pub username: String,
    /// Connection state
    pub connection_state: PeerConnectionState,
    /// Peer state within the session
    pub state: PeerState,
    /// Media type (audio, video, or both)
    pub media_type: MediaType,
    /// Whether audio is enabled
    pub audio_enabled: bool,
    /// Whether video is enabled
    pub video_enabled: bool,
    /// Local session description
    pub local_description: Option<SessionDescription>,
    /// Remote session description
    pub remote_description: Option<SessionDescription>,
    /// ICE candidates gathered so far
    pub ice_candidates: Vec<IceCandidate>,
    /// Timestamp when peer joined
    pub joined_at: chrono::DateTime<chrono::Utc>,
    /// Timestamp of last activity
    pub last_activity: chrono::DateTime<chrono::Utc>,
}

impl Peer {
    /// Create a new peer
    pub fn new(user_id: UserId, username: String, media_type: MediaType) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: nanoid::nanoid!(12),
            user_id,
            username,
            connection_state: PeerConnectionState::New,
            state: PeerState::Joining,
            media_type,
            audio_enabled: media_type == MediaType::Audio || media_type == MediaType::AudioVideo,
            video_enabled: media_type == MediaType::Video || media_type == MediaType::AudioVideo,
            local_description: None,
            remote_description: None,
            ice_candidates: Vec::new(),
            joined_at: now,
            last_activity: now,
        }
    }

    /// Update peer connection state
    pub fn set_connection_state(&mut self, state: PeerConnectionState) {
        self.connection_state = state;
        self.last_activity = chrono::Utc::now();
    }

    /// Update peer state
    pub fn set_state(&mut self, state: PeerState) {
        self.state = state;
        self.last_activity = chrono::Utc::now();
    }

    /// Enable/disable audio
    pub fn set_audio_enabled(&mut self, enabled: bool) {
        self.audio_enabled = enabled;
        self.last_activity = chrono::Utc::now();
    }

    /// Enable/disable video
    pub fn set_video_enabled(&mut self, enabled: bool) {
        self.video_enabled = enabled;
        self.last_activity = chrono::Utc::now();
    }

    /// Set local session description
    pub fn set_local_description(&mut self, desc: SessionDescription) {
        self.local_description = Some(desc);
        self.last_activity = chrono::Utc::now();
    }

    /// Set remote session description
    pub fn set_remote_description(&mut self, desc: SessionDescription) {
        self.remote_description = Some(desc);
        self.last_activity = chrono::Utc::now();
    }

    /// Add ICE candidate
    pub fn add_ice_candidate(&mut self, candidate: IceCandidate) {
        self.ice_candidates.push(candidate);
        self.last_activity = chrono::Utc::now();
    }

    /// Clear ICE candidates
    pub fn clear_ice_candidates(&mut self) {
        self.ice_candidates.clear();
    }

    /// Check if peer is active
    pub fn is_active(&self) -> bool {
        self.connection_state == PeerConnectionState::Connected
            && (self.state == PeerState::Active || self.state == PeerState::Muted || self.state == PeerState::VideoOff)
    }

    /// Check if peer has timed out
    pub fn has_timed_out(&self, timeout_seconds: i64) -> bool {
        let now = chrono::Utc::now();
        let elapsed = now.signed_duration_since(self.last_activity);
        elapsed.num_seconds() > timeout_seconds
    }
}

/// Peer manager for a WebRTC session
#[derive(Clone)]
pub struct PeerManager {
    peers: Arc<RwLock<HashMap<String, Peer>>>,
}

impl PeerManager {
    /// Create a new peer manager
    pub fn new() -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a peer to the session
    pub async fn add_peer(&self, peer: Peer) -> Result<()> {
        let mut peers = self.peers.write().await;
        if peers.contains_key(&peer.id) {
            return Err(Error::AlreadyExists("Peer already exists".to_string()));
        }
        peers.insert(peer.id.clone(), peer);
        Ok(())
    }

    /// Remove a peer from the session
    pub async fn remove_peer(&self, peer_id: &str) -> Result<Peer> {
        let mut peers = self.peers.write().await;
        peers
            .remove(peer_id)
            .ok_or_else(|| Error::NotFound("Peer not found".to_string()))
    }

    /// Get a peer by ID
    pub async fn get_peer(&self, peer_id: &str) -> Result<Peer> {
        let peers = self.peers.read().await;
        peers
            .get(peer_id)
            .cloned()
            .ok_or_else(|| Error::NotFound("Peer not found".to_string()))
    }

    /// Get a peer by user ID
    pub async fn get_peer_by_user_id(&self, user_id: &UserId) -> Result<Peer> {
        let peers = self.peers.read().await;
        for peer in peers.values() {
            if peer.user_id == *user_id {
                return Ok(peer.clone());
            }
        }
        Err(Error::NotFound("Peer not found".to_string()))
    }

    /// Update a peer
    pub async fn update_peer<F>(&self, peer_id: &str, f: F) -> Result<Peer>
    where
        F: FnOnce(&mut Peer),
    {
        let mut peers = self.peers.write().await;
        let peer = peers
            .get_mut(peer_id)
            .ok_or_else(|| Error::NotFound("Peer not found".to_string()))?;
        f(peer);
        Ok(peer.clone())
    }

    /// List all peers
    pub async fn list_peers(&self) -> Vec<Peer> {
        let peers = self.peers.read().await;
        peers.values().cloned().collect()
    }

    /// Count active peers
    pub async fn active_peer_count(&self) -> usize {
        let peers = self.peers.read().await;
        peers.values().filter(|p| p.is_active()).count()
    }

    /// Remove timed-out peers
    pub async fn remove_timed_out_peers(&self, timeout_seconds: i64) -> Vec<Peer> {
        let mut peers = self.peers.write().await;
        let mut timed_out = Vec::new();

        peers.retain(|_, peer| {
            if peer.has_timed_out(timeout_seconds) {
                timed_out.push(peer.clone());
                false
            } else {
                true
            }
        });

        timed_out
    }

    /// Clear all peers
    pub async fn clear(&self) {
        let mut peers = self.peers.write().await;
        peers.clear();
    }
}

impl Default for PeerManager {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for PeerManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerManager")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_peer_creation() {
        let user_id = UserId::new();
        let peer = Peer::new(user_id.clone(), "alice".to_string(), MediaType::AudioVideo);

        assert_eq!(peer.user_id, user_id);
        assert_eq!(peer.username, "alice");
        assert_eq!(peer.media_type, MediaType::AudioVideo);
        assert_eq!(peer.connection_state, PeerConnectionState::New);
        assert!(peer.audio_enabled);
        assert!(peer.video_enabled);
    }

    #[tokio::test]
    async fn test_peer_manager() {
        let manager = PeerManager::new();
        let user_id = UserId::new();
        let peer = Peer::new(user_id, "alice".to_string(), MediaType::AudioVideo);

        // Add peer
        manager.add_peer(peer.clone()).await.unwrap();

        // Get peer
        let retrieved = manager.get_peer(&peer.id).await.unwrap();
        assert_eq!(retrieved.id, peer.id);

        // List peers
        let peers = manager.list_peers().await;
        assert_eq!(peers.len(), 1);

        // Remove peer
        let removed = manager.remove_peer(&peer.id).await.unwrap();
        assert_eq!(removed.id, peer.id);

        // Peer should be gone
        assert!(manager.get_peer(&peer.id).await.is_err());
    }

    #[tokio::test]
    async fn test_peer_state_updates() {
        let user_id = UserId::new();
        let mut peer = Peer::new(user_id, "alice".to_string(), MediaType::AudioVideo);

        // Update connection state
        peer.set_connection_state(PeerConnectionState::Connected);
        assert_eq!(peer.connection_state, PeerConnectionState::Connected);

        // Update state
        peer.set_state(PeerState::Muted);
        assert_eq!(peer.state, PeerState::Muted);

        // Toggle audio
        peer.set_audio_enabled(false);
        assert!(!peer.audio_enabled);

        // Toggle video
        peer.set_video_enabled(false);
        assert!(!peer.video_enabled);
    }

    #[tokio::test]
    async fn test_peer_timeout() {
        let user_id = UserId::new();
        let mut peer = Peer::new(user_id, "alice".to_string(), MediaType::Audio);

        // Fresh peer should not be timed out
        assert!(!peer.has_timed_out(60));

        // Simulate old activity
        peer.last_activity = chrono::Utc::now() - chrono::Duration::seconds(120);

        // Should be timed out with 60 second threshold
        assert!(peer.has_timed_out(60));

        // Should not be timed out with 180 second threshold
        assert!(!peer.has_timed_out(180));
    }
}
