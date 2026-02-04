//! SFU Peer management

use crate::types::PeerId;
use serde::{Deserialize, Serialize};

pub struct SfuPeer {
    pub id: PeerId,
}

impl SfuPeer {
    pub fn new(id: PeerId) -> Self {
        Self { id }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PeerStats {
    pub packets_received: u64,
    pub bytes_received: u64,
}
