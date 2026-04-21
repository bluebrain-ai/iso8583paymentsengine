use crossbeam::channel::{bounded, Receiver, Sender};
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// --- Native ISO Core Dependencies ---
use chrono::Utc;
use payment_proto::canonical::*;
use iso_dialect::{DialectRouter, ConnexDialect};

static STAN_COUNTER: AtomicUsize = AtomicUsize::new(1);

fn generate_transaction(profile: &str, rrn_str: String, network: &str, entry_mode: &str) -> UniversalPaymentEvent {
    let mut rng = rand::thread_rng();
    
    let stan_val = STAN_COUNTER.fetch_add(1, Ordering::Relaxed) % 999999;
    let stan_str = format!("{:06}", stan_val);

    let now = Utc::now();
    let local_time = now.format("%H%M%S").to_string();
    let local_date = now.format("%m%d").to_string();

    let amount = if profile == "Decline" {
        rng.gen_range(50001..999999) // Generates RC 51 Insufficient Funds trigger
    } else {
        rng.gen_range(100..49999)
    };

    let mut base_fpan = match network {
        "Visa" => "4111111111111111",
        "Mastercard" => "5111111111111111",
        "Amex" => "3711111111111111",
        _ => "4111111111111111"
    };

    let mut is_tokenized = false;
    let mut dpan = None;
    let mut tavv_cryptogram = None;

    if entry_mode == "ApplePay" {
        // Apple Pay uses NFC tokenization (EMV contactless): no PIN block transmitted.
        // The DPAN (Device PAN) is the wire PAN; the TSP vault maps it to the real FPAN.
        is_tokenized = true;
        base_fpan = "4000000000009999";
        tavv_cryptogram = Some("AABBCCDDEEFF0011".to_string());
    }

    // PIN block rules:
    //   POS (chip+PIN, magstripe+PIN) → include an encrypted PIN block.
    //   EMV contactless / NFC        → no PIN; cardholder verified by cryptogram.
    //   Apple Pay (tokenized NFC)    → no PIN; auth is TAVV cryptogram only.
    let pin_block = if entry_mode == "POS" {
        bytes::Bytes::from_static(b"1234567890ABCDEF")
    } else {
        bytes::Bytes::new()
    };

    UniversalPaymentEvent {
        message_class: MessageClass::Financial,
        transaction_type: TransactionType::Purchase,
        mti: bytes::Bytes::from_static(b"0200"),
        fpan: bytes::Bytes::from(base_fpan),
        dpan,
        is_tokenized,
        tavv_cryptogram,
        processing_code: ProcessingCode("000000".to_string()),
        amount,
        stan: Stan(stan_str),
        local_time: LocalTime(local_time),
        local_date: LocalDate(local_date),
        rrn: Rrn(rrn_str),
        response_code: ResponseCode(String::new()),
        acquirer_id: bytes::Bytes::from_static(b"411111"),
        pin_block: bytes::Bytes::new(),
        risk_score: 0,
        requires_instant_clearing: false,
        domestic_settlement_data: None,
        source_account: None,
        destination_account: None,
        original_data_elements: None,
        mac_data: None,
        is_reversal: false,
    }
}

#[derive(Clone)]
pub struct DistConfig {
    pub happy: f64,
    pub timeout: f64,
    pub visa: f64,
    pub mastercard: f64,
    pub apple_pay: f64,
    pub emv: f64,
    pub base24: f64,
    pub connex: f64,
    pub interac: f64,
    pub uk_fps: f64,
    pub iso_1987: f64,
    pub iso_1993: f64,
    pub iso_2003: f64,
}

