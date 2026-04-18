use switch_core::state::GlobalState;
use std::sync::Arc;
use tokio::time::{interval, Duration};
use payment_proto::canonical::{UniversalPaymentEvent, MessageClass, TransactionType, ProcessingCode, Stan, LocalTime, LocalDate, Rrn, ResponseCode};
use switch_core::context::{TransactionContext, ActiveTransaction, NetworkState};

use switch_core::telemetry::{TelemetryLogger, TelemetryEvent};

pub async fn start_network_manager(state: Arc<GlobalState>, guard: Arc<dashmap::DashMap<String, ActiveTransaction>>, telemetry: TelemetryLogger) {
    // First tick fires after 5 seconds (warm-up), then every 60 seconds.
    let mut ticker = interval(Duration::from_secs(60));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    
    // Initial warm-up delay: give all egress clients time to establish their TCP
    // connections to the mock banks before the first health probe fires.
    tokio::time::sleep(Duration::from_secs(5)).await;

    loop {
        ticker.tick().await;

        let channel_snapshots: Vec<(String, crossbeam::channel::Sender<TransactionContext>)> = 
            state.egress_channels.iter().map(|e| (e.key().clone(), e.value().clone())).collect();

        for (node_id, channel) in channel_snapshots {
            // Use a unique STAN per probe so the idempotency guard never sees a
            // duplicate. Use the current unix-second epoch as a discriminator.
            let probe_id = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let tx = UniversalPaymentEvent {
                message_class: MessageClass::NetworkManagement,
                transaction_type: TransactionType::Purchase,
                mti: bytes::Bytes::from_static(b"0800"),
                fpan: bytes::Bytes::from_static(b"0000000000000000"),
                dpan: None,
                is_tokenized: false,
                tavv_cryptogram: None,
                processing_code: ProcessingCode("000000".to_string()),
                amount: 0,
                stan: Stan(format!("{:06}", probe_id % 1_000_000)),
                local_time: LocalTime("120000".to_string()),
                local_date: LocalDate("1231".to_string()),
                rrn: Rrn(format!("PING{:08}", probe_id)),
                response_code: ResponseCode("00".to_string()),
                acquirer_id: bytes::Bytes::from_static(b"SWITCH"),
                pin_block: bytes::Bytes::new(),
                risk_score: 0,
            };

            let ctx = TransactionContext {
                transaction: tx,
                responder: None, // fire-and-forget; we measure channel liveness, not response
            };

            let state_clone = Arc::clone(&state);
            let n_id = node_id.clone();

            // A successful send to the bounded channel means the egress client is
            // alive and consuming. A disconnected or full channel marks the node down.
            match channel.try_send(ctx) {
                Ok(_) => {
                    state_clone.node_health.insert(n_id, std::sync::atomic::AtomicBool::new(true));
                }
                Err(_) => {
                    state_clone.node_health.insert(n_id, std::sync::atomic::AtomicBool::new(false));
                }
            }
        }
    }
}

