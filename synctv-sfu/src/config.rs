//! SFU Configuration

use serde::{Deserialize, Serialize};

/// SFU configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SfuConfig {
    /// Room size threshold to automatically switch to SFU mode
    pub sfu_threshold: usize,
    /// Maximum number of concurrent SFU rooms (0 = unlimited)
    pub max_sfu_rooms: usize,
    /// Maximum peers per SFU room
    pub max_peers_per_room: usize,
    /// Enable Simulcast (multiple quality layers)
    pub enable_simulcast: bool,
    /// Simulcast layers to use
    pub simulcast_layers: Vec<String>,
    /// Maximum bitrate per peer (kbps, 0 = unlimited)
    pub max_bitrate_per_peer: u32,
    /// Enable bandwidth estimation
    pub enable_bandwidth_estimation: bool,
}

impl Default for SfuConfig {
    fn default() -> Self {
        Self {
            sfu_threshold: 5,
            max_sfu_rooms: 0,
            max_peers_per_room: 50,
            enable_simulcast: true,
            simulcast_layers: vec![
                "high".to_string(),
                "medium".to_string(),
                "low".to_string(),
            ],
            max_bitrate_per_peer: 0,
            enable_bandwidth_estimation: true,
        }
    }
}
