//! WebRTC signaling service
//!
//! Provides WebRTC signaling for peer-to-peer audio/video calls.
//! Supports STUN/TURN for NAT traversal.

pub mod signaling;
pub mod peer;
pub mod session;

pub use signaling::{SignalingService, SignalingMessage};
pub use peer::{Peer, PeerState, PeerConnectionState, PeerManager};
pub use session::{Session, SessionId, SessionState};

use serde::{Deserialize, Serialize};

/// WebRTC configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebRTCConfig {
    /// STUN server URLs for NAT traversal
    pub stun_servers: Vec<String>,
    /// TURN server configuration
    pub turn_config: Option<TurnConfig>,
    /// Maximum number of participants in a session
    pub max_participants: usize,
    /// Session timeout in seconds
    pub session_timeout_seconds: u64,
}

impl Default for WebRTCConfig {
    fn default() -> Self {
        Self {
            stun_servers: vec![
                "stun:stun.l.google.com:19302".to_string(),
                "stun:stun1.l.google.com:19302".to_string(),
            ],
            turn_config: None,
            max_participants: 8,
            session_timeout_seconds: 3600, // 1 hour
        }
    }
}

/// TURN server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnConfig {
    /// TURN server URL
    pub server_url: String,
    /// TURN username
    pub username: String,
    /// TURN password
    pub password: String,
    /// TURN protocol (udp, tcp, tls)
    pub protocol: String,
}

/// ICE candidate for WebRTC connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceCandidate {
    /// Full candidate string
    pub candidate: String,
    /// SDP mid
    pub sdp_mid: Option<String>,
    /// SDP mline index
    pub sdp_mline_index: Option<u32>,
}

/// Session description (SDP)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDescription {
    /// Session description type (offer, answer, pranswer, rollback)
    pub sdp_type: SdpType,
    /// SDP content
    pub sdp: String,
}

/// SDP type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SdpType {
    Offer,
    Answer,
    Pranswer,
    Rollback,
}

impl SdpType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Offer => "offer",
            Self::Answer => "answer",
            Self::Pranswer => "pranswer",
            Self::Rollback => "rollback",
        }
    }
}

/// Media type for the call
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaType {
    Audio,
    Video,
    AudioVideo,
}

/// Call direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallDirection {
    Incoming,
    Outgoing,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webrtc_config_default() {
        let config = WebRTCConfig::default();

        assert!(!config.stun_servers.is_empty());
        assert_eq!(config.max_participants, 8);
        assert_eq!(config.session_timeout_seconds, 3600);
        assert!(config.turn_config.is_none());
    }

    #[test]
    fn test_sdp_type() {
        let offer = SdpType::Offer;
        let answer = SdpType::Answer;

        assert_eq!(offer, SdpType::Offer);
        assert_ne!(offer, answer);
        assert_eq!(offer.as_str(), "offer");
    }

    #[test]
    fn test_session_description_serialization() {
        let desc = SessionDescription {
            sdp_type: SdpType::Offer,
            sdp: "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\n...".to_string(),
        };

        let json = serde_json::to_string(&desc).unwrap();
        let deserialized: SessionDescription = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.sdp_type, SdpType::Offer);
        assert_eq!(deserialized.sdp, desc.sdp);
    }

    #[test]
    fn test_ice_candidate() {
        let candidate = IceCandidate {
            candidate: "candidate:1 1 UDP 2130706431 192.168.1.1 54321 typ host".to_string(),
            sdp_mid: Some("0".to_string()),
            sdp_mline_index: Some(0),
        };

        let json = serde_json::to_string(&candidate).unwrap();
        let deserialized: IceCandidate = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.candidate, candidate.candidate);
        assert_eq!(deserialized.sdp_mid, candidate.sdp_mid);
        assert_eq!(deserialized.sdp_mline_index, candidate.sdp_mline_index);
    }
}
