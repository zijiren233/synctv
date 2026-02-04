//! Built-in STUN Server
//!
//! A lightweight STUN (Session Traversal Utilities for NAT) server implementation.
//! Helps WebRTC clients discover their public IP addresses and ports for P2P connectivity.
//!
//! ## STUN Protocol Overview
//! - RFC 8489: Session Traversal Utilities for NAT (STUN)
//! - Binding Request: Client asks "what's my public IP:port?"
//! - Binding Response: Server responds with XOR-MAPPED-ADDRESS
//! - Runs on UDP port 3478 (default)
//!
//! ## Implementation
//! Uses the mature `stun_codec` crate for protocol handling, avoiding manual byte
//! manipulation and reducing the risk of protocol errors.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{debug, error, info, warn};

use bytecodec::{DecodeExt, EncodeExt};
use stun_codec::{Message, MessageClass, MessageDecoder, MessageEncoder, TransactionId};
use stun_codec::rfc5389::attributes::{Software, XorMappedAddress};
use stun_codec::rfc5389::{Attribute, methods};

// Convenience constant for BINDING method
const BINDING_METHOD: stun_codec::Method = methods::BINDING;

/// STUN server configuration
#[derive(Debug, Clone)]
pub struct StunServerConfig {
    /// Bind address (e.g., "0.0.0.0:3478")
    pub bind_addr: String,
    /// Maximum UDP packet size (typically 1500 bytes for MTU)
    pub max_packet_size: usize,
}

impl Default for StunServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:3478".to_string(),
            max_packet_size: 1500,
        }
    }
}

/// STUN server metrics
#[derive(Debug, Clone)]
pub struct StunMetrics {
    /// Total requests received
    pub total_requests: u64,
    /// Total responses sent
    pub total_responses: u64,
    /// Total errors
    pub total_errors: u64,
}

/// Built-in STUN server for NAT traversal
pub struct StunServer {
    config: StunServerConfig,
    socket: Arc<UdpSocket>,
    metrics: Arc<StunMetricsInner>,
}

struct StunMetricsInner {
    total_requests: AtomicU64,
    total_responses: AtomicU64,
    total_errors: AtomicU64,
}

impl StunServer {
    /// Create and start a new STUN server
    pub async fn start(config: StunServerConfig) -> anyhow::Result<Arc<Self>> {
        let socket = UdpSocket::bind(&config.bind_addr).await?;
        let local_addr = socket.local_addr()?;

        info!(
            bind_addr = %local_addr,
            "STUN server started"
        );

        let server = Arc::new(Self {
            config,
            socket: Arc::new(socket),
            metrics: Arc::new(StunMetricsInner {
                total_requests: AtomicU64::new(0),
                total_responses: AtomicU64::new(0),
                total_errors: AtomicU64::new(0),
            }),
        });

        // Spawn background task to handle requests
        let server_clone = Arc::clone(&server);
        tokio::spawn(async move {
            server_clone.run().await;
        });

        Ok(server)
    }

