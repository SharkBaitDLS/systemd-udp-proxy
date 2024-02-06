use std::{
    fmt::Display,
    io::{self, ErrorKind},
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use log::info;
use tokio::{
    net::UdpSocket,
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    time::timeout,
};

use crate::{
    error_util::{handle_io_error, ErrorAction},
    Args, MAX_UDP_PACKET_SIZE,
};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct SessionSource {
    pub address: IpAddr,
    pub port: u16,
}

#[derive(Debug)]
pub struct Session {
    source: SessionSource,
    destination: Arc<UdpSocket>,
}

#[derive(Debug)]
pub struct SessionReply {
    pub source: SessionSource,
    pub data: Vec<u8>,
}

/// Wrapper around a [UdpSocket] that handles the boiler plate of establishing a connection to the appropriate
/// destination based on the provided [Args] and provides helper functions for TX/RX loop tasks.
impl Session {
    /// Establish a new session that binds to a [Args::source_address] and establishes
    /// a connection to [Args::destination_address] and [Args::destination_port]. Returns an [io::Error]
    /// if the connection fails to establish.
    pub async fn new(args: &Args, source: SessionSource) -> io::Result<Self> {
        // Let the OS assign us an available port
        let destination = Arc::new(UdpSocket::bind((args.source_address, 0)).await?);
        // Connect to the destination
        destination
            .connect((args.destination_address, args.destination_port))
            .await?;

        Ok(Session {
            source,
            destination,
        })
    }

    /// Loop indefinitely waiting for messages on `source_channel` and send them to the [Self::destination].
    /// Ends the loop if no message is recieved for `session_timeout` seconds.
    pub async fn tx_loop(
        &self,
        mut source_channel: UnboundedReceiver<Vec<u8>>,
        session_timeout: u64,
    ) -> io::Result<()> {
        while let Ok(Some(data)) =
            timeout(Duration::from_secs(session_timeout), source_channel.recv()).await
        {
            match self.destination.send(&data).await {
                Ok(_) => {}
                Err(err) => match err.kind() {
                    // Destination service hasn't started yet
                    ErrorKind::ConnectionRefused => {}
                    _ => return Err(err),
                },
            }
        }
        info!(
            "Closing tx session for {} due to {session_timeout} second timeout",
            self.source
        );
        Ok(())
    }

    /// Loop indefinitely waiting for replies from the [Self::destination] and forwards them to the `reply_channel`.
    /// Ends the loop if no reply is recieved for `session_timeout` seconds.
    pub async fn rx_loop(
        &self,
        reply_channel: Arc<UnboundedSender<SessionReply>>,
        session_timeout: u64,
    ) -> io::Result<()> {
        let mut buf = Vec::with_capacity(MAX_UDP_PACKET_SIZE.into());
        loop {
            match timeout(
                Duration::from_secs(session_timeout),
                self.destination.recv_buf(&mut buf),
            )
            .await
            {
                Ok(result) => {
                    if let Err(err) = result {
                        match handle_io_error(err) {
                            ErrorAction::Continue => {}
                            ErrorAction::Terminate(cause) => {
                                return Err(cause);
                            }
                        }
                    }
                }
                Err(_) => {
                    info!(
                        "Closing rx session for {} due to {session_timeout} second timeout",
                        self.source
                    );
                    return Ok(());
                }
            };

            if reply_channel
                .send(SessionReply {
                    source: self.source,
                    data: buf.clone(),
                })
                .is_err()
            {
                return Err(io::Error::new(
                    ErrorKind::ConnectionAborted,
                    "Primary tx task has stopped listening, dropping reply as the proxy will soon terminate"
                ));
            };
        }
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
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.write_fmt(format_args!("{}:{}", self.address, self.port))
    }
}
