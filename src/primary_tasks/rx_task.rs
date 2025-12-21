use std::{
    collections::{HashMap, hash_map::Entry},
    io,
    sync::Arc,
};

use log::{error, info};
use tokio::{
    net::UdpSocket,
    sync::{
        RwLock,
        mpsc::{self, UnboundedSender},
    },
};

use crate::{
    ProxyConfig,
    error_util::{ErrorAction, handle_io_error},
    session::{Session, SessionReply, SessionSource},
};

type SessionChannel = UnboundedSender<Vec<u8>>;
type SessionCache = HashMap<SessionSource, (SessionChannel, Arc<Session>)>;

/// Loops infinitely over the `rx_socket` to recieve traffic from the original source of the proxy.
///
/// For each unique [`std::net::SocketAddr`] that sends traffic to `rx_socket`, a [`Session`] is created and
/// tx/rx loop tasks are spawned to proxy traffic for that session to and from the destination. If a [`Session`]
/// does not recieve traffic for [`ProxyConfig::session_timeout`] seconds, it will close its tasks and a new one will
/// be created if any traffic resumes from it.
///
/// If a packet arrives after a Session's channel has closed but before the session is removed
/// from the cache, that packet will be dropped and the session will be cleaned up. Subsequent
/// packets from the same source will trigger creation of a new session.
pub async fn rx_task(
    config: ProxyConfig,
    reply_channel_tx: UnboundedSender<SessionReply>,
    rx_socket: Arc<UdpSocket>,
) -> io::Result<()> {
    let shared_reply_channel = Arc::new(reply_channel_tx);
    let sessions = Arc::new(RwLock::new(SessionCache::new()));

    loop {
        let mut buf = Vec::with_capacity(config.max_packet_size);
        match rx_socket.recv_buf_from(&mut buf).await {
            Err(err) => match handle_io_error(err) {
                ErrorAction::Terminate(err) => return Err(err),
                ErrorAction::Continue => {}
            },
            Ok((_len, source)) => {
                let mut session_cache = sessions.write().await;
                let session_channel_tx = match session_cache.entry(source.into()) {
                    Entry::Vacant(entry) => {
                        info!("Creating a new session for {source}");
                        let session = match Session::new(&config, source.into()).await {
                            Ok(created_session) => Arc::new(created_session),
                            Err(err) => {
                                error!("Failed to create a session for {}: {:?}", source, err);
                                continue;
                            }
                        };

                        let (tx, rx) = mpsc::unbounded_channel();

                        let tx_session = session.clone();
                        let tx_session_cache = sessions.clone();
                        tokio::spawn(async move {
                            if let Err(err) = tx_session.tx_loop(rx, config.session_timeout).await {
                                error!("TX error for {}: {:?}", source, err);
                            }
                            tx_session_cache.write().await.remove(&source.into());
                        });

                        let rx_session = session.clone();
                        let rx_session_cache = sessions.clone();
                        let rx_reply_channel = shared_reply_channel.clone();
                        tokio::spawn(async move {
                            if let Err(err) = rx_session
                                .rx_loop(
                                    rx_reply_channel,
                                    config.session_timeout,
                                    config.max_packet_size,
                                )
                                .await
                            {
                                error!("RX error for {}: {:?}", source, err);
                            }
                            rx_session_cache.write().await.remove(&source.into());
                        });

                        let (inserted_tx, _session) = entry.insert((tx, session));
                        inserted_tx
                    }
                    Entry::Occupied(entry) => {
                        let (existing_tx, _session) = entry.into_mut();
                        existing_tx
                    }
                };

                if session_channel_tx.send(buf).is_err() {
                    error!(
                        "Dropped packet for {} because its proxy session is closed",
                        source
                    );
                    sessions.write().await.remove(&source.into());
                }
            }
        }
    }
}
