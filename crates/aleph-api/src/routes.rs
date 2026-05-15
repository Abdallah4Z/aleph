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
    models::{QueryRequest, QueryResponse, OverviewStats, HourlyStat, DailyStat, AppStat, WindowStat, RecentEvent, AskRequest, AskResponse, CaptureStatus, DailySummaryResponse, SourceMetadata},
    Config, Database, TextEncoder,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, error};

struct AppState {
    db: Database,
    text_encoder: Arc<dyn TextEncoder + Send + Sync>,
}

/// Start the Axum HTTP server on `127.0.0.1:{port}`.
pub async fn run_api(port: u16, data_dir: PathBuf) -> anyhow::Result<()> {
    eprintln!("aleph: api: creating data dir...");
    std::fs::create_dir_all(&data_dir)?;

    eprintln!("aleph: api: opening database...");
    let db = Database::open(&data_dir).await?;

    eprintln!("aleph: api: loading text encoder...");
    let text_encoder: Arc<dyn TextEncoder + Send + Sync> = {
        let cache = data_dir.join("models").join("all-MiniLM-L6-v2");
        if cache.exists() {
            match aleph_core::embedding::MiniLmEncoder::from_dir(&cache) {
                Ok(enc) => Arc::new(enc),
                Err(e) => {
                    error!("Failed to load MiniLM: {}", e);
                    Arc::new(HashEncoder)
                }
            }
        } else {
            Arc::new(HashEncoder)
        }
    };

    let state = Arc::new(AppState { db, text_encoder });

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
        .route("/api/ask", post(ask_handler))
        .route("/api/capture/status", get(capture_status_handler).put(put_capture_handler))
        .route("/api/screenshots/{id}", get(screenshot_handler))
        .route("/api/daily-summary/{date}", get(daily_summary_handler))
        .route("/api/daily-summary/today", get(today_summary_handler))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    eprintln!("aleph: api: binding to {}...", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("aleph: api: listening on http://{}", addr);
    info!("Context API listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Query handler
// ---------------------------------------------------------------------------

async fn query_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QueryRequest>,
) -> Result<Json<QueryResponse>, StatusCode> {
    let question = req.question.clone();
    let top_k = req.top_k;

    let text_vec = state.text_encoder.encode(&question).map_err(|e| {
        error!("Encoding failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let image_vec = text_vec.clone();

    let query_req = QueryRequest { question, top_k };
    let response = state.db.query(query_req, &text_vec, &image_vec).await.map_err(|e| {
        error!("DB query failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(response))
}

// ---------------------------------------------------------------------------
// Ask handler — keyword search + LLM answer with sources
// ---------------------------------------------------------------------------

async fn ask_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AskRequest>,
) -> Result<Json<AskResponse>, StatusCode> {
    let results = state.db.keyword_search(&req.question, req.top_k as i64).await.map_err(|e| {
        error!("Keyword search failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if results.is_empty() {
        return Ok(Json(AskResponse {
            answer: "No matching context found in your desktop history.".into(),
            sources: vec![],
        }));
    }

    // Format context for LLM
    let mut context_lines = Vec::new();
    for ev in &results {
        let start = chrono::DateTime::from_timestamp_millis(ev.start_time)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "unknown".into());
        let dur = if ev.duration_ms > 0 {
            format!(" ({}s)", ev.duration_ms / 1000)
        } else {
            String::new()
        };
        context_lines.push(format!(
            "- [{}] App: {}, Window: \"{}\"{}{}",
            start, ev.app_name, ev.window_title, dur,
            if ev.source_type == "vision" { "" } else { "" }
        ));
    }

    let context_text = context_lines.join("\n");
    let system_prompt = "You are Aleph, a desktop context assistant. \
        Below is the user's recent desktop activity history. \
        Answer the user's question based ONLY on this context. \
        Be specific — mention app names, window titles, and timestamps. \
        If the context doesn't contain enough information, say so.";

    let user_prompt = format!(
        "My desktop activity (most relevant first):\n{}\n\nQuestion: {}",
        context_text, req.question
    );

    let config = Config::global();
    let answer = match aleph_core::llm::ask_llm(config, system_prompt, &user_prompt) {
        Ok(a) => a,
        Err(e) => {
            // Fallback to just returning the raw context
            let lines: Vec<String> = results.iter().map(|ev| {
                let start = chrono::DateTime::from_timestamp_millis(ev.start_time)
                    .map(|dt| dt.format("%H:%M").to_string()).unwrap_or_default();
                format!("{} | {} — {} ({}s)", start, ev.app_name, ev.window_title, ev.duration_ms / 1000)
            }).collect();
            format!("LLM unavailable ({}). Raw context:\n{}", e, lines.join("\n"))
        }
    };

    let sources: Vec<SourceMetadata> = results.into_iter().map(|ev| SourceMetadata {
        id: ev.id,
        app_name: ev.app_name,
        window_title: ev.window_title,
        start_time: ev.start_time,
        end_time: ev.end_time,
        source_type: ev.source_type,
    }).collect();

    Ok(Json(AskResponse { answer, sources }))
}

// ---------------------------------------------------------------------------
// Screenshot handler
// ---------------------------------------------------------------------------

async fn screenshot_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<(axum::http::StatusCode, [(axum::http::HeaderName, String); 1], Vec<u8>), StatusCode> {
    use axum::http::header;
    match state.db.get_screenshot(id).await {
        Ok(Some(png)) => Ok((StatusCode::OK, [(header::CONTENT_TYPE, "image/png".into())], png)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(e) => {
            error!("Screenshot fetch error: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// ---------------------------------------------------------------------------
// Capture status & pause/resume
// ---------------------------------------------------------------------------

async fn capture_status_handler() -> Json<CaptureStatus> {
    Json(CaptureStatus { enabled: Config::global().capture.enabled })
}

async fn put_capture_handler(Json(body): Json<CaptureStatus>) -> Result<Json<CaptureStatus>, StatusCode> {
    let mut cfg = Config::global().clone();
    cfg.capture.enabled = body.enabled;
    cfg.save().map_err(|e| {
        error!("Failed to save capture config: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let _ = aleph_core::Config::init_global();
    Ok(Json(CaptureStatus { enabled: cfg.capture.enabled }))
}

// ---------------------------------------------------------------------------
// Daily summary
// ---------------------------------------------------------------------------

async fn today_summary_handler(State(state): State<Arc<AppState>>) -> Result<Json<DailySummaryResponse>, StatusCode> {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    daily_summary_for_date(&state.db, &today).await
}

async fn daily_summary_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(date): axum::extract::Path<String>,
) -> Result<Json<DailySummaryResponse>, StatusCode> {
    daily_summary_for_date(&state.db, &date).await
}

async fn daily_summary_for_date(db: &Database, date: &str) -> Result<Json<DailySummaryResponse>, StatusCode> {
    // Check if we already have a cached summary
    if let Ok(Some(existing)) = db.get_daily_summary(date).await {
        return Ok(Json(DailySummaryResponse { date: date.to_string(), summary: existing }));
    }

    // Fetch events for that date
    let yesterday_start = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis())
        .unwrap_or(0);
    let yesterday_end = yesterday_start + 86_400_000;

    // Get events in range
    let events = db.get_recent_events(500).await.map_err(|e| {
        error!("Failed to fetch events for summary: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let day_events: Vec<_> = events.into_iter().filter(|e| {
        e.start_time >= yesterday_start && e.start_time < yesterday_end
    }).collect();

    if day_events.is_empty() {
        return Ok(Json(DailySummaryResponse {
            date: date.to_string(),
            summary: "No activity recorded on this day.".into(),
        }));
    }

    // Format for LLM
    let mut lines = Vec::new();
    for ev in &day_events {
        let t = chrono::DateTime::from_timestamp_millis(ev.start_time)
            .map(|dt| dt.format("%H:%M").to_string()).unwrap_or_default();
        lines.push(format!("- {} | {} — {} ({}s)", t, ev.app_name, ev.window_title, ev.duration_ms / 1000));
    }
    let context = lines.join("\n");

    let prompt = format!(
        "Summarize this user's day based on their desktop activity log. \
         Include: total sessions, most used apps, peak hours, notable patterns. \
         Be concise but specific.\n\n{}",
        context
    );

    let summary = match aleph_core::llm::ask_llm(Config::global(),
        "You are a daily activity summarizer. Return only the summary, no preamble.", &prompt)
    {
        Ok(s) => s,
        Err(e) => format!("LLM unavailable ({}). Raw activity:\n{}", e, context),
    };

    // Cache it
    if let Err(e) = db.insert_daily_summary(date, &summary).await {
        error!("Failed to cache daily summary: {}", e);
    }

    Ok(Json(DailySummaryResponse { date: date.to_string(), summary }))
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
    if let Some(llm) = updates.get("llm") {
        if let Some(active) = llm.get("active_provider").and_then(|v| v.as_str()) {
            cfg.llm.active_provider = active.to_string();
        }
        if let Some(providers) = llm.get("providers").and_then(|v| v.as_object()) {
            for (name, p) in providers {
                let pc = match name.as_str() {
                    "ollama" => &mut cfg.llm.providers.ollama,
                    "ollama_cloud" => &mut cfg.llm.providers.ollama_cloud,
                    "openai" => &mut cfg.llm.providers.openai,
                    "openrouter" => &mut cfg.llm.providers.openrouter,
                    "groq" => &mut cfg.llm.providers.groq,
                    _ => continue,
                };
                if let Some(enabled) = p.get("enabled").and_then(|v| v.as_bool()) {
                    pc.enabled = enabled;
                }
                if let Some(model) = p.get("model").and_then(|v| v.as_str()) {
                    if !model.is_empty() {
                        pc.model = model.to_string();
                    }
                }
                if let Some(key) = p.get("api_key").and_then(|v| v.as_str()) {
                    pc.api_key = key.to_string();
                }
                if let Some(url) = p.get("base_url").and_then(|v| v.as_str()) {
                    if !url.is_empty() {
                        pc.base_url = url.to_string();
                    }
                }
            }
        }
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
    State(state): State<Arc<AppState>>,
) -> Result<Json<OverviewStats>, StatusCode> {
    state.db.get_overview().await.map_err(|e| {
        error!("Overview failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    }).map(Json)
}

async fn stats_hourly(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<HourlyStat>>, StatusCode> {
    state.db.get_hourly_stats().await.map_err(|e| {
        error!("Hourly failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    }).map(Json)
}

async fn stats_daily(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<DailyStat>>, StatusCode> {
    state.db.get_daily_stats().await.map_err(|e| {
        error!("Daily failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    }).map(Json)
}

async fn stats_apps(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<AppStat>>, StatusCode> {
    state.db.get_app_stats().await.map_err(|e| {
        error!("Apps failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    }).map(Json)
}

async fn stats_windows(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<WindowStat>>, StatusCode> {
    state.db.get_window_stats(20).await.map_err(|e| {
        error!("Windows failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    }).map(Json)
}

async fn stats_recent(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<RecentEvent>>, StatusCode> {
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
