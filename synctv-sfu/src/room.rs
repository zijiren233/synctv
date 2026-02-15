//! SFU Room management
//!
//! This module handles complete room functionality including:
//! - P2P ↔ SFU mode switching based on peer count
//! - Media track publishing and subscription
//! - RTP packet forwarding between peers
//! - Bandwidth estimation and adaptive quality
//! - Room statistics and monitoring

use crate::config::SfuConfig;
use crate::network_monitor::NetworkQualityMonitor;
use crate::peer::SfuPeer;
use crate::track::MediaTrack;
use crate::types::{PeerId, RoomId, TrackId};
use anyhow::{anyhow, Result};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Room mode - P2P or SFU
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoomMode {
    /// Peer-to-peer mode (< threshold peers)
    P2P,
    /// SFU mode (>= threshold peers)
    SFU,
}

/// Room statistics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoomStats {
    /// Current number of peers in room
    pub peer_count: usize,
    /// Total peers that have joined (cumulative)
    pub total_peers_joined: u64,
    /// Number of mode switches
    pub mode_switches: u64,
    /// Number of audio tracks
    pub audio_tracks: usize,
    /// Number of video tracks
    pub video_tracks: usize,
    /// Total bytes relayed
    pub bytes_relayed: u64,
    /// Total packets relayed
    pub packets_relayed: u64,
}

/// SFU Room - manages peers and media routing
pub struct SfuRoom {
    /// Room ID
    pub id: RoomId,

    /// Current room mode
    pub mode: Arc<RwLock<RoomMode>>,

    /// Peers in the room (uses `DashMap` for concurrent access)
    pub peers: DashMap<PeerId, Arc<SfuPeer>>,

    /// Published tracks: `track_id` -> (`publisher_peer_id`, track)
    published_tracks: DashMap<TrackId, (PeerId, Arc<MediaTrack>)>,

    /// Track subscriptions: (`subscriber_peer_id`, `track_id`) -> ()
    subscriptions: DashMap<(PeerId, TrackId), ()>,

    /// Forwarding tasks for each track
    forwarding_tasks: DashMap<TrackId, tokio::task::JoinHandle<()>>,

    /// Configuration
    pub config: Arc<SfuConfig>,

    /// Statistics
    pub stats: Arc<RwLock<RoomStats>>,

    /// Atomic counters for hot-path stats (avoids write lock per packet)
    packets_relayed: Arc<AtomicU64>,
    bytes_relayed: Arc<AtomicU64>,

    /// Atomic peer counter for TOCTOU-safe capacity enforcement
    peer_count_atomic: Arc<AtomicUsize>,

    /// Network quality monitoring
    network_monitor: Arc<NetworkQualityMonitor>,
}

impl SfuRoom {
    /// Create a new SFU room
    pub fn new(id: RoomId, config: Arc<SfuConfig>) -> Self {
        info!(room_id = %id, "Creating new room");

        Self {
            id,
            mode: Arc::new(RwLock::new(RoomMode::P2P)),
            peers: DashMap::new(),
            published_tracks: DashMap::new(),
            subscriptions: DashMap::new(),
            forwarding_tasks: DashMap::new(),
            config,
            stats: Arc::new(RwLock::new(RoomStats::default())),
            packets_relayed: Arc::new(AtomicU64::new(0)),
            bytes_relayed: Arc::new(AtomicU64::new(0)),
            peer_count_atomic: Arc::new(AtomicUsize::new(0)),
            network_monitor: Arc::new(NetworkQualityMonitor::new()),
        }
    }

    /// Add a peer to the room.
    ///
    /// Uses an `AtomicUsize` counter for TOCTOU-safe capacity enforcement:
    /// `fetch_add` first to reserve a slot, then insert; `fetch_sub` on failure.
    pub async fn add_peer(&self, peer_id: PeerId, max_peers: usize) -> Result<Arc<SfuPeer>> {
        use dashmap::mapref::entry::Entry;

        // Reserve a slot atomically before touching DashMap
        if max_peers > 0 {
            let prev = self.peer_count_atomic.fetch_add(1, Ordering::SeqCst);
            if prev >= max_peers {
                self.peer_count_atomic.fetch_sub(1, Ordering::SeqCst);
                return Err(anyhow!("Maximum number of peers reached for this room"));
            }
        }

        let peer = match self.peers.entry(peer_id.clone()) {
            Entry::Occupied(_) => {
                // Peer already exists, release reserved slot
                if max_peers > 0 {
                    self.peer_count_atomic.fetch_sub(1, Ordering::SeqCst);
                }
                return Err(anyhow!("Peer already exists in room"));
            }
            Entry::Vacant(entry) => {
                let p = Arc::new(SfuPeer::new(peer_id.clone()));
                entry.insert(p.clone());
                p
            }
        };

        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.peer_count = self.peers.len();
            stats.total_peers_joined += 1;
        }

