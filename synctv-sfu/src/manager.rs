//! SFU Manager - Top-level orchestration for multi-room SFU management
//!
//! This module provides:
//! - Multi-room management with concurrent access
//! - Resource limit enforcement
//! - Automatic cleanup of empty rooms
//! - Global statistics collection
//! - Background maintenance tasks

use crate::config::SfuConfig;
use crate::room::{RoomMode, RoomStats, SfuRoom};
use crate::track::MediaTrack;
use crate::types::{PeerId, RoomId, TrackId};
use anyhow::{anyhow, Result};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};

/// Global SFU manager statistics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManagerStats {
    /// Number of active rooms
    pub active_rooms: usize,
    /// Number of rooms in SFU mode
    pub sfu_mode_rooms: usize,
    /// Number of rooms in P2P mode
    pub p2p_mode_rooms: usize,
    /// Total number of peers across all rooms
    pub total_peers: usize,
    /// Total number of audio tracks
    pub total_audio_tracks: usize,
    /// Total number of video tracks
    pub total_video_tracks: usize,
    /// Total bytes relayed through SFU
    pub total_bytes_relayed: u64,
    /// Total packets relayed through SFU
    pub total_packets_relayed: u64,
}

/// SFU Manager - manages multiple rooms and provides top-level orchestration
pub struct SfuManager {
    /// Configuration
    config: Arc<SfuConfig>,

    /// Active rooms (uses DashMap for lock-free concurrent access)
    rooms: DashMap<RoomId, Arc<SfuRoom>>,

    /// Global statistics
    stats: Arc<RwLock<ManagerStats>>,
}

impl SfuManager {
    /// Create a new SFU manager
    pub fn new(config: SfuConfig) -> Arc<Self> {
        let manager = Arc::new(Self {
            config: Arc::new(config),
            rooms: DashMap::new(),
            stats: Arc::new(RwLock::new(ManagerStats::default())),
        });

        info!(
            sfu_threshold = manager.config.sfu_threshold,
            max_sfu_rooms = manager.config.max_sfu_rooms,
            max_peers_per_room = manager.config.max_peers_per_room,
            "SFU Manager initialized"
        );

        // Start background tasks
        let manager_clone = Arc::clone(&manager);
        tokio::spawn(async move {
            manager_clone.cleanup_task().await;
        });

        let manager_clone = Arc::clone(&manager);
        tokio::spawn(async move {
            manager_clone.stats_collection_task().await;
        });

        manager
    }

    /// Get or create a room
    pub async fn get_or_create_room(&self, room_id: RoomId) -> Result<Arc<SfuRoom>> {
        // Check if room already exists
        if let Some(room) = self.rooms.get(&room_id) {
            debug!(room_id = %room_id, "Room already exists");
            return Ok(Arc::clone(room.value()));
        }

        // Enforce room limit (0 = unlimited)
        if self.config.max_sfu_rooms > 0 && self.rooms.len() >= self.config.max_sfu_rooms {
            warn!(
                current_rooms = self.rooms.len(),
                max_rooms = self.config.max_sfu_rooms,
                "Room limit reached"
            );
            return Err(anyhow!("Maximum number of SFU rooms reached"));
        }

        // Create new room
        let room = Arc::new(SfuRoom::new(room_id.clone(), Arc::clone(&self.config)));
        self.rooms.insert(room_id.clone(), Arc::clone(&room));

        info!(
            room_id = %room_id,
            total_rooms = self.rooms.len(),
            "Created new room"
        );

        Ok(room)
    }

    /// Add a peer to a room
    pub async fn add_peer_to_room(&self, room_id: RoomId, peer_id: PeerId) -> Result<()> {
        let room = self.get_or_create_room(room_id.clone()).await?;

        // Check peer limit (0 = unlimited)
        let peer_count = room.peer_count().await;
        if self.config.max_peers_per_room > 0 && peer_count >= self.config.max_peers_per_room {
            warn!(
                room_id = %room_id,
                current_peers = peer_count,
                max_peers = self.config.max_peers_per_room,
                "Peer limit reached for room"
            );
            return Err(anyhow!("Maximum number of peers reached for this room"));
        }

        room.add_peer(peer_id.clone()).await?;

        info!(
            room_id = %room_id,
            peer_id = %peer_id,
            peer_count = peer_count + 1,
            "Added peer to room"
        );

        Ok(())
    }

