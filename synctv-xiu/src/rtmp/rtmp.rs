use crate::streamhub::define::StreamHubEventSender;

use super::auth::AuthCallback;
use super::session::server_session;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::Error;
use tokio::net::TcpListener;

/// Default max concurrent RTMP connections.
const DEFAULT_MAX_CONNECTIONS: usize = 1000;

pub struct RtmpServer {
    address: String,
    event_producer: StreamHubEventSender,
    gop_num: usize,
    auth: Option<Arc<dyn AuthCallback>>,
    max_connections: usize,
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
        }
    }

    pub async fn run(&mut self) -> Result<(), Error> {
        let socket_addr: SocketAddr = self.address.parse().map_err(|e| {
            Error::new(std::io::ErrorKind::InvalidInput, format!("invalid address '{}': {}", self.address, e))
        })?;
        let listener = TcpListener::bind(&socket_addr).await?;
        let active_connections = Arc::new(AtomicUsize::new(0));

        log::info!("Rtmp server listening on tcp://{socket_addr} (max_connections: {})", self.max_connections);
        loop {
            let (tcp_stream, remote_addr) = listener.accept().await?;

            let current = active_connections.load(Ordering::Relaxed);
            if current >= self.max_connections {
                log::warn!(
                    "RTMP connection rejected from {}: at capacity ({}/{})",
                    remote_addr, current, self.max_connections,
                );
                drop(tcp_stream);
                continue;
            }

            active_connections.fetch_add(1, Ordering::Relaxed);
            let conn_counter = active_connections.clone();

            let mut session = server_session::ServerSession::new(
                tcp_stream,
                self.event_producer.clone(),
                self.gop_num,
                self.auth.clone(),
            );
            tokio::spawn(async move {
                if let Err(err) = session.run().await {
                    log::info!(
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
    }
}
