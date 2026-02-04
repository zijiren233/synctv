//! SFU Room management

use crate::config::SfuConfig;
use crate::peer::SfuPeer;
use crate::types::{PeerId, RoomId};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoomMode {
    P2P,
    SFU,
}

pub struct SfuRoom {
    pub id: RoomId,
    pub mode: Arc<RwLock<RoomMode>>,
    pub peers: Arc<RwLock<HashMap<PeerId, Arc<SfuPeer>>>>,
    pub config: Arc<SfuConfig>,
}

impl SfuRoom {
    pub fn new(id: RoomId, config: Arc<SfuConfig>) -> Self {
        Self {
            id,
            mode: Arc::new(RwLock::new(RoomMode::P2P)),
            peers: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    pub async fn add_peer(&self, peer_id: PeerId) -> Result<Arc<SfuPeer>> {
        let peer = Arc::new(SfuPeer::new(peer_id.clone()));
        self.peers.write().await.insert(peer_id, peer.clone());
        self.check_mode_switch().await?;
        Ok(peer)
    }

    pub async fn remove_peer(&self, peer_id: &PeerId) -> Result<()> {
        self.peers.write().await.remove(peer_id);
        self.check_mode_switch().await?;
        Ok(())
    }

    pub async fn peer_count(&self) -> usize {
        self.peers.read().await.len()
    }

    async fn check_mode_switch(&self) -> Result<()> {
        let count = self.peer_count().await;
        let threshold = self.config.sfu_threshold;
        let mut mode = self.mode.write().await;
        
        if count >= threshold && *mode == RoomMode::P2P {
            *mode = RoomMode::SFU;
        } else if count < threshold && *mode == RoomMode::SFU {
            *mode = RoomMode::P2P;
        }
        
        Ok(())
    }

    pub async fn is_empty(&self) -> bool {
        self.peers.read().await.is_empty()
    }

    pub async fn get_stats(&self) -> RoomStats {
        RoomStats {
            peer_count: self.peer_count().await,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoomStats {
    pub peer_count: usize,
    pub total_peers_joined: u64,
    pub mode_switches: u64,
}
