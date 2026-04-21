use crate::context::{ActiveTransaction, NetworkState, TransactionContext, CardProfile};
use crate::journal::Journaler;
use crate::telemetry::{TelemetryEvent, TelemetryLogger};
use crossbeam::channel::{bounded, Receiver, Sender};
use dashmap::{DashMap, mapref::entry::Entry};
use crate::hsm_client::HsmClient;
use crate::compliance::ComplianceGuard;
use crate::ledger_worker::LedgerWorker;
use prost::Message;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub enum TxState {
    Received,
    HSMPending,
    Journaled,
    Routing,
}

pub struct SwitchEngine {
    ingress_tx: Sender<TransactionContext>,
    visa_egress_rx: Receiver<TransactionContext>,
    mastercard_egress_rx: Receiver<TransactionContext>,
    bank_reply_tx: Sender<payment_proto::Transaction>,
    pub stip_db: sled::Db,
    pub idempotency_guard: Arc<DashMap<String, ActiveTransaction>>,
    pub memory_state: Arc<crate::state::GlobalState>,
    ledger_tx: Sender<payment_proto::canonical::UniversalPaymentEvent>,
}

impl SwitchEngine {
    pub fn start(journaler: Journaler, telemetry: TelemetryLogger, timeout_ms: u64) -> Self {
        let (ingress_tx, ingress_rx) = bounded::<TransactionContext>(10000);
        let (visa_egress_tx, visa_egress_rx) = bounded::<TransactionContext>(10000);
        let (mastercard_egress_tx, mastercard_egress_rx) = bounded::<TransactionContext>(10000);
        let (hsm_tx, hsm_rx) = bounded::<TransactionContext>(10000);
        let (bank_reply_tx, bank_reply_rx) = bounded::<payment_proto::Transaction>(10000);
        let (ledger_tx, ledger_rx) = bounded::<payment_proto::canonical::UniversalPaymentEvent>(10000);

        let idempotency_guard = Arc::new(DashMap::<String, ActiveTransaction>::new());
        let stip_db = sled::open("stip.db").unwrap();
        let memory_state = Arc::new(crate::state::GlobalState::new());

        memory_state.egress_channels.insert("MockVisaNode".to_string(), visa_egress_tx.clone());
        memory_state.egress_channels.insert("MockMastercardNode".to_string(), mastercard_egress_tx.clone());

        // Pillar 5 Instantiations
        let compliance_guard = Arc::new(ComplianceGuard::new());
        let ledger_worker = Arc::new(LedgerWorker::new());
        let db_path = "../ops-control-plane/ops.db".to_string(); // path to SQLite ops.db
        
        ComplianceGuard::start_polling(db_path.clone(), compliance_guard.clone());
        ledger_worker.start_polling(db_path.clone());
        ledger_worker.start_worker(db_path.clone(), ledger_rx);

        // Spawn EOD settlement cron asynchronously
        let settlement_db = db_path.clone();
        tokio::spawn(async move {
            crate::settlement_cron::SettlementEngine::start_cron(settlement_db).await;
        });

        let threads = std::cmp::max(1, num_cpus::get() / 2);
        telemetry.log(TelemetryEvent::EngineStarted { workers: threads });

        for _ in 0..threads {
            let ingress_rx = ingress_rx.clone();
            let visa_egress_tx = visa_egress_tx.clone();
            let mastercard_egress_tx = mastercard_egress_tx.clone();
            let hsm_tx = hsm_tx.clone();
            let idempotency_guard = Arc::clone(&idempotency_guard);
            let journaler = journaler.clone();
            let telemetry = telemetry.clone();
            let safe_stip_db = stip_db.clone();
            let safe_memory_state = Arc::clone(&memory_state);
            let safe_compliance_guard = compliance_guard.clone();
            let safe_ledger_tx = ledger_tx.clone();

            thread::spawn(move || {
                for mut context in ingress_rx {
                    let start = std::time::Instant::now();
                    let rrn = context.transaction.rrn.0.clone();
                    let stan = context.transaction.stan.0.clone();
                    let acquirer_id = String::from_utf8_lossy(&context.transaction.acquirer_id).to_string();

                    let _span = tracing::info_span!("process_transaction", rrn = %rrn).entered();

                    telemetry.log(TelemetryEvent::TransactionReceived {
                        rrn: rrn.clone(),
                        stan: stan.clone(),
                        acquirer_id: acquirer_id.clone(),
                    });

                    tracing::info!("Transaction parsed and validated");

                    // ── Phase 2: Compliance Interceptor ───────────────────────────────────────
                    if safe_compliance_guard.evaluate(&context) {
                        metrics::increment_counter!("transactions_total", "status" => "declined");
                        tracing::warn!("Compliance Interceptor blocked transaction {}", rrn);
                        telemetry.log(TelemetryEvent::RoutingFailed { rrn: rrn.clone() });
                        if let Some(responder) = context.responder.take() {
                            let mut decline = context.transaction.clone();
                            decline.mti = bytes::Bytes::from_static(b"0110");
                            decline.response_code = payment_proto::canonical::ResponseCode("93".to_string()); // Violation of Law
                            let _ = responder.send(decline);
                        }
                        continue;
                    }

                    // TSP Detokenizer Interception
                    if context.transaction.is_tokenized {
                        let dpan = String::from_utf8_lossy(&context.transaction.fpan).to_string();
                        if let Some(real_fpan) = safe_memory_state.tsp_vault.detokenize(&dpan) {
                            context.transaction.fpan = bytes::Bytes::from(real_fpan);
                            context.transaction.dpan = Some(dpan.clone());
                            
                            let masked_pan = format!("{}****{}", &dpan[..6], &dpan[dpan.len().saturating_sub(4)..]);
                            tracing::info!("Detokenization successful. DPAN: {}", masked_pan);
                        } else {
                            metrics::increment_counter!("transactions_total", "status" => "declined");
                            telemetry.log(TelemetryEvent::RoutingFailed { rrn: rrn.clone() });
                            if let Some(responder) = context.responder.take() {
                                let mut decline = context.transaction.clone();
                                decline.mti = bytes::Bytes::from_static(b"0110");
                                decline.response_code = payment_proto::canonical::ResponseCode("56".to_string());
                                let _ = responder.send(decline);
                            }
                            continue;
                        }
                    }

                    // Snapshot lock-free cache read exactly once at routing inception
                    let current_stip = safe_memory_state.stip.load();
                    let _current_crdb = safe_memory_state.crdb.load();

                    // ── Phase 3: Dynamic Risk-Based STIP Pre-Screen ──────────────────────────
                    // Evaluate the network risk score BEFORE the idempotency guard and online
                    // forwarding. A high-risk score (exceeding the operator-configured threshold
                    // for the card's network) triggers an instant RC 59 decline.  This happens
                    // on ALL transactions — not just STIP fallback — to prevent fraud propagation.
                    if context.transaction.risk_score > 0 {
                        let network_id = if context.transaction.fpan.starts_with(b"4") { "VISA" }
                            else if context.transaction.fpan.starts_with(b"5") { "MASTERCARD" }
                            else { "DEFAULT" };
                        // Look up the configured max risk score for this network.
                        // If no rule exists, default to 100 (screening disabled).
                        let threshold = current_stip
                            .rules
                            .get(network_id)
                            .map(|r| r.max_risk_score)
                            .unwrap_or(100u8);
                        if context.transaction.risk_score > threshold {
                            metrics::increment_counter!("transactions_total", "status" => "declined");
                            telemetry.log(TelemetryEvent::RoutingFailed { rrn: rrn.clone() });
                            if let Some(responder) = context.responder.take() {
                                let mut decline = context.transaction.clone();
                                decline.mti = bytes::Bytes::from_static(b"0110");
                                decline.response_code = payment_proto::canonical::ResponseCode("59".to_string()); // Suspected Fraud
                                let _ = responder.send(decline);
                            }
                            continue;
                        }
                    }

                    let mut _state = TxState::Received;
                    
                    let key = format!("{}-{}-{}", rrn, stan, acquirer_id);

                    use payment_proto::canonical::MessageClass;

                    match context.transaction.message_class {
                        MessageClass::NetworkManagement => {
                            let is_for_switch = !context.transaction.fpan.starts_with(b"4") && !context.transaction.fpan.starts_with(b"5");

                            if is_for_switch {
                                let mut echo_reply = context.transaction.clone();
                                echo_reply.mti = bytes::Bytes::from_static(b"0810");
                                echo_reply.response_code = payment_proto::canonical::ResponseCode("00".to_string());
                                echo_reply.message_class = MessageClass::NetworkManagementResponse;

                                if let Some(responder) = context.responder.take() {
                                    let _ = responder.send(echo_reply);
                                }
                                continue;
                            }
                        },
                        _ => {}
                    }

                    if context.transaction.is_reversal {
                        if let Some(ref odata) = context.transaction.original_data_elements {
                            if odata.len() >= 20 {
                                let orig_stan = &odata[4..10];
                                for mut entry in idempotency_guard.iter_mut() {
                                    if entry.transaction_data.stan == orig_stan {
                                        let rrn_clone = entry.transaction_data.rrn.clone();
                                        entry.state = NetworkState::ReversedPending;
                                        let mut rev_event = context.transaction.clone();
                                        rev_event.rrn = payment_proto::canonical::Rrn(rrn_clone);
                                        let _ = safe_ledger_tx.send(rev_event);
                                    }
                                }
                            }
                        }
                        
                        let mut rev_reply = context.transaction.clone();
                        rev_reply.mti = bytes::Bytes::from_static(b"0430");
                        rev_reply.response_code = payment_proto::canonical::ResponseCode("00".to_string());
                        if let Some(responder) = context.responder.take() {
                            let _ = responder.send(rev_reply);
                        }
                        continue;
                    }
                    match idempotency_guard.entry(key.clone()) {
                        Entry::Occupied(_) => {
                            metrics::increment_counter!("transactions_total", "status" => "declined");
                            metrics::histogram!("processing_latency_ms", start.elapsed().as_millis() as f64);
                            
                            telemetry.log(TelemetryEvent::DuplicateDetected { key });
                            if let Some(responder) = context.responder.take() {
                                let mut decline = context.transaction.clone();
                                decline.mti = bytes::Bytes::from_static(b"0110");
                                decline.response_code = payment_proto::canonical::ResponseCode("94".to_string()); // Duplicate
                                let _ = responder.send(decline);
                            }
                            continue;
                        }
                        Entry::Vacant(v) => {
                            v.insert(ActiveTransaction {
                                created_at: Instant::now(),
                                state: NetworkState::UpstreamSent,
                                responder: context.responder.take(),
                                transaction_data: crate::to_pb(&context.transaction),
                                // Populated post-routing when a unique egress STAN is assigned.
                                // Remains None for CRDB-routed and STIP-fallback transactions.
                                egress_stan: None,
                            });
                        }
                    }

                    if !context.transaction.pin_block.is_empty() || context.transaction.mac_data.is_some() {
                        _state = TxState::HSMPending;
                        let _ = hsm_tx.clone().send(context);
                        continue;
                    }

                    let pb_tx = crate::to_pb(&context.transaction);
                    let append_res = journaler.append(&pb_tx);

                    if append_res.is_ok() {
                        _state = TxState::Journaled;
                        telemetry.log(TelemetryEvent::Journaled {
                            rrn: rrn.clone(),
                            size_bytes: pb_tx.encoded_len(), 
                        });
                        
                        let target_egress = match context.transaction.message_class {
                            MessageClass::Authorization | MessageClass::Financial | MessageClass::ReversalAdvice | MessageClass::NetworkManagement => {
                                let router = safe_memory_state.router.load();
                                if let Some(route) = router.find(&context.transaction.fpan) {
                                    match &route.destination {
                                        crate::routing::RouteDestination::InternalCrdb => {
                                            if let Some(ch) = safe_memory_state.egress_channels.get("CRDB_DOMAIN_2") {
                                                Some(ch.clone())
                                            } else { None }
                                        },
                                        crate::routing::RouteDestination::ExternalNode(node_id) => {
                                            let mut target_node = node_id.clone();
                                            // Health semantics: treat absent entry as ONLINE (fail-open).
                                            // node_health is only populated after the network manager
                                            // completes an 0800 echo cycle. Before that first probe we
                                            // must assume the node is reachable — failing open is the
                                            // correct industry default for payment switches.
                                            let mut is_online = safe_memory_state.node_health
                                                .get(&target_node)
                                                .map(|h| h.load(std::sync::atomic::Ordering::Relaxed))
                                                .unwrap_or(true); // absent → optimistically online

                                            if !is_online {
                                                if let Some(failover) = &route.failover_node {
                                                    target_node = failover.clone();
                                                    is_online = safe_memory_state.node_health
                                                        .get(&target_node)
                                                        .map(|h| h.load(std::sync::atomic::Ordering::Relaxed))
                                                        .unwrap_or(true); // absent → optimistically online
                                                }
                                            }

                                            if is_online {
                                                if let Some(ch) = safe_memory_state.egress_channels.get(&target_node) {
                                                    Some(ch.clone())
                                                } else { None }
                                            } else {
                                                // Both primary and confirmed failover down → STIP fallback
                                                None
                                            }
                                        }
                                    }
                                } else { None }
                            },
                            _ => None
                        };

                        if let Some(tx_channel) = target_egress {
                            metrics::increment_counter!("transactions_total", "status" => "approved");
                            metrics::histogram!("processing_latency_ms", start.elapsed().as_millis() as f64);

                            // ── Phase 1: STAN Translation NAT (Outbound) ─────────────────────
                            // Generate a globally-unique 6-digit egress STAN via a lock-free
                            // AtomicU64 counter. Store the ingress→egress mapping so that the
                            // bank reply loop can reverse-translate and wake the correct POS
                            // terminal. The guard entry is also updated with the egress STAN so
                            // the Reaper can GC the NAT on SLA timeout.
                            let ingress_stan_for_nat = context.transaction.stan.0.clone();
                            let egress_stan = format!(
                                "{:06}",
                                safe_memory_state.stan_counter
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                                    % 1_000_000
                            );
                            safe_memory_state.stan_nat.insert(
                                egress_stan.clone(),
                                ingress_stan_for_nat,
                            );
                            // Overwrite Field 11 in the egress frame
                            context.transaction.stan = payment_proto::canonical::Stan(egress_stan.clone());
                            // Record egress STAN in the guard entry so Reaper can GC nat on timeout
                            if let Some(mut guard_entry) = idempotency_guard.get_mut(&key) {
                                guard_entry.egress_stan = Some(egress_stan);
                            }

                            let tx_for_ledger = context.transaction.clone();
                            // Send to Ledger Async GL Sweep for all inbound captures
                            let _ = safe_ledger_tx.send(tx_for_ledger);
                            
                            match tx_channel.send_timeout(context, Duration::from_millis(100)) {
                                Ok(_) => {
                                    _state = TxState::Routing;
                                    telemetry.log(TelemetryEvent::Routed { rrn });
                                }
                                Err(crossbeam::channel::SendTimeoutError::Timeout(mut failed_ctx)) | 
                                Err(crossbeam::channel::SendTimeoutError::Disconnected(mut failed_ctx)) => {
// ... remaining stip fallback ...
                                    // SAF STIP FALLBACK! Egress Offline
                                    if let Ok(Some(cached_bytes)) = safe_stip_db.get(&failed_ctx.transaction.fpan) {
                                        if let Ok(mut profile) = bincode::deserialize::<CardProfile>(&cached_bytes) {
                                            if profile.is_blocked {
                                                failed_ctx.transaction.mti = bytes::Bytes::from_static(b"0110");
                                                failed_ctx.transaction.response_code = payment_proto::canonical::ResponseCode("41".to_string());
                                                telemetry.log(TelemetryEvent::StipFallbackDeclined { rrn: rrn.clone(), reason: "CARD BLOCKED".to_string() });
                                            } else if profile.balance >= failed_ctx.transaction.amount as i64 {
                                                // Deduct Balance Natively
                                                profile.balance -= failed_ctx.transaction.amount as i64;
                                                let _ = safe_stip_db.insert(&failed_ctx.transaction.fpan, bincode::serialize(&profile).unwrap());
                                                
                                                failed_ctx.transaction.mti = bytes::Bytes::from_static(b"0110");
                                                failed_ctx.transaction.response_code = payment_proto::canonical::ResponseCode("00".to_string());
                                                failed_ctx.transaction.processing_code = payment_proto::canonical::ProcessingCode("STIP12".to_string());
                                                telemetry.log(TelemetryEvent::StipFallbackAuthorized { rrn: rrn.clone(), amount: failed_ctx.transaction.amount as u64 });
                                            } else {
                                                failed_ctx.transaction.mti = bytes::Bytes::from_static(b"0110");
                                                failed_ctx.transaction.response_code = payment_proto::canonical::ResponseCode("51".to_string()); // NSF
                                                telemetry.log(TelemetryEvent::StipFallbackDeclined { rrn: rrn.clone(), reason: "INSUFFICIENT FUNDS".to_string() });
                                            }
                                        } else { // decode failure
                                            failed_ctx.transaction.mti = bytes::Bytes::from_static(b"0110");
                                            failed_ctx.transaction.response_code = payment_proto::canonical::ResponseCode("05".to_string()); 
                                            telemetry.log(TelemetryEvent::RoutingFailed { rrn: rrn.clone() });
                                        }
                                    } else { // No STIP mapped 
                                        failed_ctx.transaction.mti = bytes::Bytes::from_static(b"0110");
                                        failed_ctx.transaction.response_code = payment_proto::canonical::ResponseCode("05".to_string()); 
                                        telemetry.log(TelemetryEvent::RoutingFailed { rrn: rrn.clone() });
                                    }

                                    let failed_pb = crate::to_pb(&failed_ctx.transaction);
                                    let append_stip = journaler.append(&failed_pb);
                                    
                                     if append_stip.is_ok() {
                                        if let Some(responder) = failed_ctx.responder.take() {
                                            let _ = responder.send(failed_ctx.transaction.clone());
                                        }
                                    }
                                }
                            }
                        } else {
                            // No matching egress channel found after routing. This means the node
                            // is confirmed down and has no reachable failover. Send an immediate
                            // decline (RC 05 — Do Not Honor) instead of silently dropping the
                            // context, which would cause the reaper to fire a spurious timeout.
                            metrics::increment_counter!("transactions_total", "status" => "declined");
                            metrics::histogram!("processing_latency_ms", start.elapsed().as_millis() as f64);
                            telemetry.log(TelemetryEvent::RoutingFailed { rrn: rrn.clone() });

                            // Retrieve the responder from the idempotency guard and respond.
                            if let Some(mut guarded) = idempotency_guard.get_mut(&key) {
                                if let Some(responder) = guarded.responder.take() {
                                    let mut decline = context.transaction.clone();
                                    decline.mti = bytes::Bytes::from_static(b"0110");
                                    decline.response_code = payment_proto::canonical::ResponseCode("05".to_string());
                                    let _ = responder.send(decline);
                                }
                                guarded.state = NetworkState::Completed;
                            }
                            idempotency_guard.remove(&key);
                        }
                    } else {
                        // Failed to journal
                    }
                }
            });
        }

        let journaler2 = journaler.clone();
        let telemetry2 = telemetry.clone();
        let memory_state_for_hsm = Arc::clone(&memory_state);

        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();

            rt.block_on(async move {
                let thales_client = match HsmClient::connect("127.0.0.1:1500").await {
                    Ok(client) => client,
                    Err(e) => {
                        tracing::error!("Failed to connect to HSM on port 1500: {:?}", e);
                        return;
                    }
                };

                for mut context in hsm_rx {
                    let thales_clone = thales_client.clone();
                    let safe_state2 = Arc::clone(&memory_state_for_hsm);
                    let journaler_clone = journaler2.clone();
                    let telemetry_clone = telemetry2.clone();

                    tokio::spawn(async move {
                        let pan_str = String::from_utf8_lossy(&context.transaction.fpan).to_string();
                        let pin_str = String::from_utf8_lossy(&context.transaction.pin_block).to_string();

                        let router = safe_state2.router.load();
                        let route_opt = router.find(&context.transaction.fpan);
                        
                        let mut is_pin_valid = true;
                        let mut target_ch = None;

                        if let Some(route) = route_opt {
                            match &route.destination {
                                crate::routing::RouteDestination::InternalCrdb => {
                                    let crdb = safe_state2.crdb.load();
                                    
                                    if let Some(card_record) = crdb.cards.get(&pan_str) {
                                        let mut local_mac_valid = true;
                                        if let Some(ref mac) = context.transaction.mac_data {
                                            if let Ok(m_success) = thales_clone.verify_mac(mac, &pan_str).await {
                                                if !m_success {
                                                    local_mac_valid = false;
                                                }
                                            } else {
                                                local_mac_valid = false;
                                            }
                                        }

                                        if !pin_str.is_empty() {
                                            if let Ok(success) = thales_clone.verify_pin(&pin_str, &pan_str, &card_record.pvv).await {
                                                if !success || !local_mac_valid {
                                                    is_pin_valid = false;
                                                } else {
                                                    target_ch = safe_state2.egress_channels.get("CRDB_DOMAIN_2").map(|ch| ch.clone());
                                                }
                                            } else {
                                                is_pin_valid = false;
                                            }
                                        } else {
                                            if !local_mac_valid {
                                                is_pin_valid = false;
                                            } else {
                                                target_ch = safe_state2.egress_channels.get("CRDB_DOMAIN_2").map(|ch| ch.clone());
                                            }
                                        }
                                    } else {
                                        is_pin_valid = false;
                                    }
                                },
                                crate::routing::RouteDestination::ExternalNode(node_id) => {
                                    let mut local_mac_valid = true;
                                    if let Some(ref mac) = context.transaction.mac_data {
                                        if let Ok(m_success) = thales_clone.verify_mac(mac, &pan_str).await {
                                            if !m_success {
                                                local_mac_valid = false;
                                            }
                                        } else {
                                            local_mac_valid = false;
                                        }
                                    }

                                    if !local_mac_valid {
                                        is_pin_valid = false;
                                    } else {
                                        let mut proceed = true;
                                        if !pin_str.is_empty() {
                                            if let Ok(translated_pin) = thales_clone.translate_pin(&pin_str, &pan_str).await {
                                                context.transaction.pin_block = bytes::Bytes::from(translated_pin);
                                            } else {
                                                proceed = false;
                                                is_pin_valid = false;
                                            }
                                        }

                                        if proceed {
                                            let mut n = node_id.clone();
                                            let mut ok = safe_state2.node_health
                                                .get(&n)
                                                .map(|h| h.load(std::sync::atomic::Ordering::Relaxed))
                                                .unwrap_or(true); 
                                            if !ok {
                                               if let Some(f) = &route.failover_node {
                                                    n = f.clone();
                                                    ok = safe_state2.node_health
                                                        .get(&n)
                                                        .map(|h| h.load(std::sync::atomic::Ordering::Relaxed))
                                                        .unwrap_or(true); 
                                               }
                                            }
                                            if ok { target_ch = safe_state2.egress_channels.get(&n).map(|ch| ch.clone()) }
                                        }
                                    }
                                }
                            }
                        }

                        if !is_pin_valid {
                            metrics::increment_counter!("transactions_total", "status" => "declined");
                            let rrn = context.transaction.rrn.0.clone();
                            telemetry_clone.log(TelemetryEvent::RoutingFailed { rrn });
                            
                            if let Some(responder) = context.responder.take() {
                                let mut decline = context.transaction.clone();
                                decline.mti = bytes::Bytes::from_static(b"0110");
                                decline.response_code = payment_proto::canonical::ResponseCode("55".to_string());
                                let _ = responder.send(decline);
                            }
                            return;
                        }

                        let pb_tx = crate::to_pb(&context.transaction);
                        let append_res = journaler_clone.append(&pb_tx);

                        if append_res.is_ok() {
                            let rrn = context.transaction.rrn.0.clone();
                            telemetry_clone.log(TelemetryEvent::Journaled {
                                rrn: rrn.clone(),
                                size_bytes: pb_tx.encoded_len(), 
                            });
                            
                            if let Some(ch) = target_ch {
                                if ch.send(context).is_ok() {
                                    telemetry_clone.log(TelemetryEvent::Routed { rrn });
                                }
                            }
                        }
                    });
                }
            });
        });

        // Background Reaper Engine
        let reaper_guard = Arc::clone(&idempotency_guard);
        // Make `memory_state` block properly visible below to reaper code:
        let reaper_memory_state = Arc::clone(&memory_state);

        thread::spawn(move || {
            let sleep_interval = if timeout_ms < 1000 { timeout_ms / 2 } else { 1000 };
            // Grace period before pruning Completed/Reversed entries from DashMap.
            // Without this the map grows unboundedly across load-test runs.
            let prune_after = Duration::from_secs(30);
            loop {
                thread::sleep(Duration::from_millis(std::cmp::max(1, sleep_interval)));

                // Pass 1 — timeout & reverse UpstreamSent entries that exceeded the SLA
                let mut to_reverse: Vec<String> = Vec::new();
                for entry in reaper_guard.iter() {
                    if entry.state == NetworkState::UpstreamSent {
                        if Instant::now().duration_since(entry.created_at) > Duration::from_millis(timeout_ms) {
                            to_reverse.push(entry.key().clone());
                        }
                    }
                }
                for key in to_reverse {
                    // Use a flag so we can drop the RefMut before calling remove().
                    // DashMap cannot hold get_mut() and remove() simultaneously on
                    // the same key — it would deadlock on the shard lock.
                    let mut did_reverse = false;
                    // Capture the egress STAN while holding the RefMut so we can GC
                    // the stan_nat entry after dropping the borrow.
                    let mut nat_egress_stan_to_gc: Option<String> = None;

                    if let Some(mut entry) = reaper_guard.get_mut(&key) {
                        if entry.state == NetworkState::UpstreamSent {
                            entry.state = NetworkState::Reversed;
                            // ── Phase 1: NAT Reaper GC — capture egress STAN before drop ──
                            nat_egress_stan_to_gc = entry.egress_stan.take();

                            if let Some(responder) = entry.responder.take() {
                                let mut timeout_reply = entry.transaction_data.clone();
                                timeout_reply.mti = "0110".to_string();
                                timeout_reply.response_code = "68".to_string(); // Timeout
                                let _ = responder.send(crate::from_pb(&timeout_reply));
                            }

                            // Generate 0420 Reversal Context
                            let mut pb_tx = entry.transaction_data.clone();
                            pb_tx.mti = "0420".to_string();
                            let rev_ctx = TransactionContext {
                                transaction: crate::from_pb(&pb_tx),
                                responder: None,
                            };
                            
                            let router = reaper_memory_state.router.load();
                            if let Some(route) = router.find(&rev_ctx.transaction.fpan) {
                                if let crate::routing::RouteDestination::ExternalNode(ref n) = route.destination {
                                    if let Some(ch) = reaper_memory_state.egress_channels.get(n) {
                                        let _ = ch.send(rev_ctx);
                                    }
                                }
                            }
                            did_reverse = true;
                        }
                    } // ← RefMut dropped here; shard lock released

                    // GC the NAT entry for this timed-out transaction. This MUST happen
                    // after the RefMut is dropped (above) to avoid DashMap shard deadlock.
                    if let Some(egress_stan) = nat_egress_stan_to_gc {
                        reaper_memory_state.stan_nat.remove(&egress_stan);
                    }

                    // Immediately evict — lifecycle is terminal (Reversed + 0420 sent)
                    if did_reverse {
                        reaper_guard.remove(&key);
                    }
                }

                // Pass 2 — prune Completed/Reversed entries older than grace period
                // Collect keys first to avoid holding a mutable borrow across retain
                let stale_keys: Vec<String> = reaper_guard
                    .iter()
                    .filter(|e| {
                        matches!(e.state, NetworkState::Completed | NetworkState::Reversed)
                            && Instant::now().duration_since(e.created_at) > prune_after
                    })
                    .map(|e| e.key().clone())
                    .collect();
                for key in stale_keys {
                    reaper_guard.remove(&key);
                }
            }
        });

        // Bank Reply Inbound SEDA Loop
        // 
        // [ARCHITECTURE EXPLANATION: SEDA Pipeline Thread Mapping]
        // This bank reply listener traverses synchronous disk I/O logic (Sled WAL appends). 
        // Because of this native blocking I/O, we must utilize OS-level threads (`std::thread::spawn`) 
        // instead of async Tokio closures, which would freeze the central async networking event loop.
        // 
        // We explicitly map the worker count (hardcoded to exactly 8 to match the inbound pipeline) 
        // rather than using unbounded OS threads (50-500 threads). Spinning unbounded OS threads over 
        // the Sled DB B-tree locks forces the CPU to spend more MS on "Context Switching" registers 
        // than executing data payloads, deteriorating the 10,000 TPS baseline instantly.
        // In the future, to achieve theoretical scaling bounds on mainframes, this should use dynamic hardware
        // mapping via `num_cpus::get()` to achieve Peak Harmonic Concurrency: 1 Thread per Physical CPU core.
        // 
        for _ in 0..8 {
            let reply_guard = Arc::clone(&idempotency_guard);
            let reply_journaler = journaler.clone();
            let reply_telemetry = telemetry.clone();
            let rx = bank_reply_rx.clone();
            // Clone GlobalState so each reply worker can access stan_nat for NAT reverse-translation.
            let reply_guard_nat = Arc::clone(&memory_state);
            
            thread::spawn(move || {
                for reply_tx in rx {
                // ── Phase 1: STAN Translation NAT (Inbound) ───────────────────────────────
                // The bank mirrored our *egress* STAN in Field 11. Reverse-translate it back
                // to the original *ingress* STAN to reconstruct the idempotency guard key
                // that the SEDA ingress worker inserted at transaction inception.
                // DashMap::remove() is O(1) lock-free and simultaneously GCs the NAT entry.
                let egress_stan_from_bank = reply_tx.stan.clone();
                let ingress_stan = reply_guard_nat
                    .stan_nat
                    .remove(&egress_stan_from_bank)
                    .map(|(_, ingress)| ingress)
                    .unwrap_or_else(|| egress_stan_from_bank); // fallback: 0800 pings / CRDB txns have no NAT entry

                let key = format!("{}-{}-{}", reply_tx.rrn, ingress_stan, reply_tx.acquirer_id);

                // Track the terminal outcome so we can remove after dropping the borrow.
                // DashMap: cannot call remove() while holding a get_mut() RefMut on the
                // same key — the shard lock would deadlock. Drop first, then remove.
                let mut lifecycle_ended = false;

                if let Some(mut entry) = reply_guard.get_mut(&key) {
                    if entry.state == NetworkState::Reversed {
                        // RACE CONDITION MITIGATION:
                        // The Reaper fired exactly as the bank replied (e.g. at 15.001 seconds).
                        // Reaper already sent the timeout response and the 0420 — lifecycle
                        // is fully terminal. Log, mark for removal, and drop.
                        reply_telemetry.log(TelemetryEvent::LateBankReplyDropped { rrn: reply_tx.rrn.clone() });
                        lifecycle_ended = true;
                        // fall through to remove below
                    } else {
                        // Proceed: Update state to Completed
                        entry.state = NetworkState::Completed;

                        // Journal the 0110 Approval safely to WAL
                        let append_res = reply_journaler.append(&reply_tx);

                        // Return genuine approval back.
                        // WIRE FORMAT NOTE: bitmap bytes MUST be 0x00 here to distinguish
                        // a real approval from the reaper's timeout reply (which uses 0xFF).
                        // Node's tcpSend() reads these bytes to classify ok/timeout.
                        if append_res.is_ok() {
                            if let Some(responder) = entry.responder.take() {
                                let mut approval = reply_tx.clone();
                                approval.mti = "0110".to_string();
                                let _ = responder.send(crate::from_pb(&approval));
                            }
                            lifecycle_ended = true;
                        }
                    }
                } // ← RefMut dropped here; shard lock released

                // Immediately evict once the lifecycle is terminal.
                // For Completed: the client has its response, journal is written.
                // For Reversed (late reply): the reaper already evicted or will shortly;
                //   remove here too — idempotent, DashMap::remove is a no-op if absent.
                if lifecycle_ended {
                    reply_guard.remove(&key);
                }
                }
            });
        }

        Self {
            ingress_tx,
            visa_egress_rx,
            mastercard_egress_rx,
            bank_reply_tx,
            stip_db,
            idempotency_guard,
            memory_state,
            ledger_tx,
        }
    }

    pub fn ingress(&self) -> Sender<TransactionContext> {
        self.ingress_tx.clone()
    }
    
    pub fn bank_reply(&self) -> Sender<payment_proto::Transaction> {
        self.bank_reply_tx.clone()
    }

    pub fn visa_egress(&self) -> Receiver<TransactionContext> {
        self.visa_egress_rx.clone()
    }

    pub fn mastercard_egress(&self) -> Receiver<TransactionContext> {
        self.mastercard_egress_rx.clone()
    }
}
