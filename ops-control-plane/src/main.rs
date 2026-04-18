use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::{SqliteConnectOptions, SqlitePoolOptions}, SqlitePool};
use std::str::FromStr;
use std::{collections::HashMap, env, sync::Arc};
use tower_http::cors::{Any, CorsLayer};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let db_url = env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://ops.db".to_string());
    
    let connect_options = SqliteConnectOptions::from_str(&db_url)
        .unwrap()
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_options)
        .await
        .expect("Failed to connect to SQLite database.");

    // Automatically initialize schema correctly on startup without Docker!
    sqlx::query(include_str!("../schema.sql"))
        .execute(&pool)
        .await
        .expect("Failed to execute schema initialization.");

    let app_state = Arc::new(AppState {
        db: pool,
        http_client: Client::new(),
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/crdb", get(get_active_crdb_records).post(create_crdb_record))
        .route("/api/crdb/:id", put(update_crdb_record))
        .route("/api/stip", get(get_active_stip_rules).post(create_stip_rule))
        .route("/api/stip/:id", put(update_stip_rule))
        .route("/api/routes", get(get_active_routes).post(create_route))
        .route("/api/routes/:id", put(update_route))
        .route("/api/publish/crdb", post(publish_crdb))
        .route("/api/publish/stip", post(publish_stip))
        .route("/api/publish/routes", post(publish_routes))
        .layer(cors)
        .with_state(app_state);

    let addr = "0.0.0.0:3003";
    tracing::info!("Control Plane API listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

struct AppState {
    db: SqlitePool,
    http_client: Client,
}

// --- DB Models ---

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
struct CrdbRecord {
    id: i32,
    pan_hash: String,
    daily_limit: i64,
    status: String,
    version: i32,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    is_live: bool,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
struct StipRuleRow {
    id: i32,
    network_id: String,
    condition_type: String,
    max_approval_amount: i64,
    max_risk_score: i32,
    version: i32,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    is_live: bool,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
struct RouteRow {
    id: i32,
    bin_prefix: String,
    destination_type: String,
    target_node: Option<String>,
    failover_node: Option<String>,
    version: i32,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    is_live: bool,
}

// --- Request Payloads ---

#[derive(Deserialize)]
struct CreateCrdbReq {
    pan_hash: String,
    daily_limit: i64,
    status: String,
}

#[derive(Deserialize)]
struct UpdateCrdbReq {
    daily_limit: i64,
    status: String,
}

#[derive(Deserialize)]
struct CreateStipReq {
    network_id: String,
    condition_type: String,
    max_approval_amount: i64,
    #[serde(default = "default_max_risk_score")]
    max_risk_score: i32,
}

fn default_max_risk_score() -> i32 { 100 }

#[derive(Deserialize)]
struct UpdateStipReq {
    max_approval_amount: i64,
    #[serde(default = "default_max_risk_score")]
    max_risk_score: i32,
}

#[derive(Deserialize)]
struct CreateRouteReq {
    bin_prefix: String,
    destination_type: String,
    target_node: Option<String>,
    failover_node: Option<String>,
}

#[derive(Deserialize)]
struct UpdateRouteReq {
    destination_type: String,
    target_node: Option<String>,
    failover_node: Option<String>,
}

// --- Shared Engines (Matching Payment Daemon) ---

#[derive(Serialize)]
struct StipRule {
    action: String,
    max_risk_score: u8,
}

#[derive(Serialize)]
struct CardRecord {
    balance: f64,
    is_blocked: bool,
    pvv: String,
}

#[derive(Serialize)]
struct StipEngine {
    rules: HashMap<String, StipRule>,
}

#[derive(Serialize)]
struct CrdbEngine {
    cards: HashMap<String, CardRecord>,
}

#[derive(Serialize)]
struct RoutesEngine {
    routing_rules: Vec<RouteRow>,
}

// --- Endpoints ---

async fn get_active_crdb_records(State(state): State<Arc<AppState>>) -> Json<Vec<CrdbRecord>> {
    let records = sqlx::query_as::<_, CrdbRecord>("SELECT * FROM crdb_records WHERE is_live = 1 ORDER BY id DESC")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    Json(records)
}

async fn create_crdb_record(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateCrdbReq>,
) -> StatusCode {
    let res = sqlx::query(
        "INSERT INTO crdb_records (pan_hash, daily_limit, status, version, is_live) VALUES (?, ?, ?, 1, 1)"
    )
    .bind(&payload.pan_hash)
    .bind(payload.daily_limit)
    .bind(&payload.status)
    .execute(&state.db)
    .await;

    match res {
        Ok(_) => StatusCode::CREATED,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn update_crdb_record(
    Path(id): Path<i32>,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UpdateCrdbReq>,
) -> StatusCode {
    let mut tx = state.db.begin().await.unwrap();

    let old_record = sqlx::query_as::<_, CrdbRecord>("SELECT * FROM crdb_records WHERE id = ? AND is_live = 1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .unwrap();

    if let Some(record) = old_record {
        // Soft delete
        sqlx::query("UPDATE crdb_records SET is_live = 0, valid_to = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .unwrap();

        // Insert new version
        sqlx::query(
            "INSERT INTO crdb_records (pan_hash, daily_limit, status, version, is_live) VALUES (?, ?, ?, ?, 1)"
        )
        .bind(&record.pan_hash)
        .bind(payload.daily_limit)
        .bind(&payload.status)
        .bind(record.version + 1)
        .execute(&mut *tx)
        .await
        .unwrap();

        tx.commit().await.unwrap();
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn get_active_stip_rules(State(state): State<Arc<AppState>>) -> Json<Vec<StipRuleRow>> {
    let rules = sqlx::query_as::<_, StipRuleRow>("SELECT * FROM stip_rules WHERE is_live = 1 ORDER BY id DESC")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    Json(rules)
}

async fn create_stip_rule(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateStipReq>,
) -> StatusCode {
    let res = sqlx::query(
        "INSERT INTO stip_rules (network_id, condition_type, max_approval_amount, max_risk_score, version, is_live) VALUES (?, ?, ?, ?, 1, 1)"
    )
    .bind(&payload.network_id)
    .bind(&payload.condition_type)
    .bind(payload.max_approval_amount)
    .bind(payload.max_risk_score)
    .execute(&state.db)
    .await;

    match res {
        Ok(_) => StatusCode::CREATED,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn update_stip_rule(
    Path(id): Path<i32>,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UpdateStipReq>,
) -> StatusCode {
    let mut tx = state.db.begin().await.unwrap();

    let old_record = sqlx::query_as::<_, StipRuleRow>("SELECT * FROM stip_rules WHERE id = ? AND is_live = 1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .unwrap();

    if let Some(record) = old_record {
        sqlx::query("UPDATE stip_rules SET is_live = 0, valid_to = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO stip_rules (network_id, condition_type, max_approval_amount, max_risk_score, version, is_live) VALUES (?, ?, ?, ?, ?, 1)"
        )
        .bind(&record.network_id)
        .bind(&record.condition_type)
        .bind(payload.max_approval_amount)
        .bind(payload.max_risk_score)
        .bind(record.version + 1)
        .execute(&mut *tx)
        .await
        .unwrap();

        tx.commit().await.unwrap();
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn get_active_routes(State(state): State<Arc<AppState>>) -> Json<Vec<RouteRow>> {
    let routes = sqlx::query_as::<_, RouteRow>("SELECT * FROM routing_rules WHERE is_live = 1 ORDER BY id DESC")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    Json(routes)
}

async fn create_route(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateRouteReq>,
) -> StatusCode {
    let res = sqlx::query(
        "INSERT INTO routing_rules (bin_prefix, destination_type, target_node, failover_node, version, is_live) VALUES (?, ?, ?, ?, 1, 1)"
    )
    .bind(&payload.bin_prefix)
    .bind(&payload.destination_type)
    .bind(&payload.target_node)
    .bind(&payload.failover_node)
    .execute(&state.db)
    .await;

    match res {
        Ok(_) => StatusCode::CREATED,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn update_route(
    Path(id): Path<i32>,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UpdateRouteReq>,
) -> StatusCode {
    let mut tx = state.db.begin().await.unwrap();

    let old_record = sqlx::query_as::<_, RouteRow>("SELECT * FROM routing_rules WHERE id = ? AND is_live = 1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .unwrap();

    if let Some(record) = old_record {
        sqlx::query("UPDATE routing_rules SET is_live = 0, valid_to = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO routing_rules (bin_prefix, destination_type, target_node, failover_node, version, is_live) VALUES (?, ?, ?, ?, ?, 1)"
        )
        .bind(&record.bin_prefix)
        .bind(&payload.destination_type)
        .bind(&payload.target_node)
        .bind(&payload.failover_node)
        .bind(record.version + 1)
        .execute(&mut *tx)
        .await
        .unwrap();

        tx.commit().await.unwrap();
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn publish_crdb(State(state): State<Arc<AppState>>) -> StatusCode {
    let crdb_records = sqlx::query_as::<_, CrdbRecord>("SELECT * FROM crdb_records WHERE is_live = 1")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    let mut crdb_engine = CrdbEngine { cards: HashMap::new() };
    for r in crdb_records {
        crdb_engine.cards.insert(r.pan_hash, CardRecord {
            balance: r.daily_limit as f64,
            is_blocked: r.status == "Blocked",
            pvv: "1234".to_string(), // Injected default PVV to satisfy strict Switch boundary
        });
    }

    let res = state.http_client.post("http://127.0.0.1:8080/admin/hot-reload/crdb")
        .json(&crdb_engine)
        .send()
        .await;

    match res {
        Ok(resp) if resp.status().is_success() => StatusCode::OK,
        _ => StatusCode::BAD_GATEWAY,
    }
}

async fn publish_stip(State(state): State<Arc<AppState>>) -> StatusCode {
    let stip_rules = sqlx::query_as::<_, StipRuleRow>("SELECT * FROM stip_rules WHERE is_live = 1")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    let mut stip_engine = StipEngine { rules: HashMap::new() };
    for r in stip_rules {
        let key = r.network_id;
        stip_engine.rules.insert(key, StipRule {
            action: format!("APPROVE_UP_TO_{}", r.max_approval_amount),
            max_risk_score: r.max_risk_score.clamp(0, 100) as u8,
        });
    }

    let res = state.http_client.post("http://127.0.0.1:8080/admin/hot-reload/stip")
        .json(&stip_engine)
        .send()
        .await;

    match res {
        Ok(resp) if resp.status().is_success() => StatusCode::OK,
        _ => StatusCode::BAD_GATEWAY,
    }
}

async fn publish_routes(State(state): State<Arc<AppState>>) -> StatusCode {
    let routing_rules = sqlx::query_as::<_, RouteRow>("SELECT * FROM routing_rules WHERE is_live = 1")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    let payload = RoutesEngine {
        routing_rules,
    };

    let res = state.http_client.post("http://127.0.0.1:8080/admin/hot-reload/routes")
        .json(&payload)
        .send()
        .await;

    match res {
        Ok(resp) if resp.status().is_success() => StatusCode::OK,
        _ => StatusCode::BAD_GATEWAY,
    }
}
