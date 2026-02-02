//! WebRTC signaling service
//!
//! Handles WebRTC signaling for peer-to-peer connections.
//! Manages the offer/answer exchange and ICE candidate exchange.

use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};

use crate::{models::UserId, Error, Result};
use super::{
    session::{SessionId, SessionManager, SessionState, Session},
    peer::{Peer, PeerManager, PeerConnectionState},
    {SessionDescription, SdpType, IceCandidate, MediaType, WebRTCConfig},
};
use crate::models::RoomId;

/// Signaling message types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SignalingMessage {
    /// Offer to establish a connection
    Offer { session_id: String, sdp: SessionDescription },
    /// Answer to an offer
    Answer { session_id: String, peer_id: String, sdp: SessionDescription },
    /// ICE candidate for connection establishment
    IceCandidate { session_id: String, peer_id: String, candidate: IceCandidate },
    /// Peer is joining the session
    Join { session_id: String, peer_id: String, username: String },
    /// Peer is leaving the session
    Leave { session_id: String, peer_id: String },
    /// Request to start a call
    CallRequest { room_id: String, media_type: MediaType },
    /// Accept a call request
    CallAccept { session_id: String },
    /// Reject a call request
    CallReject { room_id: String, reason: String },
    /// End a call
    EndCall { session_id: String },
}

/// WebRTC signaling service
#[derive(Clone)]
pub struct SignalingService {
    config: WebRTCConfig,
    session_manager: Arc<SessionManager>,
}

impl SignalingService {
    /// Create a new signaling service
    pub fn new(config: WebRTCConfig) -> Self {
        let session_manager = Arc::new(SessionManager::new(config.session_timeout_seconds));
        Self {
            config,
            session_manager,
        }
    }

    /// Create a signaling service with default configuration
    pub fn with_defaults() -> Self {
        Self::new(WebRTCConfig::default())
    }

    /// Get ICE server configuration for clients
    pub fn get_ice_servers(&self) -> IceServerConfig {
        IceServerConfig {
            stun_servers: self.config.stun_servers.clone(),
            turn_config: self.config.turn_config.clone(),
        }
    }

    /// Create a new WebRTC session
    pub async fn create_session(
        &self,
        room_id: String,
        media_type: MediaType,
        initiator_id: UserId,
    ) -> Result<CreateSessionResponse> {
        // Check if a session already exists for this room
        let room_id_typed = RoomId::from_string(room_id.clone());
        if let Ok(_) = self.session_manager.get_session_by_room(&room_id_typed).await {
            return Err(Error::AlreadyExists("Session already exists for this room".to_string()));
        }

        // Create new session
        let session = self
            .session_manager
            .create_session(room_id_typed, media_type, self.config.max_participants)
            .await?;

        // Add initiator as first peer
        let peer = Peer::new(initiator_id.clone(), "Initiator".to_string(), media_type);
        session.peer_manager.add_peer(peer).await?;

        Ok(CreateSessionResponse {
            session_id: session.id.0.clone(),
            ice_servers: self.get_ice_servers(),
        })
    }

    /// Join an existing WebRTC session
    pub async fn join_session(
        &self,
        session_id: &str,
        user_id: UserId,
        username: String,
    ) -> Result<JoinSessionResponse> {
        let session_id = SessionId::from_string(session_id.to_string());
        let mut session = self.session_manager.get_session(&session_id).await?;

        // Check if session is full
        if session.is_full() {
            return Err(Error::InvalidInput("Session is full".to_string()));
        }

        // Check if user is already in the session
        if let Ok(_) = session.peer_manager.get_peer_by_user_id(&user_id).await {
            return Err(Error::AlreadyExists("User already in session".to_string()));
        }

        // Add peer to session
        let peer = Peer::new(user_id.clone(), username, session.media_type);
        let peer_id = peer.id.clone();
        session.peer_manager.add_peer(peer.clone()).await?;

        // Get all other peers in the session
        let existing_peers = session
            .peer_manager
            .list_peers()
            .await
            .into_iter()
            .filter(|p| p.id != peer_id)
            .collect();

        Ok(JoinSessionResponse {
            peer_id: peer.id.clone(),
            peers: existing_peers,
            session_state: session.state,
        })
    }

