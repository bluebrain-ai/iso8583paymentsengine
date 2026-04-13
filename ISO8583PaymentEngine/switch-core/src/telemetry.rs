use crossbeam::channel::{bounded, Sender};
use std::thread;

pub enum TelemetryEvent {
    EngineStarted { workers: usize },
    TransactionReceived { rrn: String, stan: String, acquirer_id: String },
    DuplicateDetected { key: String },
    Journaled { rrn: String, size_bytes: usize },
    RoutingFailed { rrn: String },
    Routed { rrn: String },
    LateBankReplyDropped { rrn: String },
    StipFallbackAuthorized { rrn: String, amount: u64 },
    StipFallbackDeclined { rrn: String, reason: String },
}

#[derive(Clone)]
pub struct TelemetryLogger {
    tx: Sender<TelemetryEvent>,
}

impl TelemetryLogger {
    pub fn new() -> Self {
        let (tx, rx) = bounded::<TelemetryEvent>(100_000);

        thread::spawn(move || {
            for event in rx {
                match event {
                    TelemetryEvent::EngineStarted { workers } => {
                        println!("[INFO] SEDA Engine started with {} MPMC workers", workers);
                    }
                    TelemetryEvent::TransactionReceived { rrn, stan, acquirer_id } => {
                        println!("[DEBUG] Received Tx -> RRN: {} STAN: {} Acquirer: {}", rrn, stan, acquirer_id);
                    }
                    TelemetryEvent::DuplicateDetected { key } => {
                        println!("[WARN] Idempotency Guard BLOCKED duplicate: {}", key);
                    }
                    TelemetryEvent::Journaled { rrn, size_bytes } => {
                        println!("[INFO] Journaled RRN: {} ({} bytes)", rrn, size_bytes);
                    }
                    TelemetryEvent::RoutingFailed { rrn } => {
                        println!("[ERROR] Egress bounds full, dropped RRN: {}", rrn);
                    }
                    TelemetryEvent::Routed { rrn } => {
                        println!("[INFO] Routed Tx -> RRN: {}", rrn);
                    }
                    TelemetryEvent::LateBankReplyDropped { rrn } => {
                        println!("[WARN] Phantom Approval Reversed & Dropped! RRN: {}", rrn);
                    }
                    TelemetryEvent::StipFallbackAuthorized { rrn, amount } => {
                        println!("[STIP] Offline Fallback AUTHORIZED: RRN: {} | Confirmed: ${}", rrn, amount);
                    }
                    TelemetryEvent::StipFallbackDeclined { rrn, reason } => {
                        println!("[STIP] Offline Fallback DECLINED: RRN: {} | Reason: {}", rrn, reason);
                    }
                }
            }
        });

        Self { tx }
    }

    pub fn log(&self, event: TelemetryEvent) {
        if let Err(e) = self.tx.try_send(event) {
            eprintln!("Telemetry buffer full: {}", e);
        }
    }
}

pub fn init_telemetry() {
    use tracing_subscriber::{fmt, EnvFilter};
    use metrics_exporter_prometheus::PrometheusBuilder;

    // Initialize Tracing Subscriber
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    
    fmt()
        .with_env_filter(filter)
        .init();

    // Initialize Prometheus Exporter
    PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], 9090))
        .install()
        .expect("Failed to install Prometheus exporter");
        
    tracing::info!("Telemetry & Tracing Configuration Activated on port 9090");
}
