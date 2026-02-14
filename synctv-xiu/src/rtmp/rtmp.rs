use crate::streamhub::define::StreamHubEventSender;

use super::auth::AuthCallback;
use super::session::server_session;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::Error;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// Default max concurrent RTMP connections.
const DEFAULT_MAX_CONNECTIONS: usize = 1000;

/// Grace period for existing sessions to complete after shutdown signal.
const SHUTDOWN_GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(10);

pub struct RtmpServer {
    address: String,
    event_producer: StreamHubEventSender,
    gop_num: usize,
    auth: Option<Arc<dyn AuthCallback>>,
    max_connections: usize,
    shutdown_token: CancellationToken,
}

impl RtmpServer {
    #[must_use]
    pub fn new(
        address: String,
        event_producer: StreamHubEventSender,
        gop_num: usize,
        auth: Option<Arc<dyn AuthCallback>>,
    ) -> Self {
        Self {
            address,
            event_producer,
            gop_num,
            auth,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            shutdown_token: CancellationToken::new(),
        }
    }

    /// Set an external cancellation token. The server's internal shutdown token
    /// becomes a child of `parent`, so cancelling `parent` will also shut down
    /// this RTMP server and all its sessions.
    #[must_use]
    pub fn with_cancellation_token(mut self, parent: CancellationToken) -> Self {
        self.shutdown_token = parent.child_token();
        self
    }

    /// Returns a `CancellationToken` that can be used to signal graceful shutdown.
    #[must_use]
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown_token.clone()
    }

    pub async fn run(&mut self) -> Result<(), Error> {
        let socket_addr: SocketAddr = self.address.parse().map_err(|e| {
            Error::new(std::io::ErrorKind::InvalidInput, format!("invalid address '{}': {}", self.address, e))
        })?;
        let listener = TcpListener::bind(&socket_addr).await?;
        let active_connections = Arc::new(AtomicUsize::new(0));
        let session_tracker = tokio_util::task::TaskTracker::new();

        tracing::info!("Rtmp server listening on tcp://{socket_addr} (max_connections: {})", self.max_connections);
        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    let (tcp_stream, remote_addr) = accept_result?;

                    // Atomically reserve a slot first, then check. This avoids the
                    // TOCTOU race between load() and fetch_add().
                    let prev = active_connections.fetch_add(1, Ordering::Relaxed);
                    if prev >= self.max_connections {
                        active_connections.fetch_sub(1, Ordering::Relaxed);
                        tracing::warn!(
                            "RTMP connection rejected from {}: at capacity ({}/{})",
                            remote_addr, prev, self.max_connections,
                        );
                        drop(tcp_stream);
                        continue;
                    }
                    let conn_counter = active_connections.clone();

                    let mut session = server_session::ServerSession::new(
                        tcp_stream,
                        self.event_producer.clone(),
                        self.gop_num,
                        self.auth.clone(),
                    );
                    session_tracker.spawn(async move {
                        if let Err(err) = session.run().await {
                            tracing::info!(
                                "session run error: session_type: {}, app_name: {}, stream_name: {}, err: {}",
                                session.common.session_type,
                                session.app_name,
                                session.stream_name,
                                err
                            );
                        }
                        conn_counter.fetch_sub(1, Ordering::Relaxed);
                    });
                }
                () = self.shutdown_token.cancelled() => {
                    tracing::info!("RTMP server shutting down gracefully, waiting for {} active sessions",
                        active_connections.load(Ordering::Relaxed));
                    break;
                }
            }
        }

        // Stop accepting new connections; wait for existing sessions with timeout
        session_tracker.close();
        if tokio::time::timeout(SHUTDOWN_GRACE_PERIOD, session_tracker.wait()).await.is_err() {
            tracing::warn!(
                "RTMP shutdown grace period expired, {} sessions still active",
                active_connections.load(Ordering::Relaxed)
            );
        }

        tracing::info!("RTMP server shutdown complete");
        Ok(())
    }
}
