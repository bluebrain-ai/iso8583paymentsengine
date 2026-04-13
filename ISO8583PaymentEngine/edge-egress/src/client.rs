use crate::adapter::IsoAdapter;
use bytes::{Buf, BufMut, BytesMut};
use crossbeam::channel::Receiver;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use payment_proto;
use prost::Message;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use switch_core::context::TransactionContext;
use tokio::net::TcpStream;
use tokio_util::codec::{Decoder, Encoder, Framed};

pub struct Iso8583Codec;

impl Decoder for Iso8583Codec {
    type Item = BytesMut;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 2 {
            return Ok(None);
        }

        let mut length_bytes = [0u8; 2];
        length_bytes.copy_from_slice(&src[..2]);
        let length = u16::from_be_bytes(length_bytes) as usize;

        if src.len() < 2 + length {
            src.reserve(2 + length - src.len());
            return Ok(None);
        }

        src.advance(2);
        let payload = src.split_to(length);
        Ok(Some(payload))
    }
}

impl Encoder<BytesMut> for Iso8583Codec {
    type Error = io::Error;

    fn encode(&mut self, item: BytesMut, dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        let len = item.len() as u16;
        dst.reserve(2 + item.len());
        dst.put_u16(len);
        dst.extend_from_slice(&item);
        Ok(())
    }
}

pub struct UpstreamClient {
    pub host: String,
    pub rx: Receiver<TransactionContext>,
    pub saf_timeout_ms: u64,
    pub saf_db: sled::Db,
    pub bank_reply_tx: crossbeam::channel::Sender<payment_proto::Transaction>,
}

impl UpstreamClient {
    pub async fn run(self) {
        let db = self.saf_db.clone();
        let saf_db = db.clone();
        let saf_host = self.host.clone();
        let saf_timeout = self.saf_timeout_ms;
        let outbound_dialect = iso_dialect::DialectRouter::Connex(iso_dialect::ConnexDialect);

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(saf_timeout)).await;
                if let Ok(mut socket) = TcpStream::connect(&saf_host).await {
                    socket.set_nodelay(true).unwrap();
                    let mut keys_to_remove = Vec::new();

                    for row in saf_db.iter() {
                        if let Ok((key, val)) = row {
                            if let Ok(pb_tx) = payment_proto::Transaction::decode(&val[..]) {
                                let canonical_tx = switch_core::from_pb(&pb_tx);
                                let iso_bytes = iso_dialect::DialectRouter::Connex(
                                    iso_dialect::ConnexDialect,
                                )
                                .encode(&canonical_tx)
                                .unwrap_or_default();
                                let mut payload = BytesMut::new();
                                let mut encoder = Iso8583Codec;
                                let _ = encoder.encode(BytesMut::from(&iso_bytes[..]), &mut payload);

                                use tokio::io::AsyncWriteExt;
                                if socket.write_all(&payload).await.is_ok() {
                                    keys_to_remove.push(key);
                                }
                            }
                        }
                    }

                    for k in keys_to_remove {
                        let _ = saf_db.remove(k);
                    }
                }
            }
        });

        let host = self.host.clone();
        loop {
            if let Ok(socket) = TcpStream::connect(&host).await {
                socket.set_nodelay(true).unwrap();
                let framed = Framed::new(socket, Iso8583Codec);
                let (mut sink, mut stream) = framed.split();

                let reader_dialect = iso_dialect::DialectRouter::Connex(iso_dialect::ConnexDialect);
                let reply_tx = self.bank_reply_tx.clone();
                // Read from socket task
                let read_task = tokio::spawn(async move {
                    while let Some(result) = stream.next().await {
                        if let Ok(payload) = result {
                            if let Ok(canonical_tx) = reader_dialect.decode(&payload) {
                                let pb_tx = switch_core::to_pb(&canonical_tx);
                                let _ = reply_tx.send(pb_tx);
                            } else if let Err(e) = reader_dialect.decode(&payload) {
                                println!("[EGRESS DEBUG] Failed to decode bank reply! Error: {:?}", e);
                            }
                        } else {
                            println!("[EGRESS DEBUG] Stream next error or closed!");
                            break; // Connection closed or error
                        }
                    }
                });

                let (sink_tx, mut sink_rx) = tokio::sync::mpsc::channel::<BytesMut>(100_000);
                
                tokio::spawn(async move {
                    while let Some(payload) = sink_rx.recv().await {
                        if sink.feed(payload).await.is_err() { break; }
                        while let Ok(extra) = sink_rx.try_recv() {
                            if sink.feed(extra).await.is_err() { break; }
                        }
                        let _ = sink.flush().await;
                    }
                });

                loop {
                    match self.rx.try_recv() {
                        Ok(context) => {
                            let sink_tx_clone = sink_tx.clone();
                            let db_clone = db.clone();
                            let outbound_dialect_clone = iso_dialect::DialectRouter::Connex(iso_dialect::ConnexDialect);
                            
                            tokio::spawn(async move {
                                let iso_bytes = outbound_dialect_clone
                                    .encode(&context.transaction)
                                    .unwrap_or_default();
                                let payload = BytesMut::from(&iso_bytes[..]);

                                if sink_tx_clone.send(payload).await.is_err() {
                                    let mti = String::from_utf8_lossy(&context.transaction.mti);
                                    if mti == "0420" || mti == "0220" {
                                        let rrn = context.transaction.rrn.0.clone();
                                        let stan = context.transaction.stan.0.clone();
                                        let key = format!("{}-{}", stan, rrn);
                                        let val = switch_core::to_pb(&context.transaction).encode_to_vec();
                                        let _ = db_clone.insert(key.as_bytes(), val);
                                        let _ = db_clone.flush();
                                    }
                                }
                            });
                        }
                        Err(crossbeam::channel::TryRecvError::Empty) => {
                            tokio::task::yield_now().await;
                            if read_task.is_finished() {
                                break; // Socket broken implicitly
                            }
                        }
                        Err(crossbeam::channel::TryRecvError::Disconnected) => {
                            break; 
                        }
                    }
                }
            } else {
                match self.rx.try_recv() {
                    Ok(context) => {
                        let mti = String::from_utf8_lossy(&context.transaction.mti);
                        if mti == "0420" || mti == "0220" {
                            let stan = context.transaction.stan.0.clone();
                            let rrn = context.transaction.rrn.0.clone();
                            let key = format!("{}-{}", stan, rrn);
                            let val = switch_core::to_pb(&context.transaction).encode_to_vec();
                            let _ = db.insert(key.as_bytes(), val);
                            let _ = db.flush();
                        }
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}
