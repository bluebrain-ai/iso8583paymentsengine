use axum::{
    extract::{Path, State, WebSocketUpgrade, ws::{Message, WebSocket}},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::{
    process::Command,
    sync::{broadcast, Mutex},
    io::{AsyncBufReadExt, BufReader},
    net::TcpStream,
    io::AsyncWriteExt,
};
use tower_http::cors::{Any, CorsLayer};

struct AppState {
    processes: Mutex<HashMap<String, tokio::process::Child>>,
    log_tx: broadcast::Sender<String>,
}

#[derive(Serialize, Deserialize)]
struct InjectPayload {
    hex: String,
    port: Option<u16>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    
    let (log_tx, _) = broadcast::channel(1000);
    let state = Arc::new(AppState {
        processes: Mutex::new(HashMap::new()),
        log_tx,
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/process/start/:service_name", post(start_process))
        .route("/api/process/kill/:service_name", post(kill_process))
        .route("/api/inject/raw", post(inject_raw))
        .route("/api/process/scenario/:id", post(run_scenario))
        .route("/api/process/status", get(get_status))
        .route("/stream/logs", get(ws_handler))
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:9001").await.unwrap();
    tracing::info!("Test orchestrator listening on 0.0.0.0:9001");
    axum::serve(listener, app).await.unwrap();
}

async fn start_process(
    Path(service_name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<StatusCode, StatusCode> {
    let mut processes = state.processes.lock().await;
    
    if processes.contains_key(&service_name) {
        return Ok(StatusCode::ALREADY_REPORTED);
    }
    
    let working_dir = match service_name.as_str() {
        "payment-daemon" => "d:/cob2java/GitHub/ISOPaymentEngine/ISO8583PaymentEngine",
        "mock-bank-node" => "d:/cob2java/GitHub/ISOPaymentEngine/ISO8583PaymentSimulator",
        "mock_hsm" => "d:/cob2java/GitHub/ISOPaymentEngine/ISO8583PaymentSimulator",
        _ => "d:/cob2java/GitHub/ISOPaymentEngine",
    };

    let cargo_flag = if service_name == "mock-bank-node" { "-p" } else { "--bin" };

    let mut child = Command::new("cargo")
        .arg("run")
        .arg(cargo_flag)
        .arg(&service_name)
        .current_dir(working_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let tx = state.log_tx.clone();
    let service_name_clone = service_name.clone();
    
    if let Some(stdout) = child.stdout.take() {
        let tx = tx.clone();
        let svc = service_name_clone.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = tx.send(format!("[{}] [OUT] {}", svc, line));
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let tx = tx.clone();
        let svc = service_name_clone.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = tx.send(format!("[{}] [ERR] {}", svc, line));
            }
        });
    }

    processes.insert(service_name, child);
    Ok(StatusCode::CREATED)
}

async fn kill_process(
    Path(service_name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> StatusCode {
    let mut processes = state.processes.lock().await;
    let _ = state.log_tx.send(format!("[{}] Received manual chaos target kill request...", service_name));

    let exe_name = format!("{}.exe", service_name);
    let _ = state.log_tx.send(format!("[{}] Firing native taskkill /F /IM {} ...", service_name, exe_name));
    
    let kill_cmd = Command::new("taskkill")
        .arg("/F")
        .arg("/IM")
        .arg(&exe_name)
        .output()
        .await;

    if let Ok(output) = kill_cmd {
        if output.status.success() {
            let _ = state.log_tx.send(format!("[{} KILL] SUCCESS: The OS actively severed the process.", service_name));
        } else {
            let _ = state.log_tx.send(format!("[{} KILL] WARNING: The OS reported process not found or no rights.", service_name));
        }
    } else {
        let _ = state.log_tx.send(format!("[{} KILL] ERROR: Failed to invoke host taskkill utility.", service_name));
    }

    if let Some(mut child) = processes.remove(&service_name) {
        let _ = child.kill().await;
    }
    
    StatusCode::OK
}

async fn inject_raw(
    State(_state): State<Arc<AppState>>,
    Json(payload): Json<InjectPayload>,
) -> StatusCode {
    let payload_bytes = match hex::decode(&payload.hex) {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    
    let length = payload_bytes.len() as u16;
    let mut buf = length.to_be_bytes().to_vec();
    buf.extend_from_slice(&payload_bytes);
    
    let target_port = payload.port.unwrap_or(8000);
    if let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{}", target_port)).await {
        if stream.write_all(&buf).await.is_ok() {
            return StatusCode::OK;
        }
    }
    StatusCode::SERVICE_UNAVAILABLE
}

async fn get_status() -> Json<HashMap<String, String>> {
    let mut map = HashMap::new();
    for target in ["mock-bank-node", "payment-daemon", "mock_hsm"] {
        let mut target_status = "KILLED";
        let exe_name = format!("{}.exe", target);
        if let Ok(output) = tokio::process::Command::new("tasklist").arg("/NH").arg("/FI").arg(format!("IMAGENAME eq {}", exe_name)).output().await {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains(&exe_name) {
                target_status = "RUNNING";
            }
        }
        map.insert(target.to_string(), target_status.to_string());
    }
    Json(map)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.log_tx.subscribe();
    
    while let Ok(msg) = rx.recv().await {
        if socket.send(Message::Text(msg.into())).await.is_err() {
            break;
        }
    }
}

#[derive(Deserialize, Default)]
struct ScenarioConfig {
    volume: Option<usize>,
    concurrency: Option<usize>,
}

async fn run_scenario(
    Path(id): Path<String>, 
    State(state): State<Arc<AppState>>,
    payload: Option<Json<ScenarioConfig>>
) -> StatusCode {
    let config = payload.map(|p| p.0).unwrap_or_default();
    let mut processes = state.processes.lock().await;
    let _ = state.log_tx.send(format!("[SCENARIO] Triggering Scenario {}...", id));
    match id.as_str() {
        "1" => {
            let volume = config.volume.unwrap_or(10000);
            let concurrency = config.concurrency.unwrap_or(20);
            let _ = state.log_tx.send(format!("[SCENARIO 1] Baseline Stress Test: Spawning {} TPS Baseline via Fast-Load Generator on {} threads!", volume, concurrency));
            tokio::spawn(async move {
                let client = reqwest::Client::new();
                let req_payload = serde_json::json!({
                    "volume": volume,
                    "concurrency": concurrency
                });
                let _ = client.post("http://127.0.0.1:3005/sim/start").json(&req_payload).send().await;
            });
        },
        "2" => {
            if let Some(mut child) = processes.remove("mock-bank-node") {
                let _ = child.kill().await;
                let _ = state.log_tx.send("[SCENARIO 2] Egress Blackout: mock-bank-node forcefully terminated!".into());
            } else {
                let _ = state.log_tx.send("[SCENARIO 2] Egress Blackout: mock-bank-node is not running".into());
            }
        },
        "3" => {
            let _ = state.log_tx.send("[SCENARIO 3] HSM Latency Trap: Not implemented dynamically for this iteration.".into());
        },
        "4" => {
            let _ = state.log_tx.send("[SCENARIO 4] Poison Pill: Emitting corrupted ISO-8583 MTI byte-sequence!".into());
            tokio::spawn(async {
                if let Ok(mut stream) = tokio::net::TcpStream::connect("127.0.0.1:8000").await {
                    use tokio::io::AsyncWriteExt;
                    let bad_payload = vec![0x00, 0x05, b'X', b'X', b'X', b'X', b'X'];
                    let _ = stream.write_all(&bad_payload).await;
                }
            });
        },
        _ => return StatusCode::NOT_FOUND,
    }
    StatusCode::OK
}
