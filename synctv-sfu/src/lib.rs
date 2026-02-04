//! `SyncTV` SFU (Selective Forwarding Unit)
//!
//! This module implements a WebRTC SFU for handling large rooms (10+ participants).
//! The SFU receives media streams from all participants and selectively forwards
//! them to other participants, reducing client-side bandwidth requirements.
//!
//! ## Architecture
//!
//! - **`SfuRoom`**: Manages a single room with multiple peers
//! - **`SfuPeer`**: Represents a single participant in an SFU room
//! - **`MediaTrack`**: Represents an audio or video track
//! - **`QualityLayer`**: Simulcast quality selection (high/medium/low)
//!
//! ## Features
//!
//! - Selective forwarding of media streams
//! - Simulcast support (multiple quality layers)
//! - Automatic mode switching (P2P ↔ SFU based on room size)
//! - Bandwidth estimation and adaptive quality
//! - Per-peer subscription management
//!
//! ## Usage
//!
//! ```rust,ignore
//! use synctv_sfu::{SfuManager, SfuConfig};
//!
//! let config = SfuConfig {
//!     sfu_threshold: 5,
//!     max_sfu_rooms: 10,
//!     max_peers_per_room: 20,
//!     enable_simulcast: true,
//! };
//!
//! let manager = SfuManager::new(config);
//! let room = manager.create_room("room_id").await?;
//! let peer = room.add_peer("user_id", peer_connection).await?;
//! ```

mod config;
mod manager;
mod peer;
mod room;
mod track;
mod types;

pub use config::SfuConfig;
pub use manager::SfuManager;
pub use peer::{SfuPeer, PeerStats};
pub use room::{SfuRoom, RoomMode, RoomStats};
pub use track::{MediaTrack, QualityLayer, TrackKind};
pub use types::{PeerId, RoomId, TrackId};
