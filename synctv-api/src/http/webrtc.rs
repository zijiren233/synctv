//! WebRTC HTTP REST API endpoints
//!
//! Provides HTTP/JSON API for WebRTC configuration and control:
//! - `/api/rooms/{room_id}/webrtc/ice-servers` - Get ICE servers configuration (STUN/TURN)
//! - `/api/rooms/{room_id}/webrtc/network-quality` - Get network quality stats
//! - Includes TURN credential generation for authenticated users
//! - Supports all WebRTC modes (`SignalingOnly`, `PeerToPeer`, Hybrid, SFU)

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};

use crate::http::{AppError, AppResult, AppState};
use crate::http::middleware::AuthUser;
use synctv_core::models::RoomId;

/// ICE Server configuration (STUN/TURN)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceServerConfig {
    /// URLs for the ICE server (e.g., ["stun:stun.example.com:3478"])
    pub urls: Vec<String>,
    /// Username for TURN authentication (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Credential for TURN authentication (optional, time-limited)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

/// Response for ICE servers request
#[derive(Debug, Serialize, Deserialize)]
pub struct GetIceServersResponse {
    /// List of ICE servers (STUN and TURN)
    pub servers: Vec<IceServerConfig>,
}

/// Get ICE servers configuration for WebRTC
///
/// Returns a list of STUN/TURN servers configured for this deployment.
/// For TURN servers, temporary credentials are generated for the authenticated user.
///
/// Path: `GET /api/rooms/{room_id}/webrtc/ice-servers`
/// Auth: Required (JWT)
/// Permissions: Room membership required
///
/// # Response
/// ```json
/// {
///   "servers": [
///     {
///       "urls": ["stun:stun.example.com:3478"]
///     },
///     {
///       "urls": ["turn:turn.example.com:3478"],
///       "username": "1234567890:user123",
///       "credential": "base64encodedsecret"
///     }
///   ]
/// }
/// ```
///
/// # Configuration
/// The returned servers depend on the configured WebRTC mode:
/// - **`SignalingOnly`**: Empty list (client must use public STUN servers)
/// - **`PeerToPeer`**: Built-in STUN + external STUN servers
/// - **Hybrid/SFU**: Full configuration with STUN and TURN
///
/// TURN credentials are time-limited (default 24 hours) and generated using HMAC-SHA1.
pub async fn get_ice_servers(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let user_id = auth.user_id;
    let room_id = RoomId::from_string(room_id);

    // Verify user is a member of this room
    state.room_service.check_membership(&room_id, &user_id).await
        .map_err(|_| AppError::forbidden("Not a member of this room"))?;

    let response = state
        .client_api
        .get_ice_servers(&room_id, &user_id)
        .await
        .map_err(|e| AppError::internal_server_error(e.to_string()))?;

    // Convert proto response to HTTP response
    let servers: Vec<IceServerConfig> = response
        .servers
        .into_iter()
        .map(|server| IceServerConfig {
            urls: server.urls,
            username: server.username,
            credential: server.credential,
        })
        .collect();

    Ok(Json(GetIceServersResponse { servers }))
}

/// Peer network quality information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerNetworkQualityInfo {
    pub peer_id: String,
    pub rtt_ms: u32,
    pub packet_loss_rate: f32,
    pub jitter_ms: u32,
    pub available_bandwidth_kbps: u32,
    pub quality_score: u32,
    pub quality_action: String,
}

/// Response for network quality request
#[derive(Debug, Serialize, Deserialize)]
pub struct GetNetworkQualityResponse {
    pub peers: Vec<PeerNetworkQualityInfo>,
}

/// Get network quality stats for WebRTC peers in a room
///
/// Path: `GET /api/rooms/{room_id}/webrtc/network-quality`
/// Auth: Required (JWT)
/// Permissions: Room membership required
pub async fn get_network_quality(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let user_id = auth.user_id;
    let room_id = RoomId::from_string(room_id);

    // Verify user is a member of this room
    state.room_service.check_membership(&room_id, &user_id).await
        .map_err(|_| AppError::forbidden("Not a member of this room"))?;

    let response = state
        .client_api
        .get_network_quality(&room_id, &user_id)
        .await
        .map_err(|e| AppError::internal_server_error(e.to_string()))?;

    // Convert proto response to HTTP response
    let peers: Vec<PeerNetworkQualityInfo> = response
        .peers
        .into_iter()
        .map(|p| PeerNetworkQualityInfo {
            peer_id: p.peer_id,
            rtt_ms: p.rtt_ms,
            packet_loss_rate: p.packet_loss_rate,
            jitter_ms: p.jitter_ms,
            available_bandwidth_kbps: p.available_bandwidth_kbps,
            quality_score: p.quality_score,
            quality_action: p.quality_action,
        })
        .collect();

    Ok(Json(GetNetworkQualityResponse { peers }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ice_server_serialization() {
        let server = IceServerConfig {
            urls: vec!["stun:stun.example.com:3478".to_string()],
            username: None,
            credential: None,
        };

        let json = serde_json::to_string(&server).expect("IceServerConfig should serialize");
        assert!(json.contains("stun:stun.example.com:3478"));
        assert!(!json.contains("username"));
    }

    #[test]
    fn test_turn_server_serialization() {
        let server = IceServerConfig {
            urls: vec!["turn:turn.example.com:3478".to_string()],
            username: Some("1234567890:user123".to_string()),
            credential: Some("secret123".to_string()),
        };

        let json = serde_json::to_string(&server).expect("IceServerConfig should serialize");
        assert!(json.contains("turn:turn.example.com:3478"));
        assert!(json.contains("username"));
        assert!(json.contains("credential"));
    }
}
