//! WebRTC session management
//!
//! Manages WebRTC sessions (calls) with multiple participants.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;

use crate::{models::RoomId, Error, Result};
use super::{PeerManager, MediaType};

/// Unique session identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    /// Generate a new session ID
    pub fn new() -> Self {
        Self(nanoid::nanoid!(12))
    }

    /// Create session ID from string
    pub fn from_string(s: String) -> Self {
        Self(s)
    }

    /// Get session ID as string reference
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Session state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    /// Session is being created
    Creating,
    /// Session is active
    Active,
    /// Session is paused
    Paused,
    /// Session is ending
    Ending,
    /// Session has ended
    Ended,
}

/// WebRTC session (call)
#[derive(Debug, Clone)]
pub struct Session {
    /// Unique session ID
    pub id: SessionId,
    /// Room ID this session belongs to
    pub room_id: RoomId,
    /// Session state
    pub state: SessionState,
    /// Media type for the session
    pub media_type: MediaType,
    /// Maximum number of participants
    pub max_participants: usize,
    /// Peer manager for this session
    pub peer_manager: PeerManager,
    /// Session creation time
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Session start time (when it became active)
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Session end time
    pub ended_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Session {
    /// Create a new session
    pub fn new(room_id: RoomId, media_type: MediaType, max_participants: usize) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: SessionId::new(),
            room_id,
            state: SessionState::Creating,
            media_type,
            max_participants,
            peer_manager: PeerManager::new(),
            created_at: now,
            started_at: None,
            ended_at: None,
        }
    }

    /// Check if session is full
    pub fn is_full(&self) -> bool {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            self.peer_manager.active_peer_count().await >= self.max_participants
        })
    }

    /// Start the session
    pub fn start(&mut self) {
        self.state = SessionState::Active;
        self.started_at = Some(chrono::Utc::now());
    }

    /// End the session
    pub fn end(&mut self) {
        self.state = SessionState::Ended;
        self.ended_at = Some(chrono::Utc::now());
    }

    /// Pause the session
    pub fn pause(&mut self) {
        if self.state == SessionState::Active {
            self.state = SessionState::Paused;
        }
    }

    /// Resume the session
    pub fn resume(&mut self) {
        if self.state == SessionState::Paused {
            self.state = SessionState::Active;
        }
    }

    /// Get session duration (if ended)
    pub fn duration(&self) -> Option<chrono::Duration> {
        match (self.started_at, self.ended_at) {
            (Some(start), Some(end)) => Some(end.signed_duration_since(start)),
            (Some(start), None) => Some(chrono::Utc::now().signed_duration_since(start)),
            _ => None,
        }
    }

    /// Check if session has timed out
    pub fn has_timed_out(&self, timeout_seconds: i64) -> bool {
        let now = chrono::Utc::now();
        let last_activity = match (self.started_at, self.ended_at) {
            (_, Some(end)) => end,
            (Some(start), None) => start,
            (None, None) => self.created_at,
        };

        let elapsed = now.signed_duration_since(last_activity);
        elapsed.num_seconds() > timeout_seconds
    }
}