        info!(
            room_id = %self.id,
            peer_id = %peer_id,
            peer_count = self.peers.len(),
            "Added peer to room"
        );

        // Check if we need to switch modes
        self.check_mode_switch().await?;

        Ok(peer)
    }

    /// Remove a peer from the room
    pub async fn remove_peer(&self, peer_id: &PeerId) -> Result<()> {
        // Remove peer and decrement atomic counter
        if self.peers.remove(peer_id).is_some() {
            self.peer_count_atomic.fetch_sub(1, Ordering::SeqCst);
        }

        // Remove from network quality monitor
        self.network_monitor.remove_peer(peer_id);

        // Remove all tracks published by this peer
        let tracks_to_remove: Vec<TrackId> = self
            .published_tracks
            .iter()
            .filter(|entry| &entry.value().0 == peer_id)
            .map(|entry| entry.key().clone())
            .collect();

        for track_id in tracks_to_remove {
            self.remove_published_track(peer_id, &track_id).await?;
        }

        // Remove all subscriptions by this peer
        let subs_to_remove: Vec<(PeerId, TrackId)> = self
            .subscriptions
            .iter()
            .filter(|entry| &entry.key().0 == peer_id)
            .map(|entry| entry.key().clone())
            .collect();

        for (sub_peer_id, track_id) in subs_to_remove {
            self.unsubscribe_track(&sub_peer_id, &track_id).await?;
        }

        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.peer_count = self.peers.len();
        }

        info!(
            room_id = %self.id,
            peer_id = %peer_id,
            peer_count = self.peers.len(),
            "Removed peer from room"
        );

        // Check if we need to switch modes
        self.check_mode_switch().await?;

        Ok(())
    }

    /// Add a published track (holds peer reference to prevent TOCTOU)
    pub async fn add_published_track(
        &self,
        peer_id: &PeerId,
        track_id: TrackId,
        track: Arc<MediaTrack>,
    ) -> Result<()> {
        // Verify peer exists by holding a reference during insert
        let _peer_ref = self.peers
            .get(peer_id)
            .ok_or_else(|| anyhow!("Peer not found in room"))?;

        // Store track (safe: peer reference still held)
        self.published_tracks
            .insert(track_id.clone(), (peer_id.clone(), track.clone()));

        info!(
            room_id = %self.id,
            peer_id = %peer_id,
            track_id = %track_id,
            track_kind = ?track.kind,
            "Track published"
        );

        // In SFU mode, start forwarding this track
        let mode = *self.mode.read().await;
        if mode == RoomMode::SFU {
            self.start_track_forwarding(track_id, track, peer_id.clone())
                .await?;
        }

        Ok(())
    }

    /// Remove a published track
    pub async fn remove_published_track(
        &self,
        peer_id: &PeerId,
        track_id: &TrackId,
    ) -> Result<()> {
        // Stop forwarding task if it exists
        if let Some((_, task)) = self.forwarding_tasks.remove(track_id) {
            task.abort();
            debug!(
                room_id = %self.id,
                track_id = %track_id,
                "Stopped track forwarding task"
            );
        }

        // Remove track
        if let Some((_, (publisher_id, track))) = self.published_tracks.remove(track_id) {
            if &publisher_id != peer_id {
                warn!(
                    room_id = %self.id,
                    track_id = %track_id,
                    expected_publisher = %peer_id,
                    actual_publisher = %publisher_id,
                    "Track publisher mismatch"
                );
            }

            info!(
                room_id = %self.id,
                peer_id = %peer_id,
                track_id = %track_id,
                track_kind = ?track.kind,
                "Track unpublished"
            );
        }

        // Remove all subscriptions to this track
        let subs_to_remove: Vec<(PeerId, TrackId)> = self
            .subscriptions
            .iter()
            .filter(|entry| &entry.key().1 == track_id)
            .map(|entry| entry.key().clone())
            .collect();

        for (sub_peer_id, sub_track_id) in subs_to_remove {
            self.unsubscribe_track(&sub_peer_id, &sub_track_id).await?;
        }

        Ok(())
    }

    /// Subscribe to a track (holds references to prevent TOCTOU)
    pub async fn subscribe_track(
        &self,
        subscriber_peer_id: &PeerId,
        track_id: &TrackId,
    ) -> Result<()> {
        // Verify by holding references during insert
        let _peer_ref = self.peers
            .get(subscriber_peer_id)
            .ok_or_else(|| anyhow!("Subscriber peer not found in room"))?;
        let _track_ref = self.published_tracks
            .get(track_id)
            .ok_or_else(|| anyhow!("Track not found in room"))?;

        // Add subscription (safe: references still held)
        self.subscriptions
            .insert((subscriber_peer_id.clone(), track_id.clone()), ());

        info!(
            room_id = %self.id,
            subscriber = %subscriber_peer_id,
            track_id = %track_id,
            "Subscribed to track"
        );

        Ok(())
    }

    /// Unsubscribe from a track
    pub async fn unsubscribe_track(
        &self,
        subscriber_peer_id: &PeerId,
        track_id: &TrackId,
    ) -> Result<()> {
        self.subscriptions
            .remove(&(subscriber_peer_id.clone(), track_id.clone()));

        info!(
            room_id = %self.id,
            subscriber = %subscriber_peer_id,
            track_id = %track_id,
            "Unsubscribed from track"
        );

        Ok(())
    }

    /// Start forwarding a track to subscribers
    async fn start_track_forwarding(
        &self,
        track_id: TrackId,
        track: Arc<MediaTrack>,
        publisher_peer_id: PeerId,
    ) -> Result<()> {
        // Clone necessary data for the background task
        let track_id_clone = track_id.clone();
        let room_id = self.id.clone();
        let peers = self.peers.clone();
        let subscriptions = self.subscriptions.clone();
        let packets_relayed = Arc::clone(&self.packets_relayed);
        let bytes_relayed = Arc::clone(&self.bytes_relayed);

        // Spawn forwarding task
        let task = tokio::spawn(async move {
            if let Err(e) = Self::forward_track_packets(
                room_id,
                track_id_clone,
                track,
                peers,
                subscriptions,
                publisher_peer_id,
                packets_relayed,
                bytes_relayed,
            )
            .await
            {
                error!(error = %e, "Track forwarding task failed");
            }
        });

        self.forwarding_tasks.insert(track_id, task);

        Ok(())
    }

    /// Forward track packets to subscribers (background task)
    #[allow(clippy::too_many_arguments)]
    async fn forward_track_packets(
        room_id: RoomId,
        track_id: TrackId,
        track: Arc<MediaTrack>,
        peers: DashMap<PeerId, Arc<SfuPeer>>,
        subscriptions: DashMap<(PeerId, TrackId), ()>,
        publisher_peer_id: PeerId,
        packets_relayed: Arc<AtomicU64>,
        bytes_relayed: Arc<AtomicU64>,
    ) -> Result<()> {
        // Start reading packets from the track (uses interior mutability)
        let mut packet_rx = track.start_reading().await?;

        debug!(
            room_id = %room_id,
            track_id = %track_id,
            "Started forwarding track packets"
        );

        // Forward packets to subscribers
        while let Some(packet) = packet_rx.recv().await {
            // Find all subscribers for this track (excluding the publisher)
            let subscribers: Vec<PeerId> = subscriptions
                .iter()
                .filter(|entry| {
                    let (subscriber_id, sub_track_id) = entry.key();
                    sub_track_id == &track_id && subscriber_id != &publisher_peer_id
                })
                .map(|entry| entry.key().0.clone())
                .collect();

            // Forward to each subscriber via their packet channel
            let packet_size = packet.data.len();
            for subscriber_id in &subscribers {
                if let Some(peer) = peers.get(subscriber_id) {
                    if peer.try_forward_packet(&packet) {
                        peer.record_sent_bytes(packet_size);
                        packets_relayed.fetch_add(1, Ordering::Relaxed);
                        bytes_relayed.fetch_add(packet_size as u64, Ordering::Relaxed);
                    }
                }
            }
        }

        info!(
            room_id = %room_id,
            track_id = %track_id,
            "Stopped forwarding track packets"
        );

        Ok(())
    }

    /// Check if mode switch is needed and perform it
    async fn check_mode_switch(&self) -> Result<()> {
        let peer_count = self.peers.len();
        let threshold = self.config.sfu_threshold;
        let mut mode = self.mode.write().await;

        match *mode {
            RoomMode::P2P if peer_count >= threshold => {
                info!(
                    room_id = %self.id,
                    peer_count,
                    threshold,
                    "Switching from P2P to SFU mode"
                );
                *mode = RoomMode::SFU;

                // Update statistics
                let mut stats = self.stats.write().await;
                stats.mode_switches += 1;
                drop(stats);
                drop(mode);

                // Start forwarding all published tracks
                self.switch_to_sfu().await?;
            }
            RoomMode::SFU if peer_count < threshold => {
                info!(
                    room_id = %self.id,
                    peer_count,
                    threshold,
                    "Switching from SFU to P2P mode"
                );
                *mode = RoomMode::P2P;

                // Update statistics
                let mut stats = self.stats.write().await;
                stats.mode_switches += 1;
                drop(stats);
                drop(mode);

                // Stop forwarding all tracks
                self.switch_to_p2p().await?;
            }
            _ => {}
        }

        Ok(())
    }

    /// Switch to SFU mode - start forwarding all tracks
    pub(crate) async fn switch_to_sfu(&self) -> Result<()> {
        for entry in &self.published_tracks {
            let track_id = entry.key().clone();
            let (publisher_peer_id, track) = entry.value().clone();

            self.start_track_forwarding(track_id, track, publisher_peer_id)
                .await?;
        }

        info!(
            room_id = %self.id,
            track_count = self.published_tracks.len(),
            "Started forwarding for all tracks"
        );

        Ok(())
    }

    /// Switch to P2P mode - stop forwarding all tracks
    pub(crate) async fn switch_to_p2p(&self) -> Result<()> {
        // Stop all forwarding tasks
        let track_ids: Vec<TrackId> = self
            .forwarding_tasks
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        for track_id in track_ids {
            if let Some((_, task)) = self.forwarding_tasks.remove(&track_id) {
                task.abort();
            }
        }

        info!(
            room_id = %self.id,
            "Stopped all track forwarding tasks"
        );

        Ok(())
    }

    /// Get current peer count
    pub async fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Check if room is empty
    pub async fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Get room statistics
    pub async fn get_stats(&self) -> RoomStats {
        let mut stats = self.stats.read().await.clone();

        // Update current peer count
        stats.peer_count = self.peers.len();

        // Read hot-path counters from atomics
        stats.packets_relayed = self.packets_relayed.load(Ordering::Relaxed);
        stats.bytes_relayed = self.bytes_relayed.load(Ordering::Relaxed);

        // Count tracks by type
        stats.audio_tracks = 0;
        stats.video_tracks = 0;

        for entry in &self.published_tracks {
            let (_, track) = entry.value();
            if track.is_audio() {
                stats.audio_tracks += 1;
            } else if track.is_video() {
                stats.video_tracks += 1;
            }
        }

        stats
    }

    /// Get current room mode
    pub async fn get_mode(&self) -> RoomMode {
        *self.mode.read().await
    }

    /// Get list of all peer IDs
    #[must_use] 
    pub fn get_peer_ids(&self) -> Vec<PeerId> {
        self.peers.iter().map(|entry| entry.key().clone()).collect()
    }

    /// Get list of all published track IDs
    #[must_use]
    pub fn get_track_ids(&self) -> Vec<TrackId> {
        self.published_tracks
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get network quality stats for all peers in the room
    #[must_use] 
    pub fn get_network_quality_stats(&self) -> Vec<(String, crate::network_monitor::NetworkStats)> {
        self.network_monitor.get_all_stats()
    }

    /// Get network quality monitor (for advanced use)
    #[must_use]
    pub const fn network_monitor(&self) -> &Arc<NetworkQualityMonitor> {
        &self.network_monitor
    }
}

