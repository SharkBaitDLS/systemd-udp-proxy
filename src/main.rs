use std::{
    collections::HashMap,
    io::{self, ErrorKind},
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};

use clap::Parser;
use listenfd::ListenFd;
use log::warn;
use primary_tasks::{rx_loop, tx_loop};
use session::{Session, SessionReply, SessionSource};
use tokio::{
    net::UdpSocket,
    sync::{
        mpsc::{self, UnboundedSender},
        RwLock,
    },
};

mod error_util;
mod primary_tasks;
mod session;

type SessionChannel = UnboundedSender<Vec<u8>>;
type SessionCache = HashMap<SessionSource, (SessionChannel, Arc<Session>)>;

#[derive(Parser, Debug)]
struct Args {
    /// The destination port to proxy traffic to
    #[arg(short, long)]
    destination_port: u16,
    /// The address to bind to to send proxy traffic from
    #[arg(short, long, default_value_t = IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)))]
    source_address: IpAddr,
    /// The destination address to send proxy traffic to
    #[arg(short, long, default_value_t = IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)))]
    destination_address: IpAddr,
    /// How many seconds sessions should be cached before expiring
    #[arg(short, long, default_value_t = 10)]
    session_timeout: u64,
}

const MAX_UDP_PACKET_SIZE: u16 = u16::MAX;

#[tokio::main]
async fn main() -> io::Result<()> {
    let args = Args::parse();
    env_logger::init();

    let mut listen_fd = ListenFd::from_env();
    if listen_fd.len() == 0 {
        return Err(io::Error::new(
            ErrorKind::AddrNotAvailable,
            "No socket was passed by systemd",
        ));
    }

    if listen_fd.len() > 1 {
        warn!("More than one socket was passed by systemd, but only the first will be used.");
    }
    let std_source_socket = listen_fd.take_udp_socket(0)?.ok_or(io::Error::new(
        io::ErrorKind::AddrInUse,
        "Socket was unexpectedly consumed before use",
    ))?;
    // The socket must be set to non-blocking mode in order to be used in async contexts
    std_source_socket.set_nonblocking(true)?;

    let source_socket = Arc::new(UdpSocket::from_std(std_source_socket)?);
    let (reply_channel_tx, reply_channel_rx) = mpsc::unbounded_channel::<SessionReply>();

    let rx_task = tokio::spawn(rx_loop(
        args,
        reply_channel_tx,
        source_socket.clone(),
        Arc::new(RwLock::new(HashMap::new())),
    ));

    let tx_task = tokio::spawn(tx_loop(reply_channel_rx, source_socket.clone()));

    rx_task.await??;
    tx_task.await??;
    Ok(())
}