#[derive(Serialize, Deserialize, Clone)]
#[allow(non_snake_case)]
pub struct Pulse {
    pub id: String,
    pub phase: String,
    pub currentNode: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[allow(non_snake_case)]
pub struct HistoryRecord {
    pub id: String,
    pub timestamp: String,
    pub r#type: String, 
    pub status: String, 
    pub amount: String,
    pub totalDurationMs: u64,
    pub rawIsoHex: String,
    pub responseCode: String,
    pub network: String,
    pub entryMode: String,
    pub dialect: String,
}

pub struct StatusCounters {
    pub running: AtomicBool,
    pub total_target: AtomicUsize,
    pub generated: AtomicUsize,
    pub in_flight: AtomicUsize,
    pub completed: AtomicUsize,
    pub success: AtomicUsize,
    pub timeout: AtomicUsize,
    pub decline: AtomicUsize,
    pub tps: AtomicUsize,
    pub avg_latency_ms: AtomicUsize,
    pub elapsed_ms: AtomicUsize,
}

impl Default for StatusCounters {
    fn default() -> Self {
        Self {
            running: AtomicBool::new(false),
            total_target: AtomicUsize::new(0),
            generated: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
            success: AtomicUsize::new(0),
            timeout: AtomicUsize::new(0),
            decline: AtomicUsize::new(0),
            tps: AtomicUsize::new(0),
            avg_latency_ms: AtomicUsize::new(0),
            elapsed_ms: AtomicUsize::new(0),
        }
    }
}

pub struct SimEngine {
    pub status: StatusCounters,
    pub history: Mutex<VecDeque<HistoryRecord>>,
    pub pulses: dashmap::DashMap<String, Pulse>,
    pub started_at: Mutex<Option<Instant>>,
    pub total_latency_sum: AtomicUsize,
    pub completion_times: Mutex<VecDeque<Instant>>,
    pub cancel_tx: Mutex<Option<tokio::sync::broadcast::Sender<()>>>,
}

impl SimEngine {
    pub fn new() -> Self {
        Self {
            status: StatusCounters::default(),
            history: Mutex::new(VecDeque::with_capacity(100)),
            pulses: dashmap::DashMap::new(),
            started_at: Mutex::new(None),
            total_latency_sum: AtomicUsize::new(0),
            completion_times: Mutex::new(VecDeque::with_capacity(10000)),
            cancel_tx: Mutex::new(None),
        }
    }

    pub fn to_status_json(&self) -> serde_json::Value {
        let mut elapsed = 0;
        if let Some(start) = *self.started_at.lock().unwrap() {
            elapsed = start.elapsed().as_millis() as usize;
        }

        serde_json::json!({
            "running": self.status.running.load(Ordering::Relaxed),
            "totalTarget": self.status.total_target.load(Ordering::Relaxed),
            "generated": self.status.generated.load(Ordering::Relaxed),
            "inFlight": self.status.in_flight.load(Ordering::Relaxed),
            "completed": self.status.completed.load(Ordering::Relaxed),
            "success": self.status.success.load(Ordering::Relaxed),
            "timeout": self.status.timeout.load(Ordering::Relaxed),
            "decline": self.status.decline.load(Ordering::Relaxed),
            "tps": self.status.tps.load(Ordering::Relaxed),
            "avgLatencyMs": self.status.avg_latency_ms.load(Ordering::Relaxed),
            "elapsedMs": elapsed,
        })
    }

