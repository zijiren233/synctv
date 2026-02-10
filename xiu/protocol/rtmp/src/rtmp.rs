use streamhub::define::StreamHubEventSender;

use super::auth::AuthCallback;
use super::session::server_session;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::Error;
use tokio::net::TcpListener;

pub struct RtmpServer {
    address: String,
    event_producer: StreamHubEventSender,
    gop_num: usize,
    auth: Option<Arc<dyn AuthCallback>>,
}

impl RtmpServer {
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
        }
    }

    pub async fn run(&mut self) -> Result<(), Error> {
        let socket_addr: &SocketAddr = &self.address.parse().unwrap();
        let listener = TcpListener::bind(socket_addr).await?;

        log::info!("Rtmp server listening on tcp://{}", socket_addr);
        loop {
            let (tcp_stream, _) = listener.accept().await?;

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
            });
        }
    }
}
