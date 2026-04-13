use switch_core::state::GlobalState;
use std::sync::Arc;
use tokio::time::{interval, Duration};
use payment_proto::canonical::{CanonicalTransaction, MessageClass, TransactionType, ProcessingCode, Stan, LocalTime, LocalDate, Rrn, ResponseCode};
use switch_core::context::{TransactionContext, ActiveTransaction, NetworkState};

use switch_core::telemetry::{TelemetryLogger, TelemetryEvent};

pub async fn start_network_manager(state: Arc<GlobalState>, guard: Arc<dashmap::DashMap<String, ActiveTransaction>>, telemetry: TelemetryLogger) {
    let mut ticker = interval(Duration::from_secs(60));
    
    loop {
        ticker.tick().await;

        let channel_snapshots: Vec<(String, crossbeam::channel::Sender<TransactionContext>)> = 
            state.egress_channels.iter().map(|e| (e.key().clone(), e.value().clone())).collect();

        for (node_id, channel) in channel_snapshots {
            let tx = CanonicalTransaction {
                message_class: MessageClass::NetworkManagement,
                transaction_type: TransactionType::Purchase,
                mti: bytes::Bytes::from_static(b"0800"),
                pan: bytes::Bytes::from_static(b"0000000000000000"),
                processing_code: ProcessingCode("000000".to_string()),
                amount: 0,
                stan: Stan("000800".to_string()),
                local_time: LocalTime("120000".to_string()),
                local_date: LocalDate("1231".to_string()),
                rrn: Rrn(format!("NETECHO_{}", node_id)),
                response_code: ResponseCode("00".to_string()),
                acquirer_id: bytes::Bytes::from_static(b"SWITCH"),
                pin_block: bytes::Bytes::new(),
            };

            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            
            let tx_clone = tx.clone();
            let key = format!("{}-{}-{}", tx.rrn.0, tx.stan.0, String::from_utf8_lossy(&tx.acquirer_id));
            guard.insert(
                key,
                ActiveTransaction {
                    created_at: std::time::Instant::now(),
                    state: NetworkState::UpstreamSent,
                    responder: Some(reply_tx),
                    transaction_data: switch_core::to_pb(&tx_clone),
                }
            );

            let ctx = TransactionContext {
                transaction: tx,
                responder: None,
            };

            let state_clone = Arc::clone(&state);
            let n_id = node_id.clone();
            let telly = telemetry.clone();
            let record_rrn = tx_clone.rrn.0.clone();

            tokio::spawn(async move {
                if channel.send(ctx).is_ok() {
                    telly.log(TelemetryEvent::Journaled { rrn: record_rrn.clone(), size_bytes: 40 });
                    telly.log(TelemetryEvent::Routed { rrn: record_rrn.clone() });
                    match tokio::time::timeout(std::time::Duration::from_secs(5), reply_rx).await {
                        Ok(Ok(_)) => {
                            state_clone.node_health.insert(n_id, std::sync::atomic::AtomicBool::new(true));
                        }
                        _ => {
                            state_clone.node_health.insert(n_id, std::sync::atomic::AtomicBool::new(false));
                        }
                    }
                } else {
                    state_clone.node_health.insert(n_id, std::sync::atomic::AtomicBool::new(false));
                }
            });
        }
    }
}
