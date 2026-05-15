//! API routes: query, stats, and dashboard frontend.

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, Json},
    routing::{get, post},
    Router,
};
use std::borrow::Cow;
use aleph_core::{
    models::{QueryRequest, QueryResponse, OverviewStats, HourlyStat, DailyStat, AppStat, WindowStat, RecentEvent},
    Config, Database, TextEncoder,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, error};

struct AppState {
    db: Database,
    text_encoder: Box<dyn TextEncoder + Send + Sync>,
}

/// Start the Axum HTTP server on `127.0.0.1:{port}`.
pub async fn run_api(port: u16, data_dir: PathBuf) -> anyhow::Result<()> {
    std::fs::create_dir_all(&data_dir)?;

    let db = Database::open(&data_dir).await?;

    let text_encoder: Box<dyn TextEncoder + Send + Sync> = {
        let cache = data_dir.join("models").join("all-MiniLM-L6-v2");
        if cache.exists() {
            match aleph_core::embedding::MiniLmEncoder::from_dir(&cache) {
                Ok(enc) => Box::new(enc),
                Err(e) => {
                    error!("Failed to load MiniLM: {}", e);
                    Box::new(HashEncoder)
                }
            }
        } else {
            Box::new(HashEncoder)
        }
    };

    let state = Arc::new(Mutex::new(AppState { db, text_encoder }));

    let app = Router::new()
        .route("/", get(dashboard_handler))
        .route("/settings", get(settings_handler))
        .route("/query", post(query_handler))
        .route("/health", post(health_handler))
        .route("/api/stats/overview", get(stats_overview))
        .route("/api/stats/hourly", get(stats_hourly))
        .route("/api/stats/daily", get(stats_daily))
        .route("/api/stats/apps", get(stats_apps))
        .route("/api/stats/windows", get(stats_windows))
        .route("/api/stats/recent", get(stats_recent))
        .route("/api/settings", get(get_settings).put(put_settings))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    info!("Context API listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Query handler
// ---------------------------------------------------------------------------

async fn query_handler(
    State(state): State<Arc<Mutex<AppState>>>,
    Json(req): Json<QueryRequest>,
) -> Result<Json<QueryResponse>, StatusCode> {
    let state = state.lock().await;
    let text_vec = state.text_encoder.encode(&req.question).map_err(|e| {
        error!("Encoding failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let image_vec = text_vec.clone();
    let response = state.db.query(req, &text_vec, &image_vec).await.map_err(|e| {
        error!("DB query failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(response))
}

const DASHBOARD_HTML: &str = include_str!("../../../dashboard/index.html");
const SETTINGS_HTML: &str = include_str!("../../../dashboard/settings.html");

async fn settings_handler() -> Result<Html<Cow<'static, str>>, StatusCode> {
    Ok(Html(Cow::Borrowed(SETTINGS_HTML)))
}

async fn dashboard_handler() -> Result<Html<Cow<'static, str>>, StatusCode> {
    Ok(Html(Cow::Borrowed(DASHBOARD_HTML)))
}

async fn health_handler() -> &'static str {
    "ok"
}

async fn get_settings() -> Json<aleph_core::Config> {
    Json(Config::global().clone())
}

async fn put_settings(Json(updates): Json<serde_json::Value>) -> Result<Json<aleph_core::Config>, StatusCode> {
    let mut cfg = Config::global().clone();
    if let Some(port) = updates.get("general").and_then(|g| g.get("port")).and_then(|v| v.as_u64()) {
        cfg.general.port = port as u16;
    }
    if let Some(interval) = updates.get("polling").and_then(|p| p.get("interval_secs")).and_then(|v| v.as_u64()) {
        cfg.polling.interval_secs = interval;
    }
    if let Some(threshold) = updates.get("dedup").and_then(|d| d.get("threshold")).and_then(|v| v.as_f64()) {
        cfg.dedup.threshold = threshold as f32;
    }
    if let Some(last_n) = updates.get("dedup").and_then(|d| d.get("last_n")).and_then(|v| v.as_u64()) {
        cfg.dedup.last_n = last_n as usize;
    }
    if let Some(max) = updates.get("retention").and_then(|r| r.get("max_events")).and_then(|v| v.as_i64()) {
        cfg.retention.max_events = max;
    }
    if let Some(text) = updates.get("encoders").and_then(|e| e.get("text")).and_then(|v| v.as_bool()) {
        cfg.encoders.text = text;
    }
    if let Some(vision) = updates.get("encoders").and_then(|e| e.get("vision")).and_then(|v| v.as_bool()) {
        cfg.encoders.vision = vision;
    }
    if let Some(theme) = updates.get("dashboard").and_then(|d| d.get("theme")).and_then(|v| v.as_str()) {
        cfg.dashboard.theme = theme.to_string();
    }
    if let Some(level) = updates.get("general").and_then(|g| g.get("log_level")).and_then(|v| v.as_str()) {
        cfg.general.log_level = level.to_string();
    }

    cfg.save().map_err(|e| {
        error!("Failed to save config: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Reload global config
    let _ = aleph_core::Config::init_global();

    Ok(Json(cfg))
}

// ---------------------------------------------------------------------------
// Stats handlers
// ---------------------------------------------------------------------------

async fn stats_overview(
    State(state): State<Arc<Mutex<AppState>>>,
) -> Result<Json<OverviewStats>, StatusCode> {
    let state = state.lock().await;
    state.db.get_overview().await.map_err(|e| {
        error!("Overview failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    }).map(Json)
}

async fn stats_hourly(
    State(state): State<Arc<Mutex<AppState>>>,
) -> Result<Json<Vec<HourlyStat>>, StatusCode> {
    let state = state.lock().await;
    state.db.get_hourly_stats().await.map_err(|e| {
        error!("Hourly failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    }).map(Json)
}

async fn stats_daily(
    State(state): State<Arc<Mutex<AppState>>>,
) -> Result<Json<Vec<DailyStat>>, StatusCode> {
    let state = state.lock().await;
    state.db.get_daily_stats().await.map_err(|e| {
        error!("Daily failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    }).map(Json)
}

async fn stats_apps(
    State(state): State<Arc<Mutex<AppState>>>,
) -> Result<Json<Vec<AppStat>>, StatusCode> {
    let state = state.lock().await;
    state.db.get_app_stats().await.map_err(|e| {
        error!("Apps failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    }).map(Json)
}

async fn stats_windows(
    State(state): State<Arc<Mutex<AppState>>>,
) -> Result<Json<Vec<WindowStat>>, StatusCode> {
    let state = state.lock().await;
    state.db.get_window_stats(20).await.map_err(|e| {
        error!("Windows failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    }).map(Json)
}

async fn stats_recent(
    State(state): State<Arc<Mutex<AppState>>>,
) -> Result<Json<Vec<RecentEvent>>, StatusCode> {
    let state = state.lock().await;
    state.db.get_recent_events(50).await.map_err(|e| {
        error!("Recent failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    }).map(Json)
}

// ---------------------------------------------------------------------------
// Fallback text encoder (used when MiniLM weights are missing)
// ---------------------------------------------------------------------------

struct HashEncoder;

impl aleph_core::TextEncoder for HashEncoder {
    fn encode(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut vec = Vec::with_capacity(aleph_core::embedding::TEXT_DIM);
        for i in 0..aleph_core::embedding::TEXT_DIM {
            let hash = xxhash_rust::xxh3::xxh3_64(&[text.as_bytes(), &i.to_le_bytes()].concat());
            let val = (hash as f32 / u64::MAX as f32) * 2.0 - 1.0;
            vec.push(val);
        }
        let norm = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        Ok(vec.iter().map(|x| x / (norm + 1e-12)).collect())
    }
}
