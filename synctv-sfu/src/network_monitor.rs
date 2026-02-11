//! Network Quality Monitoring
//!
//! Collects and analyzes network statistics per peer to calculate quality scores
//! and drive adaptive quality decisions.
//!
//! ## Quality Score
//! - 5: Excellent (RTT < 50ms, loss < 1%)
//! - 4: Good (RTT < 100ms, loss < 3%)
//! - 3: Fair (RTT < 200ms, loss < 5%)
//! - 2: Poor (RTT < 300ms, loss < 10%)
//! - 1: Bad (RTT >= 300ms or loss >= 10%)
//! - 0: Unknown / No data
//!
//! ## Adaptive Quality
//! - High packet loss (>10%): Switch to Low quality
//! - Low bandwidth (<500kbps): Reduce to 15fps
//! - Severe packet loss (>20%): Audio-only mode

use crate::peer::PeerStats;
use crate::types::PeerId;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Network statistics for a single peer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStats {
    /// Round-trip time in milliseconds
    pub rtt_ms: u32,

    /// Packet loss rate (0.0 - 1.0)
    pub packet_loss_rate: f32,

    /// Jitter in milliseconds
    pub jitter_ms: u32,

    /// Available bandwidth in kbps
    pub available_bandwidth_kbps: u32,

    /// Quality score (0-5)
    pub quality_score: u8,

    /// Suggested quality action
    pub quality_action: QualityAction,
}

impl Default for NetworkStats {
    fn default() -> Self {
        Self {
            rtt_ms: 0,
            packet_loss_rate: 0.0,
            jitter_ms: 0,
            available_bandwidth_kbps: 1000,
            quality_score: 0,
            quality_action: QualityAction::None,
        }
    }
}

/// Suggested quality action based on network conditions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityAction {
    /// No action needed
    None,
    /// Switch to low quality layer
    ReduceQuality,
    /// Reduce framerate (e.g., 30fps -> 15fps)
    ReduceFramerate,
    /// Disable video, audio only
    AudioOnly,
}

/// Peer monitoring entry with history
struct PeerMonitorEntry {
    /// Current computed stats
    stats: NetworkStats,

    /// Rolling window of RTT samples (last 30 seconds)
    rtt_samples: Vec<(Instant, u32)>,

    /// Rolling window of packet loss samples
    loss_samples: Vec<(Instant, f32)>,

    /// Last update time
    last_updated: Instant,
}

impl PeerMonitorEntry {
    fn new() -> Self {
        Self {
            stats: NetworkStats::default(),
            rtt_samples: Vec::new(),
            loss_samples: Vec::new(),
            last_updated: Instant::now(),
        }
    }
}

/// Network quality monitor for all peers in the SFU
pub struct NetworkQualityMonitor {
    /// Per-peer monitoring data
    peers: DashMap<PeerId, PeerMonitorEntry>,

    /// Sample window duration (default 30 seconds)
    window_duration: Duration,
}

impl NetworkQualityMonitor {
    /// Create a new network quality monitor
    #[must_use] 
    pub fn new() -> Self {
        Self {
            peers: DashMap::new(),
            window_duration: Duration::from_secs(30),
        }
    }

