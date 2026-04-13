use crate::codec::Iso8583Codec;
use crossbeam::channel::Sender;
use futures::{SinkExt, StreamExt};
use payment_proto::Transaction;
use switch_core::context::TransactionContext;
use tokio::net::TcpListener;
use tokio_util::codec::Framed;
use tracing::Instrument;
use std::sync::atomic::{AtomicUsize, Ordering};
use iso_dialect::{DialectRouter, Base24Dialect};

static CURRENT_CONNS: AtomicUsize = AtomicUsize::new(0);

struct ConnectionGuard;
impl ConnectionGuard {
    fn new() -> Self {
        let count = CURRENT_CONNS.fetch_add(1, Ordering::SeqCst) + 1;
        metrics::gauge!("active_tcp_connections", count as f64);
        Self
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        let count = CURRENT_CONNS.fetch_sub(1, Ordering::SeqCst) - 1;
        metrics::gauge!("active_tcp_connections", count as f64);
    }
}
pub async fn run_server(listener: TcpListener, core_tx: Sender<TransactionContext>) {
    loop {
        match listener.accept().await {
            Ok((socket, addr)) => {
                socket.set_nodelay(true).unwrap();
                let tx = core_tx.clone();
                let span = tracing::info_span!("atm_connection", peer_addr = %addr);
                
                tokio::spawn(async move {
                    let _guard = ConnectionGuard::new();
                    let framed = Framed::new(socket, Iso8583Codec);
                    let (mut sink, mut stream) = framed.split();
                    
                    let (reply_tx, mut reply_rx) = tokio::sync::mpsc::channel::<bytes::BytesMut>(10000);
                    
                    tokio::spawn(async move {
                        while let Some(msg) = reply_rx.recv().await {
                            let _ = sink.feed(msg).await;
                            while let Ok(extra_msg) = reply_rx.try_recv() {
                                let _ = sink.feed(extra_msg).await;
                            }
                            let _ = sink.flush().await;
                        }
                    });

                    let (drop_notifier_tx, _) = tokio::sync::broadcast::channel::<()>(1);

                    while let Some(result) = stream.next().await {
                        match result {
                            Ok(payload) => {
                                let dialect_decode = DialectRouter::Base24(Base24Dialect);
                                if let Ok(canonical_tx) = dialect_decode.decode(&payload) {
                                    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                                    
                                    let ctx = TransactionContext {
                                        transaction: canonical_tx.clone(),
                                        responder: Some(resp_tx),
                                    };
                                    
                                    let tx_chan = tx.clone();
                                    let reply_tx_clone = reply_tx.clone();
                                    let mut drop_rx = drop_notifier_tx.subscribe();
                                    let panic_tx_chan = tx.clone();

                                    tokio::spawn(async move {
                                        if tx_chan.send(ctx).is_ok() {
                                            tokio::select! {
                                                resp_res = resp_rx => {
                                                    if let Ok(response_tx) = resp_res {
                                                        let dialect_encode = DialectRouter::Base24(Base24Dialect);
                                                        if let Ok(encoded) = dialect_encode.encode(&response_tx) {
                                                            let _ = reply_tx_clone.send(bytes::BytesMut::from(&encoded[..])).await;
                                                        }
                                                    }
                                                }
                                                _ = drop_rx.recv() => {
                                                    // CRITICAL FIX: The physical TCP socket dropped before the Bank replied!
                                                    // Trigger an instantaneous 0420 Reversal to the upstream bank immediately.
                                                    let mut rev_tx = canonical_tx;
                                                    rev_tx.mti = bytes::Bytes::from_static(b"0420");
                                                    let rev_ctx = TransactionContext {
                                                        transaction: rev_tx,
                                                        responder: None,
                                                    };
                                                    let _ = panic_tx_chan.send(rev_ctx);
                                                }
                                            }
                                        }
                                    });
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    
                    // Trigger instantaneous teardown of all pending async tasks on this socket
                    let _ = drop_notifier_tx.send(());
                }.instrument(span));
            }
            Err(_) => break,
        }
    }
}