impl Drop for SfuRoom {
    fn drop(&mut self) {
        // Abort all forwarding tasks to prevent leaked spawned tasks
        for entry in &self.forwarding_tasks {
            entry.value().abort();
        }
        debug!(
            room_id = %self.id,
            task_count = self.forwarding_tasks.len(),
            "Room dropped, aborted all forwarding tasks"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_room_creation() {
        let config = Arc::new(SfuConfig::default());
        let room = SfuRoom::new(RoomId::from("test-room"), config);

        assert_eq!(room.get_mode().await, RoomMode::P2P);
        assert!(room.is_empty().await);
    }

    #[tokio::test]
    async fn test_peer_lifecycle() {
        let config = Arc::new(SfuConfig::default());
        let room = SfuRoom::new(RoomId::from("test-room"), config);

        // Add peer
        let peer_id = PeerId::from("peer1");
        room.add_peer(peer_id.clone(), 0).await.unwrap();
        assert_eq!(room.peer_count().await, 1);
        assert!(!room.is_empty().await);

        // Remove peer
        room.remove_peer(&peer_id).await.unwrap();
        assert_eq!(room.peer_count().await, 0);
        assert!(room.is_empty().await);
    }

    #[tokio::test]
    async fn test_mode_switch() {
        let mut config = SfuConfig::default();
        config.sfu_threshold = 3;
        let config = Arc::new(config);

        let room = SfuRoom::new(RoomId::from("test-room"), config);

        // Start in P2P mode
        assert_eq!(room.get_mode().await, RoomMode::P2P);

        // Add peers up to threshold
        room.add_peer(PeerId::from("peer1"), 0).await.unwrap();
        room.add_peer(PeerId::from("peer2"), 0).await.unwrap();
        assert_eq!(room.get_mode().await, RoomMode::P2P);

        // Should switch to SFU
        room.add_peer(PeerId::from("peer3"), 0).await.unwrap();
        assert_eq!(room.get_mode().await, RoomMode::SFU);

        // Should switch back to P2P
        room.remove_peer(&PeerId::from("peer3")).await.unwrap();
        assert_eq!(room.get_mode().await, RoomMode::P2P);
    }
}