    /// Remove a peer from a room
    pub async fn remove_peer_from_room(&self, room_id: &RoomId, peer_id: &PeerId) -> Result<()> {
        if let Some(room_entry) = self.rooms.get(room_id) {
            let room = room_entry.value();
            room.remove_peer(peer_id).await?;

            info!(
                room_id = %room_id,
                peer_id = %peer_id,
                "Removed peer from room"
            );

            // If room is empty, it will be cleaned up by the cleanup task
        } else {
            debug!(room_id = %room_id, "Room not found when removing peer");
        }

        Ok(())
    }

    /// Publish a track in a room
    pub async fn publish_track(
        &self,
        room_id: &RoomId,
        peer_id: &PeerId,
        track_id: TrackId,
        track: Arc<MediaTrack>,
    ) -> Result<()> {
        if let Some(room_entry) = self.rooms.get(room_id) {
            let room = room_entry.value();
            room.add_published_track(peer_id, track_id, track).await?;
            Ok(())
        } else {
            Err(anyhow!("Room not found"))
        }
    }

    /// Unpublish a track from a room
    pub async fn unpublish_track(
        &self,
        room_id: &RoomId,
        peer_id: &PeerId,
        track_id: &TrackId,
    ) -> Result<()> {
        if let Some(room_entry) = self.rooms.get(room_id) {
            let room = room_entry.value();
            room.remove_published_track(peer_id, track_id).await?;
            Ok(())
        } else {
            Err(anyhow!("Room not found"))
        }
    }

    /// Subscribe to a track
    pub async fn subscribe_track(
        &self,
        room_id: &RoomId,
        subscriber_peer_id: &PeerId,
        track_id: &TrackId,
    ) -> Result<()> {
        if let Some(room_entry) = self.rooms.get(room_id) {
            let room = room_entry.value();
            room.subscribe_track(subscriber_peer_id, track_id).await?;
            Ok(())
        } else {
            Err(anyhow!("Room not found"))
        }
    }

    /// Unsubscribe from a track
    pub async fn unsubscribe_track(
        &self,
        room_id: &RoomId,
        subscriber_peer_id: &PeerId,
        track_id: &TrackId,
    ) -> Result<()> {
        if let Some(room_entry) = self.rooms.get(room_id) {
            let room = room_entry.value();
            room.unsubscribe_track(subscriber_peer_id, track_id).await?;
            Ok(())
        } else {
            Err(anyhow!("Room not found"))
        }
    }

    /// Get room statistics
    pub async fn get_room_stats(&self, room_id: &RoomId) -> Result<RoomStats> {
        if let Some(room_entry) = self.rooms.get(room_id) {
            let room = room_entry.value();
            Ok(room.get_stats().await)
        } else {
            Ok(RoomStats::default())
        }
    }

    /// Get global manager statistics
    pub async fn get_stats(&self) -> ManagerStats {
        self.stats.read().await.clone()
    }

    /// Get configuration
    pub fn config(&self) -> &SfuConfig {
        &self.config
    }

    /// Get list of all active room IDs
    pub fn get_room_ids(&self) -> Vec<RoomId> {
        self.rooms.iter().map(|entry| entry.key().clone()).collect()
    }

