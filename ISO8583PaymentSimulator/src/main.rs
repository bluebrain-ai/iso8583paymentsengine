mod engine;

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use engine::SimEngine;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Instant;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

#[derive(Deserialize)]
struct StartRequest {
    volume: Option<usize>,
    concurrency: Option<usize>,
    dist: Option<Distribution>,
}

#[derive(Deserialize)]
struct Distribution {
    happy: Option<f64>,
    timeout: Option<f64>,
    decline: Option<f64>,
}

async fn start_sim(
    State(state): State<Arc<SimEngine>>,
    Json(payload): Json<StartRequest>,
) -> impl IntoResponse {
    if state.status.running.load(std::sync::atomic::Ordering::Relaxed) {
        return (StatusCode::CONFLICT, Json(serde_json::json!({ "error": "Already running" })));
    }

    let volume = payload.volume.unwrap_or(100).max(1);
    let concurrency = payload.concurrency.unwrap_or(10).max(1);
    let dist = payload.dist.unwrap_or(Distribution {
        happy: Some(0.8),
        timeout: Some(0.1),
        decline: Some(0.1),
    });

    let happy = dist.happy.unwrap_or(0.8);
    let timeout = dist.timeout.unwrap_or(0.1);

    // Reset Engine State
    state.status.running.store(true, std::sync::atomic::Ordering::Relaxed);
    state.status.total_target.store(volume, std::sync::atomic::Ordering::Relaxed);
    state.status.generated.store(0, std::sync::atomic::Ordering::Relaxed);
    state.status.in_flight.store(0, std::sync::atomic::Ordering::Relaxed);
    state.status.completed.store(0, std::sync::atomic::Ordering::Relaxed);
    state.status.success.store(0, std::sync::atomic::Ordering::Relaxed);
    state.status.timeout.store(0, std::sync::atomic::Ordering::Relaxed);
    state.status.decline.store(0, std::sync::atomic::Ordering::Relaxed);
    state.status.tps.store(0, std::sync::atomic::Ordering::Relaxed);
    state.status.avg_latency_ms.store(0, std::sync::atomic::Ordering::Relaxed);
    
    *state.history.lock().unwrap() = std::collections::VecDeque::new();
    *state.completion_times.lock().unwrap() = std::collections::VecDeque::new();
    state.total_latency_sum.store(0, std::sync::atomic::Ordering::Relaxed);
    *state.started_at.lock().unwrap() = Some(Instant::now());
    state.pulses.clear();

    let (cancel_tx, _) = tokio::sync::broadcast::channel(1);
    *state.cancel_tx.lock().unwrap() = Some(cancel_tx.clone());

    for _ in 0..concurrency {
        let e = Arc::clone(&state);
        let rx = cancel_tx.subscribe();
        tokio::spawn(async move {
            engine::run_worker(e, happy, timeout, rx).await;
        });
    }

    let e = Arc::clone(&state);
    tokio::spawn(async move {
        // Wait until generated hits target
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            if !e.status.running.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            if e.status.completed.load(std::sync::atomic::Ordering::Relaxed) >= volume {
                e.status.running.store(false, std::sync::atomic::Ordering::Relaxed);
                break;
            }
        }
    });

    (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

async fn cancel_sim(State(state): State<Arc<SimEngine>>) -> impl IntoResponse {
    state.status.running.store(false, std::sync::atomic::Ordering::Relaxed);
    if let Some(tx) = state.cancel_tx.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
    (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

async fn get_status(State(state): State<Arc<SimEngine>>) -> impl IntoResponse {
    Json(state.to_status_json())
}

async fn get_history(State(state): State<Arc<SimEngine>>) -> impl IntoResponse {
    let hist = state.history.lock().unwrap();
    let vec: Vec<_> = hist.iter().cloned().collect();
    Json(vec)
}

async fn get_pulses(State(state): State<Arc<SimEngine>>) -> impl IntoResponse {
    let mut pulses = Vec::new();
    for entry in state.pulses.iter() {
        pulses.push(entry.value().clone());
    }
    Json(pulses)
}

async fn proxy_dashmap() -> impl IntoResponse {
    match reqwest::get("http://localhost:8080/api/dashmap").await {
        Ok(res) => {
            if let Ok(json) = res.json::<serde_json::Value>().await {
                return (StatusCode::OK, Json(json));
            }
        }
        Err(_) => {}
    }
    (StatusCode::OK, Json(serde_json::json!({ "active_transactions": [], "total_guarded": 0 })))
}

async fn proxy_journal() -> impl IntoResponse {
    match reqwest::get("http://localhost:8080/api/journal").await {
        Ok(res) => {
            if let Ok(json) = res.json::<serde_json::Value>().await {
                return (StatusCode::OK, Json(json));
            }
        }
        Err(_) => {}
    }
    (StatusCode::OK, Json(serde_json::json!({ "journal_events": [] })))
}

#[tokio::main]
async fn main() {
    let engine = Arc::new(SimEngine::new());

    let app = Router::new()
        .route("/sim/start", post(start_sim))
        .route("/sim/cancel", post(cancel_sim))
        .route("/sim/status", get(get_status))
        .route("/sim/history", get(get_history))
        .route("/sim/pulses", get(get_pulses))
        .route("/api/dashmap", get(proxy_dashmap))
        .route("/api/journal", get(proxy_journal))
        .nest_service("/", ServeDir::new("."))
        .layer(CorsLayer::permissive())
        .with_state(engine);

    println!("\n╔══════════════════════════════════════════════╗");
    println!("║   IsoSwitch Payment Command Center           ║");
    println!("║   → http://localhost:3001                    ║");
    println!("║   Sim Engine: Rust Native Axum (this process)║");
    println!("║   Rust Daemon expected on TCP :8000          ║");
    println!("║   Telemetry expected on HTTP :8080           ║");
    println!("╚══════════════════════════════════════════════╝\n");

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3001").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
