// RTMP server integration using xiu library
//
// The actual RTMP server implementation is in:
// - /synctv-stream/src/streaming/rtmp.rs
//
// This module provides configuration types for convenience.

use std::net::SocketAddr;

/// RTMP server configuration
#[derive(Debug, Clone)]
pub struct RtmpConfig {
    /// RTMP listen address (default: 0.0.0.0:1935)
    pub listen_addr: SocketAddr,
    /// Maximum number of concurrent streams
    pub max_streams: usize,
    /// Chunk size for RTMP (default: 4096)
    pub chunk_size: u32,
    /// Enable GOP cache (number of GOPs to cache)
    pub gop_num: usize,
}

impl Default for RtmpConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:1935".parse().expect("valid default RTMP listen address"),
            max_streams: 50,
            chunk_size: 4096,
            gop_num: 2, // Cache last 2 GOPs
        }
    }
}

/// RTMP server
///
/// This is a convenience wrapper around the actual RTMP server
/// implemented in streaming/rtmp.rs.
///
/// The actual server uses xiu's `rtmp::rtmp::RtmpServer` which:
/// 1. Handles RTMP handshake and protocol
/// 2. Integrates with `StreamHub` for event handling
/// 3. Calls `PublisherManager` for Redis registration
/// 4. Supports GOP cache for fast viewer startup
pub struct RtmpServer {
    config: RtmpConfig,
}

impl RtmpServer {
    /// Create a new RTMP server configuration
    #[must_use] 
    pub const fn new(config: RtmpConfig) -> Self {
        Self { config }
    }

    /// Get the RTMP server configuration
    #[must_use] 
    pub const fn config(&self) -> &RtmpConfig {
        &self.config
    }

    /// Build the server
    ///
    /// This returns the configuration needed to create the actual
    /// server in streaming/rtmp.rs
    #[must_use] 
    pub fn build(&self) -> (String, usize) {
        (self.config.listen_addr.to_string(), self.config.gop_num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rtmp_config_default() {
        let config = RtmpConfig::default();
        assert_eq!(config.listen_addr.port(), 1935);
        assert_eq!(config.max_streams, 50);
        assert_eq!(config.chunk_size, 4096);
        assert_eq!(config.gop_num, 2);
    }

    #[test]
    fn test_rtmp_server_build() {
        let config = RtmpConfig {
            listen_addr: "127.0.0.1:1935".parse().unwrap(),
            ..Default::default()
        };
        let server = RtmpServer::new(config);
        let (addr, gop_num) = server.build();
        assert_eq!(addr, "127.0.0.1:1935".to_string());
        assert_eq!(gop_num, 2);
    }
}
