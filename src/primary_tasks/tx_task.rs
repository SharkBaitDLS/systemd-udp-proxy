use std::{io, sync::Arc};

use tokio::{net::UdpSocket, sync::mpsc::UnboundedReceiver};

use crate::{
    error_util::{ErrorAction, handle_io_error},
    session::SessionReply,
    telemetry::{NetworkDirection, Peer, ProxyMetrics},
};

/// Loops infinitely over the `reply_channel_rx` to forward traffic from the destination of the proxy.
///
/// This task recives channel messages representing responses from the proxy destination over
/// `reply_channel_tx` from [`crate::session::Session`]s and sends them back to the original
/// source via `tx_socket`.
pub async fn tx_task(
    mut reply_channel_rx: UnboundedReceiver<SessionReply>,
    tx_socket: Arc<UdpSocket>,
    metrics: Arc<ProxyMetrics>,
) -> io::Result<()> {
    let dir = NetworkDirection::Transmit;
    let peer = Peer::Client;

    while let Some(reply) = reply_channel_rx.recv().await {
        match tx_socket
            .send_to(&reply.data, (reply.source.address, reply.source.port))
            .await
        {
            Ok(_) => {
                metrics.count_packet(&dir, &peer);
                metrics.count_bytes(&dir, &peer, reply.data.len() as u64);
            }
            Err(err) => {
                metrics.count_dropped_packet(&peer);
                match handle_io_error(err) {
                    ErrorAction::Terminate(err) => {
                        metrics.count_io_error(&dir, &peer, false);
                        return Err::<(), io::Error>(err);
                    }
                    ErrorAction::Continue => {
                        metrics.count_io_error(&dir, &peer, true);
                    }
                }
            }
        }
    }
    Ok(())
}
