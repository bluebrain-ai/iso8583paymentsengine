use edge_ingress::codec::Iso8583Codec;
use futures::{SinkExt, StreamExt};
use iso_dialect::{Base24Dialect, ConnexDialect, DialectRouter};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_util::codec::Framed;
use payment_proto::canonical::{CanonicalTransaction, MessageClass, ResponseCode};

#[tokio::main]
async fn main() {
    println!("Starting Mock Bank Executable...");
    
    // Spawn Mock Visa Endpoint
    tokio::spawn(async move {
        if let Ok(listener) = TcpListener::bind("127.0.0.1:9188").await {
            println!("[VISA MOCK] Listening on 127.0.0.1:9188");
            loop {
                if let Ok((socket, _)) = listener.accept().await {
                    socket.set_nodelay(true).unwrap();
                    tokio::spawn(async move {
                        let framed = Framed::new(socket, Iso8583Codec);
                        let (mut sink, mut stream) = framed.split();
                        let dialect = DialectRouter::Connex(ConnexDialect);
                        
                        let (tx, mut rx) = tokio::sync::mpsc::channel(100_000);
                        
                        tokio::spawn(async move {
                            while let Some(msg) = rx.recv().await {
                                let _ = sink.feed(msg).await;
                                while let Ok(extra_msg) = rx.try_recv() {
                                    let _ = sink.feed(extra_msg).await;
                                }
                                let _ = sink.flush().await;
                            }
                        });

                        while let Some(result) = stream.next().await {
                            if let Ok(payload) = result {
                                let tx_clone = tx.clone();
                                let dialect_clone = DialectRouter::Connex(ConnexDialect);
                                tokio::spawn(async move {
                                    if let Ok(canonical_tx) = dialect_clone.decode(&payload) {
                                        tokio::time::sleep(Duration::from_millis(5)).await;
                                        if let Some(encoded) = process_bank_rules(&canonical_tx, &dialect_clone) {
                                            let _ = tx_clone.send(bytes::BytesMut::from(&encoded[..])).await;
                                        }
                                    } else if let Err(e) = dialect_clone.decode(&payload) {
                                        // Strict bank firewall drops invalid formats
                                        println!("[MASTERCARD BANK ERROR] Dropping invalid format from Core! Error: {:?}", e);
                                        return;
                                    }
                                });
                            }
                        }
                    });
                }
            }
        }
    });

    // Spawn Mock Mastercard Endpoint
    tokio::spawn(async move {
        if let Ok(listener) = TcpListener::bind("127.0.0.1:9200").await {
            println!("[MASTERCARD MOCK] Listening on 127.0.0.1:9200");
            loop {
                if let Ok((socket, _)) = listener.accept().await {
                    socket.set_nodelay(true).unwrap();
                    tokio::spawn(async move {
                        let framed = Framed::new(socket, Iso8583Codec);
                        let (mut sink, mut stream) = framed.split();
                        let dialect = DialectRouter::Base24(Base24Dialect);
                        
                        let (tx, mut rx) = tokio::sync::mpsc::channel(100_000);
                        
                        tokio::spawn(async move {
                            while let Some(msg) = rx.recv().await {
                                let _ = sink.feed(msg).await;
                                while let Ok(extra_msg) = rx.try_recv() {
                                    let _ = sink.feed(extra_msg).await;
                                }
                                let _ = sink.flush().await;
                            }
                        });

                        while let Some(result) = stream.next().await {
                            if let Ok(payload) = result {
                                let tx_clone = tx.clone();
                                let dialect_clone = DialectRouter::Base24(Base24Dialect);
                                tokio::spawn(async move {
                                    if let Ok(canonical_tx) = dialect_clone.decode(&payload) {
                                        tokio::time::sleep(Duration::from_millis(10)).await;
                                        if let Some(encoded) = process_bank_rules(&canonical_tx, &dialect_clone) {
                                            let _ = tx_clone.send(bytes::BytesMut::from(&encoded[..])).await;
                                        }
                                    } else if let Err(_e) = dialect_clone.decode(&payload) {
                                        // Strict bank firewall drops invalid formats
                                        return;
                                    }
                                });
                            }
                        }
                    });
                }
            }
        }
    });

    // Indefinitely block main thread
    tokio::signal::ctrl_c().await.unwrap();
    println!("Shutting down Mock Bank Executable.");
}

fn process_bank_rules(canonical_tx: &CanonicalTransaction, dialect: &DialectRouter) -> Option<bytes::Bytes> {
    let mut response = canonical_tx.clone();

    match canonical_tx.message_class {
        MessageClass::NetworkManagement => {
            response.mti = bytes::Bytes::from_static(b"0810");
            response.response_code = ResponseCode("00".to_string());
            response.message_class = MessageClass::NetworkManagementResponse;
        },
        MessageClass::ReversalAdvice => {
            response.mti = bytes::Bytes::from_static(b"0430");
            response.response_code = ResponseCode("00".to_string());
            response.message_class = MessageClass::ReversalResponse;
        },
        MessageClass::Authorization | MessageClass::Financial => {
            if canonical_tx.message_class == MessageClass::Authorization {
                response.mti = bytes::Bytes::from_static(b"0110");
            } else {
                response.mti = bytes::Bytes::from_static(b"0210");
            }

            if canonical_tx.amount > 50000 {
                response.response_code = ResponseCode("51".to_string()); // Insufficient Funds
            } else if canonical_tx.pan.ends_with(b"9999") {
                response.response_code = ResponseCode("59".to_string()); // Suspected Fraud
            } else {
                response.response_code = ResponseCode("00".to_string()); // Approved
            }
        },
        _ => return None // Drop anything else
    }

    dialect.encode(&response).map(|b| bytes::Bytes::from(b)).ok()
}

