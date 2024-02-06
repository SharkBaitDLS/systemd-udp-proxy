use std::{io, sync::Arc};

use tokio::{net::UdpSocket, sync::mpsc::UnboundedReceiver};

use crate::{
    error_util::{handle_io_error, ErrorAction},
    session::SessionReply,
};

/// Loops infinitely over the `tx_socket` to forward traffic from the destination of the proxy.
///
/// This task recives channel messages representing responses from the proxy destination over
/// `reply_channel_tx` from [crate::session::Session]s and sends them back to the original
/// source via `tx_socket`.
pub async fn tx_loop(
    mut reply_channel_rx: UnboundedReceiver<SessionReply>,
    tx_socket: Arc<UdpSocket>,
) -> io::Result<()> {
    while let Some(reply) = reply_channel_rx.recv().await {
        match tx_socket
            .send_to(&reply.data, (reply.source.address, reply.source.port))
            .await
        {
            Ok(_) => continue,
            Err(err) => match handle_io_error(err) {
                ErrorAction::Terminate(err) => return Err::<(), io::Error>(err),
                ErrorAction::Continue => continue,
            },
        }
    }
    Ok(())
}