/// Session manager for all active WebRTC sessions
#[derive(Clone)]
pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<SessionId, Session>>>,
    /// Session timeout in seconds
    session_timeout: i64,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(session_timeout_seconds: u64) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            session_timeout: session_timeout_seconds as i64,
        }
    }

    /// Create a new session
    pub async fn create_session(
        &self,
        room_id: RoomId,
        media_type: MediaType,
        max_participants: usize,
    ) -> Result<Session> {
        let session = Session::new(room_id, media_type, max_participants);

        let mut sessions = self.sessions.write().await;
        sessions.insert(session.id.clone(), session.clone());

        Ok(session)
    }

    /// Get a session by ID
    pub async fn get_session(&self, session_id: &SessionId) -> Result<Session> {
        let sessions = self.sessions.read().await;
        sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| Error::NotFound("Session not found".to_string()))
    }

    /// Get session by room ID
    pub async fn get_session_by_room(&self, room_id: &RoomId) -> Result<Session> {
        let sessions = self.sessions.read().await;
        for session in sessions.values() {
            if session.room_id == *room_id {
                return Ok(session.clone());
            }
        }
        Err(Error::NotFound("Session not found for room".to_string()))
    }

    /// Update a session
    pub async fn update_session<F>(&self, session_id: &SessionId, f: F) -> Result<Session>
    where
        F: FnOnce(&mut Session),
    {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| Error::NotFound("Session not found".to_string()))?;
        f(session);
        Ok(session.clone())
    }

    /// End and remove a session
    pub async fn end_session(&self, session_id: &SessionId) -> Result<Session> {
        let mut sessions = self.sessions.write().await;
        let mut session = sessions
            .remove(session_id)
            .ok_or_else(|| Error::NotFound("Session not found".to_string()))?;

        // Clear all peers
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            session.peer_manager.clear().await;
        });

        session.end();
        Ok(session)
    }

    /// List all active sessions
    pub async fn list_sessions(&self) -> Vec<Session> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
    }

    /// Remove timed-out sessions
    pub async fn remove_timed_out_sessions(&self) -> Vec<Session> {
        let mut sessions = self.sessions.write().await;
        let mut timed_out = Vec::new();

        sessions.retain(|_, session| {
            if session.has_timed_out(self.session_timeout) {
                timed_out.push(session.clone());
                false
            } else {
                true
            }
        });

        timed_out
    }

    /// Clear all sessions
    pub async fn clear(&self) {
        let mut sessions = self.sessions.write().await;
        sessions.clear();
    }

    /// Get active session count
    pub async fn active_session_count(&self) -> usize {
        let sessions = self.sessions.read().await;
        sessions.values().filter(|s| s.state == SessionState::Active).count()
    }

    /// Get total participant count across all sessions
    pub async fn total_participant_count(&self) -> usize {
        let sessions = self.sessions.read().await;
        let mut total = 0;

        for session in sessions.values() {
            let rt = tokio::runtime::Runtime::new().unwrap();
            total += rt.block_on(async { session.peer_manager.active_peer_count().await });
        }

        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_creation() {
        let room_id = RoomId("room1".to_string());
        let session = Session::new(room_id.clone(), MediaType::AudioVideo, 8);

        assert_eq!(session.room_id, room_id);
        assert_eq!(session.media_type, MediaType::AudioVideo);
        assert_eq!(session.max_participants, 8);
        assert_eq!(session.state, SessionState::Creating);
    }

    #[tokio::test]
    async fn test_session_lifecycle() {
        let mut session = Session::new(
            RoomId("room1".to_string()),
            MediaType::Audio,
            5,
        );

        // Start session
        session.start();
        assert_eq!(session.state, SessionState::Active);
        assert!(session.started_at.is_some());

        // Pause session
        session.pause();
        assert_eq!(session.state, SessionState::Paused);

        // Resume session
        session.resume();
        assert_eq!(session.state, SessionState::Active);

        // End session
        session.end();
        assert_eq!(session.state, SessionState::Ended);
        assert!(session.ended_at.is_some());
    }

    #[tokio::test]
    async fn test_session_manager() {
        let manager = SessionManager::new(3600);
        let room_id = RoomId("room1".to_string());

        // Create session
        let session = manager
            .create_session(room_id.clone(), MediaType::AudioVideo, 8)
            .await
            .unwrap();

        // Get session
        let retrieved = manager.get_session(&session.id).await.unwrap();
        assert_eq!(retrieved.id, session.id);

        // Get session by room
        let by_room = manager.get_session_by_room(&room_id).await.unwrap();
        assert_eq!(by_room.id, session.id);

        // End session
        let ended = manager.end_session(&session.id).await.unwrap();
        assert_eq!(ended.state, SessionState::Ended);

        // Session should be gone
        assert!(manager.get_session(&session.id).await.is_err());
    }

    #[tokio::test]
    async fn test_session_timeout() {
        let mut session = Session::new(
            RoomId("room1".to_string()),
            MediaType::Audio,
            5,
        );

        // Fresh session should not be timed out
        assert!(!session.has_timed_out(3600));

        // Set old creation time
        session.created_at = chrono::Utc::now() - chrono::Duration::seconds(7200);

        // Should be timed out
        assert!(session.has_timed_out(3600));
    }
}
