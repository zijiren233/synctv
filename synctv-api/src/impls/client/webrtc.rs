//! WebRTC operations: ICE servers, network quality

use synctv_core::models::{RoomId, UserId};

use super::ClientApiImpl;
use super::convert::network_stats_to_proto;

impl ClientApiImpl {
    /// Get ICE servers configuration for WebRTC (STUN only, no TURN)
    pub async fn get_ice_servers(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<crate::proto::client::GetIceServersResponse, anyhow::Error> {
        use crate::proto::client::{IceServer, GetIceServersResponse};

        // Check membership
        self.room_service.check_membership(room_id, user_id).await
            .map_err(|e| anyhow::anyhow!("Forbidden: {e}"))?;

        let webrtc_config = &self.config.webrtc;
        let mut servers = Vec::new();

        // Add built-in STUN server if enabled
        if webrtc_config.enable_builtin_stun {
            let stun_url = format!(
                "stun:{}:{}",
                self.config.server.host,
                webrtc_config.stun_port
            );
            servers.push(IceServer {
                urls: vec![stun_url],
                username: None,
                credential: None,
            });
        }

        // Add external STUN servers
        for url in &webrtc_config.external_stun_servers {
            servers.push(IceServer {
                urls: vec![url.clone()],
                username: None,
                credential: None,
            });
        }

        Ok(GetIceServersResponse { servers })
    }

    /// Get network quality stats for peers in a room
    pub async fn get_network_quality(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<crate::proto::client::GetNetworkQualityResponse, anyhow::Error> {
        use crate::proto::client::GetNetworkQualityResponse;

        // Check membership
        self.room_service.check_membership(room_id, user_id).await
            .map_err(|e| anyhow::anyhow!("Forbidden: {e}"))?;

        let sfu_manager = if let Some(mgr) = &self.sfu_manager { mgr } else {
            tracing::debug!(
                room_id = %room_id,
                user_id = %user_id,
                "Network quality requested but SFU manager not enabled"
            );
            return Ok(GetNetworkQualityResponse { peers: vec![] });
        };

        let stats = sfu_manager.get_room_network_quality(
            &synctv_sfu::RoomId::from(room_id.as_str()),
        )?;

        let peers = stats
            .into_iter()
            .map(|(peer_id, ns)| network_stats_to_proto(peer_id, ns))
            .collect();

        Ok(GetNetworkQualityResponse { peers })
    }
}
