use switch_core::engine::SwitchEngine;
use switch_core::journal::Journaler;
use switch_core::telemetry::TelemetryLogger;
use axum::{routing::{get, post}, Router, Json, extract::State};
use switch_core::state::{CrdbEngine, StipEngine};
use std::sync::Arc;
mod network_manager;

#[derive(Clone)]
struct AppState {
    guard: Arc<dashmap::DashMap<String, switch_core::context::ActiveTransaction>>,
    memory_state: Arc<switch_core::state::GlobalState>,
}

#[tokio::main]
async fn main() {
    switch_core::telemetry::init_telemetry();

    println!("============================================================");
    println!("     ISO-8583 RUST PAYMENT SWITCH ENGINE INITIATING...      ");
    println!("============================================================");

    // Initialize Telemetry
    let telemetry = TelemetryLogger::new();

    // Initialize 100MB WAL Array 
    let journaler = Journaler::new("live_payment_wal.log", 100 * 1024 * 1024).expect("Failed to initialize system memory-mapped logger! Do you have hard-disk permission?");
    
    // Boot the Multi-Threaded SEDA Core Engine
    // High-Throughput 1500ms System-Wide Timeout Fallbacks matching STIP bounds
    let engine = SwitchEngine::start(journaler, telemetry.clone(), 1500); 

    // Retrieve the crossbeam SEDA channels exposing bounds natively
    let ingress_tx = engine.ingress();
    let visa_rx = engine.visa_egress();
    let mastercard_rx = engine.mastercard_egress();
    let bank_reply_tx = engine.bank_reply();

    // Spawn Background Network Manager (0800 Pings)
    let safe_state = engine.memory_state.clone();
    let safe_guard = engine.idempotency_guard.clone();
    let safe_telemetry = telemetry.clone();
    tokio::spawn(async move {
        network_manager::start_network_manager(safe_state, safe_guard, safe_telemetry).await;
    });

    // Spawn Backend Axum Telemetry API
    let app_state = AppState {
        guard: engine.idempotency_guard.clone(),
        memory_state: engine.memory_state.clone(),
    };
    let app = Router::new()
        .route("/api/dashmap", get(get_dashmap))
        .route("/api/journal", get(get_journal))
        .route("/admin/hot-reload/crdb", post(hot_reload_crdb))
        .route("/admin/hot-reload/stip", post(hot_reload_stip))
        .route("/admin/hot-reload/routes", post(hot_reload_routes))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(app_state);

    tokio::spawn(async move {
        if let Ok(listener) = tokio::net::TcpListener::bind("0.0.0.0:8080").await {
            println!("[INFO] Live Telemetry Axum API listening on http://0.0.0.0:8080");
            let _ = axum::serve(listener, app).await;
        }
    });

    // Boot the SAF Database Queue
    let saf_db = sled::open("saf_queue.db").expect("Failed to bind SAF Database");

    // Boot Real Visa Upstream TCP Client
    let visa_client = edge_egress::client::UpstreamClient {
        host: "127.0.0.1:9188".to_string(), // Mapped to mock-bank-node for local dev
        rx: visa_rx,
        saf_timeout_ms: 10_000,
        saf_db: saf_db.clone(),
        bank_reply_tx: bank_reply_tx.clone(),
    };
    tokio::spawn(async move {
        visa_client.run().await;
    });

    // Boot Real Mastercard Upstream TCP Client
    let mc_client = edge_egress::client::UpstreamClient {
        host: "127.0.0.1:9200".to_string(), // Mapped to mock-bank-node
        rx: mastercard_rx,
        saf_timeout_ms: 10_000,
        saf_db: saf_db.clone(),
        bank_reply_tx: bank_reply_tx.clone(),
    };
    tokio::spawn(async move {
        mc_client.run().await;
    });

    println!("[INFO] Dispatching Egress Real Clients globally...");

    println!("[INFO] Deploying Edge Ingress Server bounded heavily on localhost:8000!");
    if let Ok(listener) = tokio::net::TcpListener::bind("127.0.0.1:8000").await {
        edge_ingress::server::run_server(listener, ingress_tx).await;
    } else {
        println!("[FATAL] Core SEDA Ingress failed to bind to port.");
    }
}

