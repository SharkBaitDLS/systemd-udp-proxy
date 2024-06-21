#[cfg(not(debug_assertions))]
use std::io::ErrorKind;
use std::{
    io,
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};

use clap::Parser;
#[cfg(not(debug_assertions))]
use listenfd::ListenFd;
#[cfg(not(debug_assertions))]
use log::warn;
use primary_tasks::{rx_loop, tx_loop};
use session::SessionReply;
use tokio::{net::UdpSocket, sync::mpsc};

mod error_util;
mod log_config;
mod primary_tasks;
mod session;

#[derive(Parser, Debug)]
struct Args {
    /// The destination port to proxy traffic to
    #[arg(short = 'p', long)]
    destination_port: u16,
    /// The address to bind to to send proxy traffic from
    #[arg(short = 's', long, default_value_t = IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)))]
    source_address: IpAddr,
    /// The destination address to send proxy traffic to
    #[arg(short = 'd', long, default_value_t = IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)))]
    destination_address: IpAddr,
    /// How many seconds sessions should be cached before expiring
    #[arg(short = 't', long, default_value_t = 60)]
    session_timeout: u64,
}

const MAX_UDP_PACKET_SIZE: u16 = u16::MAX;

#[tokio::main]
async fn main() -> io::Result<()> {
    log_config::init();
    let args = Args::parse();

    #[cfg(debug_assertions)]
    let std_source_socket = std::net::UdpSocket::bind((Ipv4Addr::new(127, 0, 0, 1), 8123))?;
    #[cfg(not(debug_assertions))]
    let std_source_socket = {
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
        listen_fd.take_udp_socket(0)?.ok_or(io::Error::new(
            io::ErrorKind::AddrInUse,
            "Socket was unexpectedly consumed before use",
        ))?
    };
    // The socket must be set to non-blocking mode in order to be used in async contexts
    std_source_socket.set_nonblocking(true)?;

    let source_socket = Arc::new(UdpSocket::from_std(std_source_socket)?);
    let (reply_channel_tx, reply_channel_rx) = mpsc::unbounded_channel::<SessionReply>();

    let rx_task = tokio::spawn(rx_loop(args, reply_channel_tx, source_socket.clone()));
    let tx_task = tokio::spawn(tx_loop(reply_channel_rx, source_socket.clone()));

    rx_task.await??;
    tx_task.await??;
    Ok(())
}