    pub fn update_tps(&self) {
        let mut times = self.completion_times.lock().unwrap();
        let now = Instant::now();
        while let Some(t) = times.front() {
            if now.duration_since(*t).as_millis() > 5000 {
                times.pop_front();
            } else {
                break;
            }
        }
        self.status.tps.store(times.len() / 5, Ordering::Relaxed);
    }
}

pub async fn run_worker(
    engine: Arc<SimEngine>,
    config: DistConfig,
    mut cancel_rx: tokio::sync::broadcast::Receiver<()>,
) {
    // Standard forward path for non-tokenized transactions.
    let fwd_nodes_standard = vec!["terminal", "ingress", "core", "hsm", "egress", "bank"];
    // Tokenized (Apple Pay) path: transaction passes through the TSP Vault for detokenization
    // before being routed to the upstream bank. The HSM node is skipped (no PIN).
    let fwd_nodes_tokenized = vec!["terminal", "ingress", "tsp", "core", "egress", "bank"];
    let rev_ok_nodes = vec!["bank", "egress", "core", "ingress", "terminal"];
    let rev_err_nodes = vec!["core", "ingress", "terminal"];
    let mut streams: std::collections::HashMap<u16, tokio::net::TcpStream> = std::collections::HashMap::new();

    loop {
        if !engine.status.running.load(Ordering::Relaxed) {
            break;
        }

        let generated = engine.status.generated.fetch_add(1, Ordering::Relaxed);
        let target = engine.status.total_target.load(Ordering::Relaxed);

        if generated >= target {
            engine.status.generated.fetch_sub(1, Ordering::Relaxed); // Cap it
            break;
        }

        engine.status.in_flight.fetch_add(1, Ordering::Relaxed);

        let rand_val: f64 = rand::thread_rng().gen();
        let tx_type = if rand_val < config.happy {
            "Happy Path"
        } else if rand_val < config.happy + config.timeout {
            "Timeout"
        } else {
            "Decline"
        };
        
        let rand_net: f64 = rand::thread_rng().gen();
        let network = if rand_net < config.visa {
            "Visa"
        } else if rand_net < config.visa + config.mastercard {
            "Mastercard"
        } else {
            "Amex"
        };
        
        let rand_mode: f64 = rand::thread_rng().gen();
        let entry_mode = if rand_mode < config.apple_pay {
            "ApplePay"
        } else if rand_mode < config.apple_pay + config.emv {
            "EMV"
        } else {
            "POS"
        };

        let rand_dialect: f64 = rand::thread_rng().gen();
        let dialect_name = if rand_dialect < config.base24 {
            "Base24"
        } else if rand_dialect < config.base24 + config.connex {
            "Connex"
        } else if rand_dialect < config.base24 + config.connex + config.interac {
            "Interac"
        } else if rand_dialect < config.base24 + config.connex + config.interac + config.uk_fps {
            "UK FPS"
        } else if rand_dialect < config.base24 + config.connex + config.interac + config.uk_fps + config.iso_1987 {
            "ISO 1987"
        } else if rand_dialect < config.base24 + config.connex + config.interac + config.uk_fps + config.iso_1987 + config.iso_1993 {
            "ISO 1993"
        } else {
            "ISO 2003"
        };

        let profile = if tx_type == "Decline" { "Decline" } else { "Happy Path" };

        let id = uuid::Uuid::new_v4().simple().to_string()[0..7].to_string().to_uppercase();
        let rrn_str = uuid::Uuid::new_v4().simple().to_string()[0..12].to_string().to_uppercase();

        let tx_data = generate_transaction(profile, rrn_str, network, entry_mode);
        let amount = tx_data.amount.to_string();

        // Select the correct topology pulse path based on whether this is a tokenized flow.
        // Apple Pay → TSP Vault detokenization path (no HSM — cryptogram replaces PIN).
        // Standard  → HSM PIN-translate path (for POS PIN-entry transactions).
        let fwd_nodes: &Vec<&str> = if tx_data.is_tokenized {
            &fwd_nodes_tokenized
        } else {
            &fwd_nodes_standard
        };
        
        // NATIVE ENCODING HERE - utilizing strict canonical bytes mapped via DialectRouter
        let encoded = if dialect_name == "Base24" || dialect_name == "Interac" || dialect_name == "UK FPS" {
            // For now, load generator just maps Interac to Base24 format dynamically internally, or we can use DialectRouter
            match DialectRouter::Base24(iso_dialect::Base24Dialect).encode(&tx_data) {
                Ok(b) => b,
                Err(_) => {
                    engine.status.in_flight.fetch_sub(1, Ordering::Relaxed);
                    continue;
                }
            }
        } else if dialect_name == "Connex" {
            match DialectRouter::Connex(ConnexDialect).encode(&tx_data) {
                Ok(b) => b,
                Err(_) => {
                    engine.status.in_flight.fetch_sub(1, Ordering::Relaxed);
                    continue;
                }
            }
        } else {
            // New ISO Epoch Versions
            use payment_proto::iso_dialect::DialectAdapter;
            let mut dynamic_tx = tx_data.clone();
            match dialect_name {
                "ISO 1987" => dynamic_tx.mti = bytes::Bytes::from_static(b"0200"),
                "ISO 1993" => dynamic_tx.mti = bytes::Bytes::from_static(b"1200"),
                "ISO 2003" => dynamic_tx.mti = bytes::Bytes::from_static(b"2200"),
                _ => {}
            }
            payment_proto::iso_dialect::StandardDialect.build_response(&dynamic_tx).payload.to_vec().into()
        };
        
        let hex = encoded.iter().map(|b| format!("{:02x}", b)).collect::<String>();
        let start_time = Instant::now();

        // FWD Animation bounds
        for node in fwd_nodes {
            if let Ok(_) = cancel_rx.try_recv() {
                return;
            }
            engine.pulses.insert(
                id.clone(),
                Pulse {
                    id: id.clone(),
                    phase: "req".to_string(),
                    currentNode: node.to_string(),
                },
            );
            tokio::time::sleep(Duration::from_millis(15)).await;
        }

        // Native Dialect TCP Blast Targeting Central Engine Switch Ingress Bounds
        let mut ok = false;
        let mut response_code = String::from("68");
        let encoded_clone = encoded.clone();
        
        let mut target_port = 8000;
        if dialect_name == "Interac" {
            target_port = 8001;
        } else if dialect_name == "UK FPS" {
            target_port = 8002;
        }

        if !streams.contains_key(&target_port) {
            if let Ok(s) = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", target_port)).await {
                let _ = s.set_nodelay(true);
                streams.insert(target_port, s);
            }
        }

        if let Some(stream) = streams.get_mut(&target_port) {
            let len_bytes = (encoded_clone.len() as u16).to_be_bytes();
            let mut payload = Vec::with_capacity(2 + encoded_clone.len());
            payload.extend_from_slice(&len_bytes);
            payload.extend_from_slice(&encoded_clone);

            let net_task = async {
                if tokio::io::AsyncWriteExt::write_all(stream, &payload).await.is_ok() {
                    let mut head = [0u8; 2];
                    if tokio::io::AsyncReadExt::read_exact(stream, &mut head).await.is_ok() {
                        let len = u16::from_be_bytes(head) as usize;
                        let mut body = vec![0u8; len];
                        if tokio::io::AsyncReadExt::read_exact(stream, &mut body).await.is_ok() {
                            let decode_res = if dialect_name == "Base24" || dialect_name == "Interac" || dialect_name == "UK FPS" {
                                DialectRouter::Base24(iso_dialect::Base24Dialect).decode(&body).map_err(|_| ())
                            } else if dialect_name == "Connex" {
                                DialectRouter::Connex(ConnexDialect).decode(&body).map_err(|_| ())
                            } else {
                                use payment_proto::iso_dialect::DialectAdapter;
                                let msg = payment_proto::iso_dialect::Iso8583Message { payload: bytes::Bytes::copy_from_slice(&body) };
                                payment_proto::iso_dialect::StandardDialect.parse(&msg).map_err(|_| ())
                            };
                            match decode_res {
                                Ok(res_canonical) => return Some(res_canonical.response_code.0),
                                Err(_) => return None
                            }
                        }
                    }
                }
                None
            };

            match tokio::time::timeout(Duration::from_millis(20000), net_task).await {
                Ok(Some(rc)) => {
                    response_code = rc;
                    if response_code == "00" { ok = true; }
                }
                _ => {
                    // TCP Timeout or Socket Error
                    streams.remove(&target_port);
                    response_code = "68".to_string();
                }
            }
        } else {
            response_code = "68".to_string();
        }

        let duration_ms = start_time.elapsed().as_millis() as u64;

        // Rev Animation
        let rev_seq = if ok { &rev_ok_nodes } else { &rev_err_nodes };
        for node in rev_seq {
            engine.pulses.insert(
                id.clone(),
                Pulse {
                    id: id.clone(),
                    phase: if ok { "res".to_string() } else { "rev".to_string() },
                    currentNode: node.to_string(),
                },
            );
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        engine.pulses.remove(&id);

        let tx_status = if ok {
            "success"
        } else if response_code == "68" || duration_ms >= 5000 {
            "timeout"
        } else {
            "decline"
        };

        engine.status.in_flight.fetch_sub(1, Ordering::Relaxed);
        let completed_count = engine.status.completed.fetch_add(1, Ordering::Relaxed) + 1;

        if tx_status == "success" {
            engine.status.success.fetch_add(1, Ordering::Relaxed);
        } else if tx_status == "timeout" {
            engine.status.timeout.fetch_add(1, Ordering::Relaxed);
        } else {
            engine.status.decline.fetch_add(1, Ordering::Relaxed);
        }

        engine.total_latency_sum.fetch_add(duration_ms as usize, Ordering::Relaxed);
        engine.status.avg_latency_ms.store(
            engine.total_latency_sum.load(Ordering::Relaxed) / completed_count,
            Ordering::Relaxed,
        );

        let mut times = engine.completion_times.lock().unwrap();
        times.push_back(Instant::now());
        drop(times);
        engine.update_tps();

        let record = HistoryRecord {
            id: id.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            r#type: tx_type.to_string(),
            status: tx_status.to_string(),
            amount,
            totalDurationMs: duration_ms,
            rawIsoHex: hex,
            responseCode: response_code,
            network: network.to_string(),
            entryMode: entry_mode.to_string(),
            dialect: dialect_name.to_string(),
        };

        let mut h = engine.history.lock().unwrap();
        h.push_front(record);
        if h.len() > 100 {
            h.pop_back();
        }

        engine.pulses.remove(&id);
    }
}
