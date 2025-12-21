use std::{
    fmt::{self, Display, Formatter},
    io::{self, ErrorKind},
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use log::{info, warn};
use tokio::{
    net::UdpSocket,
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    time::{Instant, timeout},
};

use crate::{
    ProxyConfig,
    error_util::{ErrorAction, handle_io_error},
    telemetry::ProxyMetrics,
};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct SessionSource {
    pub address: IpAddr,
    pub port: u16,
}

/// Wrapper around a [`UdpSocket`] that handles the boiler plate of establishing a connection to the appropriate
/// backend destination. It retains the original [`SessionSource`] of the traffic it will be proxying
/// so that replies from the backend can be properly routed back.
pub struct Session {
    /// The source that this session is receiving traffic from
    source: SessionSource,
    /// The socket that this session is using to communicate with the destination
    destination_socket: Arc<UdpSocket>,
    metrics: Arc<ProxyMetrics>,
    start: Instant,
}

pub struct SessionReply {
    pub source: SessionSource,
    pub data: Vec<u8>,
}

impl SessionReply {
    pub fn new(source: SessionSource, data: Vec<u8>) -> Self {
        SessionReply { source, data }
    }
}

impl Session {
    /// Establish a new session that binds to an [`ProxyConfig::source_address`] and establishes
    /// a connection to [`ProxyConfig::destination_address`] on [`ProxyConfig::destination_port`].
    /// Returns an [`io::Error`] if the connection fails to establish.
    pub async fn new(
        config: &ProxyConfig,
        source: SessionSource,
        metrics: Arc<ProxyMetrics>,
    ) -> io::Result<Self> {
        // Let the OS assign us an available port
        let destination_socket = Arc::new(UdpSocket::bind((config.source_address, 0)).await?);
        // Connect to the destination
        destination_socket
            .connect((config.destination_address, config.destination_port))
            .await?;

        Ok(Session {
            source,
            destination_socket,
            metrics,
            start: Instant::now(),
        })
    }

    /// Loops indefinitely waiting for messages on `source_channel` and send them to the [`Self::destination_socket`].
    /// Ends the loop if no message is recieved for `session_timeout` seconds or any unrecoverable
    /// error occurs in transmission.
    pub async fn tx_loop(
        &self,
        mut source_channel: UnboundedReceiver<Vec<u8>>,
        session_timeout: u64,
    ) -> io::Result<()> {
        let duration = Duration::from_secs(session_timeout);
        while let Ok(Some(data)) = timeout(duration, source_channel.recv()).await {
            match self.destination_socket.send(&data).await {
                Ok(_) => {}
                Err(err) => match err.kind() {
                    // Destination service hasn't started yet
                    ErrorKind::ConnectionRefused => {
                        warn!("Destination service refused connection");
                    }
                    _ => match handle_io_error(err) {
                        ErrorAction::Terminate(cause) => return Err(cause),
                        ErrorAction::Continue => {}
                    },
                },
            }
        }
        info!("Closing tx session for {}", self.source);
        Ok(())
    }

    /// Loops indefinitely waiting for replies from the [`Self::destination_socket`] and forwards them to
    /// the `reply_channel`. Ends the loop if no reply is recieved for `session_timeout` seconds.
    pub async fn rx_loop(
        &self,
        reply_channel: Arc<UnboundedSender<SessionReply>>,
        session_timeout: u64,
        max_packet_size: usize,
    ) -> io::Result<()> {
        let duration = Duration::from_secs(session_timeout);
        loop {
            let mut buf = Vec::with_capacity(max_packet_size);
            match timeout(duration, self.destination_socket.recv_buf(&mut buf)).await {
                Ok(result) => {
                    if let Err(err) = result {
                        match handle_io_error(err) {
                            ErrorAction::Terminate(cause) => return Err(cause),
                            ErrorAction::Continue => {}
                        }
                    }
                }
                Err(_timeout_exceeded) => {
                    info!("Closing rx session for {}", self.source);
                    return Ok(());
                }
            }

            if reply_channel
                .send(SessionReply::new(self.source, buf))
                .is_err()
            {
                return Err(io::Error::new(
                    ErrorKind::ConnectionAborted,
                    "Primary tx task has stopped listening, dropping reply as the proxy will soon terminate",
                ));
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.metrics
            .record_session_duration(Instant::now().duration_since(self.start).as_secs_f64());
    }
}

impl From<SocketAddr> for SessionSource {
    fn from(value: SocketAddr) -> Self {
        SessionSource {
            address: value.ip(),
            port: value.port(),
        }
    }
}

impl Display for SessionSource {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> fmt::Result {
        fmt.write_fmt(format_args!("{}:{}", self.address, self.port))
    }
}
