//! SFU Manager

use crate::config::SfuConfig;
use crate::room::{RoomStats, SfuRoom};
use crate::types::{PeerId, RoomId};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct SfuManager {
    config: Arc<SfuConfig>,
    rooms: Arc<RwLock<HashMap<RoomId, Arc<SfuRoom>>>>,
}

impl SfuManager {
    pub fn new(config: SfuConfig) -> Self {
        Self {
            config: Arc::new(config),
            rooms: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_or_create_room(&self, room_id: RoomId) -> Result<Arc<SfuRoom>> {
        let mut rooms = self.rooms.write().await;
        if let Some(room) = rooms.get(&room_id) {
            return Ok(room.clone());
        }

        let room = Arc::new(SfuRoom::new(room_id.clone(), self.config.clone()));
        rooms.insert(room_id, room.clone());
        Ok(room)
    }

    pub async fn add_peer_to_room(&self, room_id: RoomId, peer_id: PeerId) -> Result<()> {
        let room = self.get_or_create_room(room_id).await?;
        room.add_peer(peer_id).await?;
        Ok(())
    }

    pub async fn remove_peer_from_room(&self, room_id: &RoomId, peer_id: &PeerId) -> Result<()> {
        let rooms = self.rooms.read().await;
        if let Some(room) = rooms.get(room_id) {
            room.remove_peer(peer_id).await?;
            if room.is_empty().await {
                drop(rooms);
                self.rooms.write().await.remove(room_id);
            }
        }
        Ok(())
    }

    pub async fn get_room_stats(&self, room_id: &RoomId) -> Result<RoomStats> {
        let rooms = self.rooms.read().await;
        if let Some(room) = rooms.get(room_id) {
            Ok(room.get_stats().await)
        } else {
            Ok(RoomStats::default())
        }
    }

    pub fn config(&self) -> &SfuConfig {
        &self.config
    }
}