    /// Main server loop
    async fn run(&self) {
        let mut buf = vec![0u8; self.config.max_packet_size];

        loop {
            match self.socket.recv_from(&mut buf).await {
                Ok((len, peer_addr)) => {
                    self.metrics.total_requests.fetch_add(1, Ordering::Relaxed);

                    debug!(
                        peer_addr = %peer_addr,
                        len = len,
                        "Received STUN request"
                    );

                    // Handle request in background to avoid blocking
                    let data = buf[..len].to_vec();
                    let socket = Arc::clone(&self.socket);
                    let metrics = Arc::clone(&self.metrics);

                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_request(&socket, &data, peer_addr, &metrics).await {
                            error!(
                                peer_addr = %peer_addr,
                                error = %e,
                                "Failed to handle STUN request"
                            );
                            metrics.total_errors.fetch_add(1, Ordering::Relaxed);
                        } else {
                            metrics.total_responses.fetch_add(1, Ordering::Relaxed);
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "Failed to receive UDP packet");
                    self.metrics.total_errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Handle a single STUN request using the stun_codec crate
    async fn handle_request(
        socket: &UdpSocket,
        data: &[u8],
        peer_addr: SocketAddr,
        _metrics: &StunMetricsInner,
    ) -> anyhow::Result<()> {
        // Decode STUN message using stun_codec
        let mut decoder = MessageDecoder::<Attribute>::new();
        let decoded = decoder.decode_from_bytes(data)
            .map_err(|e| anyhow::anyhow!("Failed to decode STUN message: {e}"))?;

        // Handle potential broken message
        let request = match decoded {
            Ok(msg) => msg,
            Err(broken) => {
                warn!(
                    peer_addr = %peer_addr,
                    "Received broken STUN message: {:?}", broken
                );
                return Err(anyhow::anyhow!("Broken STUN message"));
            }
        };

        // Only handle Binding Requests
        if request.method() != BINDING_METHOD || request.class() != MessageClass::Request {
            debug!(
                peer_addr = %peer_addr,
                method = ?request.method(),
                class = ?request.class(),
                "Ignoring non-Binding STUN request"
            );
            return Ok(());
        }

        // Build Binding Success Response
        let response = Self::build_binding_response(&request, peer_addr)?;

        // Encode response
        let mut encoder = MessageEncoder::new();
        let response_bytes = encoder.encode_into_bytes(response)
            .map_err(|e| anyhow::anyhow!("Failed to encode STUN response: {e}"))?;

        // Send response
        socket.send_to(&response_bytes, peer_addr).await?;

        debug!(
            peer_addr = %peer_addr,
            response_len = response_bytes.len(),
            "Sent STUN Binding Response"
        );

        Ok(())
    }

    /// Build STUN Binding Success Response with XOR-MAPPED-ADDRESS
    fn build_binding_response(
        request: &Message<Attribute>,
        peer_addr: SocketAddr,
    ) -> anyhow::Result<Message<Attribute>> {
        // Create response message
        let mut response = Message::new(
            MessageClass::SuccessResponse,
            BINDING_METHOD,
            request.transaction_id(),
        );

        // Add XOR-MAPPED-ADDRESS attribute (RFC 5389 Section 15.2)
        // This tells the client their public IP:port as seen by the server
        response.add_attribute(Attribute::XorMappedAddress(XorMappedAddress::new(peer_addr)));

        // Add SOFTWARE attribute (optional but recommended)
        response.add_attribute(Attribute::Software(Software::new(
            "SyncTV STUN Server v1.0".to_string()
        )?));

        Ok(response)
    }

    /// Get current metrics
    pub fn metrics(&self) -> StunMetrics {
        StunMetrics {
            total_requests: self.metrics.total_requests.load(Ordering::Relaxed),
            total_responses: self.metrics.total_responses.load(Ordering::Relaxed),
            total_errors: self.metrics.total_errors.load(Ordering::Relaxed),
        }
    }

    /// Get the local bind address
    pub fn local_addr(&self) -> anyhow::Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stun_server_start() {
        let config = StunServerConfig {
            bind_addr: "127.0.0.1:0".to_string(), // Use random port
            max_packet_size: 1500,
        };

        let server = StunServer::start(config).await.unwrap();
        let addr = server.local_addr().unwrap();

        assert!(addr.port() > 0);

        // Give server time to initialize
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn test_stun_binding_request() {
        // Start server on random port
        let config = StunServerConfig {
            bind_addr: "127.0.0.1:0".to_string(),
            max_packet_size: 1500,
        };

        let server = StunServer::start(config).await.unwrap();
        let server_addr = server.local_addr().unwrap();

        // Give server time to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Create client socket
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        // Create STUN Binding Request
        let transaction_id = TransactionId::new([0u8; 12]);
        let request = Message::<Attribute>::new(
            MessageClass::Request,
            BINDING_METHOD,
            transaction_id,
        );

        // Encode request
        let mut encoder = MessageEncoder::new();
        let request_bytes = encoder.encode_into_bytes(request.clone()).unwrap();

        // Send request
        client.send_to(&request_bytes, server_addr).await.unwrap();

        // Receive response with timeout
        let mut buf = vec![0u8; 1500];
        let (len, _) = tokio::time::timeout(
            tokio::time::Duration::from_secs(2),
            client.recv_from(&mut buf),
        )
        .await
        .expect("Timeout waiting for response")
        .unwrap();

        // Decode response
        let mut decoder = MessageDecoder::<Attribute>::new();
        let response = decoder.decode_from_bytes(&buf[..len]).unwrap();

        // Verify response
        assert_eq!(response.class(), MessageClass::SuccessResponse);
        assert_eq!(response.method(), BINDING_METHOD);
        assert_eq!(response.transaction_id(), transaction_id);

        // Verify XOR-MAPPED-ADDRESS is present
        let has_xor_mapped = response.attributes().iter().any(|attr| {
            matches!(attr, Attribute::XorMappedAddress(_))
        });
        assert!(has_xor_mapped, "Response should contain XOR-MAPPED-ADDRESS");

        // Check metrics
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        let metrics = server.metrics();
        assert!(metrics.total_requests >= 1);
        assert!(metrics.total_responses >= 1);
    }

    #[tokio::test]
    async fn test_build_binding_response() {
        let transaction_id = TransactionId::new([0u8; 12]);
        let request = Message::<Attribute>::new(
            MessageClass::Request,
            BINDING_METHOD,
            transaction_id,
        );

        let peer_addr: SocketAddr = "192.168.1.100:12345".parse().unwrap();

        let response = StunServer::build_binding_response(&request, peer_addr).unwrap();

        // Verify response properties
        assert_eq!(response.class(), MessageClass::SuccessResponse);
        assert_eq!(response.method(), BINDING_METHOD);
        assert_eq!(response.transaction_id(), transaction_id);

        // Verify XOR-MAPPED-ADDRESS attribute
        let xor_mapped = response
            .attributes()
            .iter()
            .find_map(|attr| {
                if let Attribute::XorMappedAddress(addr) = attr {
                    Some(addr)
                } else {
                    None
                }
            })
            .expect("Response should contain XOR-MAPPED-ADDRESS");

        assert_eq!(xor_mapped.address(), peer_addr);

        // Verify SOFTWARE attribute
        let has_software = response
            .attributes()
            .iter()
            .any(|attr| matches!(attr, Attribute::Software(_)));
        assert!(has_software, "Response should contain SOFTWARE attribute");
    }

    #[tokio::test]
    async fn test_metrics() {
        let config = StunServerConfig {
            bind_addr: "127.0.0.1:0".to_string(),
            max_packet_size: 1500,
        };

        let server = StunServer::start(config).await.unwrap();

        // Initial metrics should be zero
        let metrics = server.metrics();
        assert_eq!(metrics.total_requests, 0);
        assert_eq!(metrics.total_responses, 0);
        assert_eq!(metrics.total_errors, 0);
    }
}