    /// Get number of active rooms
    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }

    /// Cleanup empty rooms
    pub async fn cleanup_empty_rooms(&self) {
        let mut removed_count = 0;
        let mut room_ids_to_remove = Vec::new();

        // Collect empty room IDs
        for entry in self.rooms.iter() {
            let room_id = entry.key();
            let room = entry.value();

            if room.is_empty().await {
                room_ids_to_remove.push(room_id.clone());
            }
        }

        // Remove empty rooms
        for room_id in room_ids_to_remove {
            self.rooms.remove(&room_id);
            removed_count += 1;
            debug!(room_id = %room_id, "Removed empty room");
        }

        if removed_count > 0 {
            info!(
                removed_count,
                remaining_rooms = self.rooms.len(),
                "Cleaned up empty rooms"
            );
        }
    }

    /// Background task for periodic cleanup
    async fn cleanup_task(self: Arc<Self>) {
        let mut ticker = interval(Duration::from_secs(60));
        info!("Starting cleanup task (interval: 60s)");

        loop {
            ticker.tick().await;
            self.cleanup_empty_rooms().await;
        }
    }

    /// Background task for statistics collection
    async fn stats_collection_task(self: Arc<Self>) {
        let mut ticker = interval(Duration::from_secs(5));
        info!("Starting statistics collection task (interval: 5s)");

        loop {
            ticker.tick().await;
            if let Err(e) = self.update_global_stats().await {
                error!(error = %e, "Failed to update global statistics");
            }
        }
    }

    /// Update global statistics by aggregating room stats
    async fn update_global_stats(&self) -> Result<()> {
        let mut stats = ManagerStats {
            active_rooms: self.rooms.len(),
            ..Default::default()
        };

        for entry in self.rooms.iter() {
            let room = entry.value();
            let room_stats = room.get_stats().await;

            // Count peers
            stats.total_peers += room_stats.peer_count;

            // Count tracks
            stats.total_audio_tracks += room_stats.audio_tracks;
            stats.total_video_tracks += room_stats.video_tracks;

            // Aggregate bytes and packets
            stats.total_bytes_relayed += room_stats.bytes_relayed;
            stats.total_packets_relayed += room_stats.packets_relayed;

            // Count room modes
            match *room.mode.read().await {
                RoomMode::SFU => stats.sfu_mode_rooms += 1,
                RoomMode::P2P => stats.p2p_mode_rooms += 1,
            }
        }

        // Log before updating
        debug!(
            active_rooms = stats.active_rooms,
            total_peers = stats.total_peers,
            sfu_rooms = stats.sfu_mode_rooms,
            p2p_rooms = stats.p2p_mode_rooms,
            "Updated global statistics"
        );

        // Update shared stats
        *self.stats.write().await = stats;

        Ok(())
    }

    /// Force a specific room into SFU or P2P mode (for testing/debugging)
    pub async fn set_room_mode(&self, room_id: &RoomId, mode: RoomMode) -> Result<()> {
        if let Some(room_entry) = self.rooms.get(room_id) {
            let room = room_entry.value();
            let mut current_mode = room.mode.write().await;

            if *current_mode != mode {
                info!(
                    room_id = %room_id,
                    old_mode = ?*current_mode,
                    new_mode = ?mode,
                    "Forcing room mode change"
                );
                *current_mode = mode;
            }

            Ok(())
        } else {
            Err(anyhow!("Room not found"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_manager_creation() {
        let config = SfuConfig::default();
        let manager = SfuManager::new(config);
        assert_eq!(manager.room_count(), 0);
    }

    #[tokio::test]
    async fn test_room_lifecycle() {
        let config = SfuConfig::default();
        let manager = SfuManager::new(config);

        let room_id = RoomId::from("test-room");
        let room = manager.get_or_create_room(room_id.clone()).await.unwrap();
        assert_eq!(manager.room_count(), 1);

        // Getting the same room should return the existing one
        let room2 = manager.get_or_create_room(room_id.clone()).await.unwrap();
        assert_eq!(manager.room_count(), 1);
        assert!(Arc::ptr_eq(&room, &room2));
    }

    #[tokio::test]
    async fn test_peer_limit() {
        let mut config = SfuConfig::default();
        config.max_peers_per_room = 2;
        let manager = SfuManager::new(config);

        let room_id = RoomId::from("test-room");

        // Add peers up to limit
        manager.add_peer_to_room(room_id.clone(), PeerId::from("peer1")).await.unwrap();
        manager.add_peer_to_room(room_id.clone(), PeerId::from("peer2")).await.unwrap();

        // Adding one more should fail
        let result = manager.add_peer_to_room(room_id.clone(), PeerId::from("peer3")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_room_limit() {
        let mut config = SfuConfig::default();
        config.max_sfu_rooms = 2;
        let manager = SfuManager::new(config);

        // Create rooms up to limit
        manager.get_or_create_room(RoomId::from("room1")).await.unwrap();
        manager.get_or_create_room(RoomId::from("room2")).await.unwrap();

        // Creating one more should fail
        let result = manager.get_or_create_room(RoomId::from("room3")).await;
        assert!(result.is_err());
    }
}