    /// Update stats for a peer from SFU `PeerStats` and bandwidth estimate
    pub fn update_peer_stats(
        &self,
        peer_id: &PeerId,
        peer_stats: &PeerStats,
        bandwidth_kbps: u32,
    ) {
        let now = Instant::now();

        let mut entry = self
            .peers
            .entry(peer_id.clone())
            .or_insert_with(PeerMonitorEntry::new);

        let entry = entry.value_mut();

        // Calculate packet loss rate from cumulative stats
        let total_packets = peer_stats.packets_received + peer_stats.packet_loss_count;
        let loss_rate = if total_packets > 0 {
            peer_stats.packet_loss_count as f32 / total_packets as f32
        } else {
            0.0
        };

        // Add samples
        entry.rtt_samples.push((now, 0)); // RTT from RTCP when available
        entry.loss_samples.push((now, loss_rate));

        // Prune old samples outside the window
        let cutoff = now.checked_sub(self.window_duration).unwrap();
        entry.rtt_samples.retain(|(t, _)| *t >= cutoff);
        entry.loss_samples.retain(|(t, _)| *t >= cutoff);

        // Compute averaged stats
        let avg_rtt = if entry.rtt_samples.is_empty() {
            0
        } else {
            let sum: u32 = entry.rtt_samples.iter().map(|(_, v)| v).sum();
            sum / entry.rtt_samples.len() as u32
        };

        let avg_loss = if entry.loss_samples.is_empty() {
            0.0
        } else {
            let sum: f32 = entry.loss_samples.iter().map(|(_, v)| v).sum();
            sum / entry.loss_samples.len() as f32
        };

        // Calculate jitter from RTT variance
        let jitter = if entry.rtt_samples.len() > 1 {
            let mean = f64::from(avg_rtt);
            let variance: f64 = entry
                .rtt_samples
                .iter()
                .map(|(_, v)| {
                    let diff = f64::from(*v) - mean;
                    diff * diff
                })
                .sum::<f64>()
                / entry.rtt_samples.len() as f64;
            variance.sqrt() as u32
        } else {
            0
        };

        // Calculate quality score
        let quality_score = calculate_quality_score(avg_rtt, avg_loss, bandwidth_kbps);

        // Determine quality action
        let quality_action = determine_quality_action(avg_loss, bandwidth_kbps);

        // Update entry
        entry.stats = NetworkStats {
            rtt_ms: avg_rtt,
            packet_loss_rate: avg_loss,
            jitter_ms: jitter,
            available_bandwidth_kbps: bandwidth_kbps,
            quality_score,
            quality_action,
        };
        entry.last_updated = now;
    }

    /// Update RTT for a peer (from RTCP reports)
    pub fn update_rtt(&self, peer_id: &PeerId, rtt_ms: u32) {
        let now = Instant::now();

        let mut entry = self
            .peers
            .entry(peer_id.clone())
            .or_insert_with(PeerMonitorEntry::new);

        let entry = entry.value_mut();
        entry.rtt_samples.push((now, rtt_ms));

        // Prune old samples
        let cutoff = now.checked_sub(self.window_duration).unwrap();
        entry.rtt_samples.retain(|(t, _)| *t >= cutoff);
    }

    /// Get network stats for a specific peer
    #[must_use] 
    pub fn get_peer_stats(&self, peer_id: &PeerId) -> Option<NetworkStats> {
        self.peers.get(peer_id).map(|entry| entry.stats.clone())
    }

    /// Get network stats for all peers
    #[must_use] 
    pub fn get_all_stats(&self) -> Vec<(String, NetworkStats)> {
        self.peers
            .iter()
            .map(|entry| (entry.key().as_str().to_string(), entry.value().stats.clone()))
            .collect()
    }

    /// Remove a peer from monitoring
    pub fn remove_peer(&self, peer_id: &PeerId) {
        self.peers.remove(peer_id);
    }

    /// Get the number of monitored peers
    #[must_use] 
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }
}

impl Default for NetworkQualityMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculate quality score (0-5) based on network conditions
fn calculate_quality_score(rtt_ms: u32, packet_loss_rate: f32, bandwidth_kbps: u32) -> u8 {
    let mut score: i8 = 5;

    // RTT penalties
    if rtt_ms >= 300 {
        score -= 2;
    } else if rtt_ms >= 200 {
        score -= 1;
    } else if rtt_ms >= 100 {
        // slight penalty is already baked into the thresholds
    }

    // Packet loss penalties
    if packet_loss_rate >= 0.15 {
        score -= 3;
    } else if packet_loss_rate >= 0.10 {
        score -= 2;
    } else if packet_loss_rate >= 0.05 {
        score -= 1;
    } else if packet_loss_rate >= 0.03 {
        // Minor loss, no penalty beyond existing threshold
    }

    // Bandwidth penalties
    if bandwidth_kbps < 300 {
        score -= 2;
    } else if bandwidth_kbps < 500 {
        score -= 1;
    }

    score.clamp(0, 5) as u8
}

