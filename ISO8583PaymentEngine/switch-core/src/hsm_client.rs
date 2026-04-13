use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use payment_proto::thales::{ThalesCodec, ThalesMessage};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_util::codec::{FramedRead, FramedWrite};

#[derive(Debug, Clone)]
pub struct HsmError(pub String);

#[derive(Clone)]
pub struct HsmClient {
    tx: mpsc::Sender<ThalesMessage>,
    correlation_map: Arc<DashMap<String, oneshot::Sender<ThalesMessage>>>,
    counter: Arc<AtomicU32>,
}

impl HsmClient {
    pub async fn connect(addr: &str) -> Result<Self, std::io::Error> {
        let stream = TcpStream::connect(addr).await?;
        let (rx, tx) = stream.into_split();
        let mut framed_read = FramedRead::new(rx, ThalesCodec);
        let mut framed_write = FramedWrite::new(tx, ThalesCodec);

        let (mpsc_tx, mut mpsc_rx) = mpsc::channel::<ThalesMessage>(100_000);
        let correlation_map = Arc::new(DashMap::<String, oneshot::Sender<ThalesMessage>>::new());
        let map_clone = Arc::clone(&correlation_map);

        // Writer task
        tokio::spawn(async move {
            while let Some(msg) = mpsc_rx.recv().await {
                if let Err(e) = framed_write.send(msg).await {
                    tracing::error!("HSM Writer error: {:?}", e);
                    break;
                }
            }
        });

        // Reader task
        tokio::spawn(async move {
            while let Some(result) = framed_read.next().await {
                match result {
                    Ok(msg) => {
                        let header = msg.header.clone();
                        if let Some((_, sender)) = map_clone.remove(&header) {
                            let _ = sender.send(msg);
                        } else {
                            tracing::warn!("Received HSM response for unknown correlation ID: {}", header);
                        }
                    }
                    Err(e) => {
                        tracing::error!("HSM Reader error: {:?}", e);
                        break;
                    }
                }
            }
        });

        Ok(Self {
            tx: mpsc_tx,
            correlation_map,
            counter: Arc::new(AtomicU32::new(0)),
        })
    }

    fn next_header(&self) -> String {
        let val = self.counter.fetch_add(1, Ordering::SeqCst) % 10000;
        format!("{:04}", val)
    }

    pub async fn verify_pin(&self, pin_block: &str, pan: &str, pvv: &str) -> Result<bool, HsmError> {
        let header = self.next_header();

        let (resp_tx, resp_rx) = oneshot::channel();
        self.correlation_map.insert(header.clone(), resp_tx);

        let formatted_pan = if pan.len() >= 12 {
            &pan[pan.len() - 13..pan.len() - 1]
        } else {
            pan
        };

        let mut payload = String::new();
        // AWK + PIN Block + Format + PAN + PVK + PVV
        payload.push_str("AWK1234567890123");
        payload.push_str(pin_block);
        payload.push_str("01"); // Format
        payload.push_str(formatted_pan);
        payload.push_str("PVK1234567890123");
        payload.push_str(pvv);

        let msg = ThalesMessage {
            header: header.clone(),
            command: "DA".to_string(),
            payload,
        };

        if self.tx.send(msg).await.is_err() {
            self.correlation_map.remove(&header);
            return Err(HsmError("HSM Dispatch Channel Closed".to_string()));
        }

        match tokio::time::timeout(std::time::Duration::from_secs(2), resp_rx).await {
            Ok(Ok(resp)) => {
                if resp.command == "DB" && resp.payload.starts_with("00") {
                    Ok(true)
                } else if resp.command == "DB" && resp.payload.starts_with("05") {
                     Ok(false)
                } else {
                    Err(HsmError("Invalid Verify PIN DB HSM Response".to_string()))
                }
            }
            Ok(Err(_)) => Err(HsmError("Receiver dropped".to_string())),
            Err(_) => {
                self.correlation_map.remove(&header);
                Err(HsmError("Timeout awaiting HSM DB response".to_string()))
            }
        }
    }

    pub async fn translate_pin(&self, pin_block: &str, pan: &str) -> Result<String, HsmError> {
        let header = self.next_header();

        let (resp_tx, resp_rx) = oneshot::channel();
        self.correlation_map.insert(header.clone(), resp_tx);

        let formatted_pan = if pan.len() >= 12 {
            &pan[pan.len() - 13..pan.len() - 1]
        } else {
            pan
        };

        let mut payload = String::new();
        payload.push_str("AWK1234567890123");
        payload.push_str("ZWK1234567890123");
        payload.push_str("12"); // max pin len
        payload.push_str(pin_block);
        payload.push_str("01"); // format
        payload.push_str(formatted_pan);

        let msg = ThalesMessage {
            header: header.clone(),
            command: "CA".to_string(),
            payload,
        };

        if self.tx.send(msg).await.is_err() {
            self.correlation_map.remove(&header);
            return Err(HsmError("HSM Dispatch Channel Closed".to_string()));
        }

        match tokio::time::timeout(std::time::Duration::from_secs(2), resp_rx).await {
            Ok(Ok(resp)) => {
                if resp.command == "CB" && resp.payload.starts_with("00") {
                    Ok(resp.payload[2..].to_string())
                } else {
                    Err(HsmError("Failed to translate PIN".to_string()))
                }
            }
            _ => {
                self.correlation_map.remove(&header);
                Err(HsmError("Timeout or fail awaiting HSM CB response".to_string()))
            }
        }
    }
}