async fn get_dashmap(State(app_state): State<AppState>) -> Json<serde_json::Value> {
    let mut transactions = Vec::new();
    for entry in app_state.guard.iter() {
        let tx = entry.value();
        transactions.push(serde_json::json!({
            "key": entry.key(),
            "state": tx.state,
            "created_at_ms": tx.created_at.elapsed().as_millis(),
            "amount": tx.transaction_data.amount,
            "mti": tx.transaction_data.mti,
            "rrn": tx.transaction_data.rrn,
        }));
    }
    Json(serde_json::json!({ "active_transactions": transactions, "total_guarded": transactions.len() }))
}

async fn get_journal() -> Json<serde_json::Value> {
    let mut events = Vec::new();
    if let Ok(file) = std::fs::File::open("live_payment_wal.log") {
        if let Ok(mmap) = unsafe { memmap2::Mmap::map(&file) } {
            let mut offset = 0;
            while offset + 4 <= mmap.len() {
                let len = u32::from_be_bytes([mmap[offset], mmap[offset+1], mmap[offset+2], mmap[offset+3]]) as usize;
                offset += 4;
                if len == 0 || offset + len > mmap.len() {
                    break;
                }
                use prost::Message;
                if let Ok(tx) = payment_proto::Transaction::decode(&mmap[offset..offset+len]) {
                    events.push(serde_json::json!({
                        "mti": tx.mti,
                        "rrn": tx.rrn,
                        "amount": tx.amount,
                        "response_code": tx.response_code
                    }));
                }
                offset += len;
            }
        }
    }
    
    // Reverse and limit to massive frontend crashing
    let recent: Vec<_> = events.into_iter().rev().take(100).collect();
    Json(serde_json::json!({ "journal_events": recent }))
}

#[derive(serde::Deserialize)]
pub struct RouteRow {
    pub bin_prefix: String,
    pub destination_type: String, // "InternalCrdb" or "ExternalNode"
    pub target_node: Option<String>,
    pub failover_node: Option<String>,
}

#[derive(serde::Deserialize)]
struct PublishCrdb {
    cards: std::collections::HashMap<String, switch_core::state::CardRecord>,
}

#[derive(serde::Deserialize)]
struct PublishStip {
    rules: std::collections::HashMap<String, switch_core::state::StipRule>,
}

#[derive(serde::Deserialize)]
struct PublishRoutes {
    routing_rules: Vec<RouteRow>,
}

async fn hot_reload_crdb(State(app_state): State<AppState>, Json(payload): Json<PublishCrdb>) -> Json<serde_json::Value> {
    println!("[HOT-RELOAD] Valid CRDB payload received. Commencing zero-latency lockless pointer swap...");
    let engine = CrdbEngine { cards: payload.cards };
    app_state.memory_state.crdb.store(Arc::new(engine));
    println!("[HOT-RELOAD] Active pipeline successfully switched to new CRDB boundaries.");
    Json(serde_json::json!({ "status": "success", "message": "Live memory pointer swap accomplished" }))
}

async fn hot_reload_stip(State(app_state): State<AppState>, Json(payload): Json<PublishStip>) -> Json<serde_json::Value> {
    println!("[HOT-RELOAD] Valid STIP payload received. Commencing zero-latency lockless pointer swap...");
    let engine = StipEngine { rules: payload.rules };
    app_state.memory_state.stip.store(Arc::new(engine));
    println!("[HOT-RELOAD] Active pipeline successfully switched to new STIP boundaries.");
    Json(serde_json::json!({ "status": "success", "message": "Live memory pointer swap accomplished" }))
}

async fn hot_reload_routes(State(app_state): State<AppState>, Json(payload): Json<PublishRoutes>) -> Json<serde_json::Value> {
    println!("[HOT-RELOAD] Valid Routing payload received. Commencing zero-latency lockless pointer swap...");
    
    // Compile Radix Trie
    let mut trie = switch_core::routing::BinTrie::new();
    for row in payload.routing_rules {
        let dest = if row.destination_type == "InternalCrdb" {
            switch_core::routing::RouteDestination::InternalCrdb
        } else {
            if let Some(target) = row.target_node {
                switch_core::routing::RouteDestination::ExternalNode(target)
            } else {
                continue; // invalid
            }
        };

        let route = switch_core::routing::Route {
            destination: dest,
            failover_node: row.failover_node,
        };
        trie.insert(row.bin_prefix.as_bytes(), route);
    }
    
    app_state.memory_state.router.store(Arc::new(trie));
    println!("[HOT-RELOAD] Active pipeline successfully switched to new Matrix Routing rules.");

    Json(serde_json::json!({ "status": "success", "message": "Live memory pointer swap accomplished" }))
}