/// Determine quality action based on network conditions
fn determine_quality_action(packet_loss_rate: f32, bandwidth_kbps: u32) -> QualityAction {
    if packet_loss_rate > 0.20 {
        // Severe loss: audio only
        QualityAction::AudioOnly
    } else if packet_loss_rate > 0.10 {
        // High loss: reduce to low quality
        QualityAction::ReduceQuality
    } else if bandwidth_kbps < 500 {
        // Low bandwidth: reduce framerate
        QualityAction::ReduceFramerate
    } else {
        QualityAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quality_score_excellent() {
        let score = calculate_quality_score(30, 0.005, 3000);
        assert_eq!(score, 5);
    }

    #[test]
    fn test_quality_score_good() {
        let score = calculate_quality_score(80, 0.02, 2000);
        assert_eq!(score, 5); // RTT < 100, loss < 3%, bandwidth good
    }

    #[test]
    fn test_quality_score_fair() {
        let score = calculate_quality_score(150, 0.06, 1500);
        assert_eq!(score, 4); // loss penalty -1
    }

    #[test]
    fn test_quality_score_poor() {
        let score = calculate_quality_score(250, 0.12, 800);
        assert_eq!(score, 2); // RTT -1, loss -2
    }

    #[test]
    fn test_quality_score_bad() {
        let score = calculate_quality_score(400, 0.25, 200);
        assert_eq!(score, 0); // RTT -2, loss -3, bandwidth -2 -> clamped to 0
    }

    #[test]
    fn test_quality_action_none() {
        let action = determine_quality_action(0.02, 2000);
        assert_eq!(action, QualityAction::None);
    }

    #[test]
    fn test_quality_action_reduce_framerate() {
        let action = determine_quality_action(0.02, 400);
        assert_eq!(action, QualityAction::ReduceFramerate);
    }

    #[test]
    fn test_quality_action_reduce_quality() {
        let action = determine_quality_action(0.15, 2000);
        assert_eq!(action, QualityAction::ReduceQuality);
    }

    #[test]
    fn test_quality_action_audio_only() {
        let action = determine_quality_action(0.25, 2000);
        assert_eq!(action, QualityAction::AudioOnly);
    }

    #[test]
    fn test_monitor_update_and_get() {
        let monitor = NetworkQualityMonitor::new();
        let peer_id = PeerId::from("peer1");

        let stats = PeerStats {
            packets_received: 1000,
            bytes_received: 500_000,
            packets_sent: 900,
            bytes_sent: 450_000,
            packet_loss_count: 10,
            bandwidth_kbps: 2000,
        };

        monitor.update_peer_stats(&peer_id, &stats, 2000);

        let result = monitor.get_peer_stats(&peer_id);
        assert!(result.is_some());
        let net_stats = result.unwrap();
        assert!(net_stats.quality_score > 0);
        assert_eq!(net_stats.available_bandwidth_kbps, 2000);
    }

    #[test]
    fn test_monitor_remove_peer() {
        let monitor = NetworkQualityMonitor::new();
        let peer_id = PeerId::from("peer1");

        let stats = PeerStats::default();
        monitor.update_peer_stats(&peer_id, &stats, 1000);
        assert_eq!(monitor.peer_count(), 1);

        monitor.remove_peer(&peer_id);
        assert_eq!(monitor.peer_count(), 0);
        assert!(monitor.get_peer_stats(&peer_id).is_none());
    }

    #[test]
    fn test_monitor_all_stats() {
        let monitor = NetworkQualityMonitor::new();

        let stats = PeerStats::default();
        monitor.update_peer_stats(&PeerId::from("peer1"), &stats, 2000);
        monitor.update_peer_stats(&PeerId::from("peer2"), &stats, 1000);

        let all_stats = monitor.get_all_stats();
        assert_eq!(all_stats.len(), 2);
    }
}