    /// Handle WebRTC offer
    pub async fn handle_offer(
        &self,
        session_id: &str,
        peer_id: &str,
        offer: SessionDescription,
    ) -> Result<HandleOfferResponse> {
        let session_id = SessionId::from_string(session_id.to_string());
        let mut session = self.session_manager.get_session(&session_id).await?;

        // Update peer with local description
        let _peer = session
            .peer_manager
            .update_peer(peer_id, |peer| {
                peer.set_local_description(offer.clone());
                peer.set_connection_state(PeerConnectionState::Connecting);
            })
            .await?;

        Ok(HandleOfferResponse {
            success: true,
        })
    }

    /// Handle WebRTC answer
    pub async fn handle_answer(
        &self,
        session_id: &str,
        peer_id: &str,
        answer: SessionDescription,
    ) -> Result<HandleAnswerResponse> {
        let session_id = SessionId::from_string(session_id.to_string());
        let mut session = self.session_manager.get_session(&session_id).await?;

        // Update peer with remote description
        let _peer = session
            .peer_manager
            .update_peer(peer_id, |peer| {
                peer.set_remote_description(answer.clone());
                peer.set_connection_state(PeerConnectionState::Connecting);
            })
            .await?;

        Ok(HandleAnswerResponse {
            success: true,
        })
    }

    /// Handle ICE candidate
    pub async fn handle_ice_candidate(
        &self,
        session_id: &str,
        peer_id: &str,
        candidate: IceCandidate,
    ) -> Result<HandleIceCandidateResponse> {
        let session_id = SessionId::from_string(session_id.to_string());
        let mut session = self.session_manager.get_session(&session_id).await?;

        // Add ICE candidate to peer
        let peer = session
            .peer_manager
            .update_peer(peer_id, |peer| {
                peer.add_ice_candidate(candidate.clone());
            })
            .await?;

        // In a real implementation, we would broadcast this candidate to other peers
        // For now, just acknowledge receipt

        Ok(HandleIceCandidateResponse {
            success: true,
        })
    }

    /// Leave a WebRTC session
    pub async fn leave_session(&self, session_id: &str, peer_id: &str) -> Result<()> {
        let session_id = SessionId::from_string(session_id.to_string());
        let mut session = self.session_manager.get_session(&session_id).await?;

        // Remove peer from session
        let _peer = session.peer_manager.remove_peer(peer_id).await?;

        // If no peers left, end the session
        let peer_count = session.peer_manager.active_peer_count().await;
        if peer_count == 0 {
            session.end();
        }

        Ok(())
    }

    /// End a WebRTC session
    pub async fn end_session(&self, session_id: &str) -> Result<()> {
        let session_id = SessionId::from_string(session_id.to_string());
        let _session = self.session_manager.end_session(&session_id).await?;
        Ok(())
    }

    /// Get session information
    pub async fn get_session_info(&self, session_id: &str) -> Result<SessionInfo> {
        let session_id = SessionId::from_string(session_id.to_string());
        let session = self.session_manager.get_session(&session_id).await?;

        let peers = session.peer_manager.list_peers().await;

        Ok(SessionInfo {
            session_id: session.id.0,
            room_id: session.room_id.0,
            state: session.state,
            media_type: session.media_type,
            peer_count: peers.len(),
            peers,
            created_at: session.created_at,
        })
    }

    /// Get list of active sessions
    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let sessions = self.session_manager.list_sessions().await;

        let mut session_infos = Vec::new();
        for session in sessions {
            let peers = session.peer_manager.list_peers().await;

            session_infos.push(SessionInfo {
                session_id: session.id.0,
                room_id: session.room_id.0,
                state: session.state,
                media_type: session.media_type,
                peer_count: peers.len(),
                peers,
                created_at: session.created_at,
            });
        }

