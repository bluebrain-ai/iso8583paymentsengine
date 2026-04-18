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
        .route("/stream/logs", get(ws_handler))
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:9000").await.unwrap();
    tracing::info!("Test orchestrator listening on 0.0.0.0:9000");
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
    
    let mut child = Command::new("cargo")
        .arg("run")
        .arg("--bin")
        .arg(&service_name)
        .current_dir("d:/cob2java/GitHub/ISOPaymentEngine")
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
    if let Some(mut child) = processes.remove(&service_name) {
        let _ = child.kill().await;
        let _ = state.log_tx.send(format!("[{}] was killed by orchestrator", service_name));
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
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
    
    if let Ok(mut stream) = TcpStream::connect("127.0.0.1:8000").await {
        if stream.write_all(&buf).await.is_ok() {
            return StatusCode::OK;
        }
    }
    StatusCode::SERVICE_UNAVAILABLE
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