        Ok(session_infos)
    }

    /// Clean up timed-out sessions
    pub async fn cleanup_timed_out_sessions(&self) -> Result<Vec<String>> {
        let timed_out = self.session_manager.remove_timed_out_sessions().await;

        let session_ids = timed_out
            .into_iter()
            .map(|s| s.id.0)
            .collect();

        Ok(session_ids)
    }
}

/// ICE server configuration for clients
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceServerConfig {
    pub stun_servers: Vec<String>,
    pub turn_config: Option<super::TurnConfig>,
}

/// Response for creating a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionResponse {
    pub session_id: String,
    pub ice_servers: IceServerConfig,
}

/// Response for joining a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinSessionResponse {
    pub peer_id: String,
    pub peers: Vec<Peer>,
    pub session_state: SessionState,
}

/// Response for handling an offer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandleOfferResponse {
    pub success: bool,
}

/// Response for handling an answer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandleAnswerResponse {
    pub success: bool,
}

/// Response for handling an ICE candidate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandleIceCandidateResponse {
    pub success: bool,
}

/// Session information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub room_id: String,
    pub state: SessionState,
    pub media_type: MediaType,
    pub peer_count: usize,
    pub peers: Vec<Peer>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_signaling_service_creation() {
        let service = SignalingService::with_defaults();

        let ice_servers = service.get_ice_servers();
        assert!(!ice_servers.stun_servers.is_empty());
    }

    #[tokio::test]
    async fn test_create_session() {
        let service = SignalingService::with_defaults();
        let user_id = UserId::new();

        let response = service
            .create_session("room1".to_string(), MediaType::AudioVideo, user_id)
            .await
            .unwrap();

        assert!(!response.session_id.is_empty());
        assert!(!response.ice_servers.stun_servers.is_empty());
    }

    #[tokio::test]
    async fn test_join_session() {
        let service = SignalingService::with_defaults();
        let user1_id = UserId::new();

        // Create session
        let create_response = service
            .create_session("room1".to_string(), MediaType::AudioVideo, user1_id)
            .await
            .unwrap();

        // Join session with another user
        let user2_id = UserId::new();
        let join_response = service
            .join_session(
                &create_response.session_id,
                user2_id,
                "user2".to_string(),
            )
            .await
            .unwrap();

        assert!(!join_response.peer_id.is_empty());
        assert_eq!(join_response.peers.len(), 1); // Should have 1 existing peer
    }

    #[tokio::test]
    async fn test_session_info() {
        let service = SignalingService::with_defaults();
        let user_id = UserId::new();

        let create_response = service
            .create_session("room1".to_string(), MediaType::Audio, user_id)
            .await
            .unwrap();

        let session_info = service
            .get_session_info(&create_response.session_id)
            .await
            .unwrap();

        assert_eq!(session_info.session_id, create_response.session_id);
        assert_eq!(session_info.room_id, "room1");
        assert_eq!(session_info.media_type, MediaType::Audio);
    }

    #[tokio::test]
    async fn test_leave_session() {
        let service = SignalingService::with_defaults();
        let user1_id = UserId::new();
        let user2_id = UserId::new();

        let create_response = service
            .create_session("room1".to_string(), MediaType::Audio, user1_id)
            .await
            .unwrap();

        let join_response = service
            .join_session(
                &create_response.session_id,
                user2_id,
                "user2".to_string(),
            )
            .await
            .unwrap();

        // Leave session
        service
            .leave_session(&create_response.session_id, &join_response.peer_id)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_end_session() {
        let service = SignalingService::with_defaults();
        let user_id = UserId::new();

        let create_response = service
            .create_session("room1".to_string(), MediaType::Audio, user_id)
            .await
            .unwrap();

        // End session
        service
            .end_session(&create_response.session_id)
            .await
            .unwrap();

        // Session should no longer exist
        assert!(service
            .get_session_info(&create_response.session_id)
            .await
            .is_err());
    }
}
