//! SQLite-backed vector store.
//!
//! [`Database`] manages two kinds of tables:
//!
//! - **`context_events`** — Event metadata (app name, window title, timestamps, source type).
//! - **`text_vectors`** / **`image_vectors`** — Embedding vectors stored as raw BLOBs (4 bytes × dim).
//!
//! KNN search is brute-force over all rows, which is fast enough for < 10k vectors.

use anyhow::Result;
use chrono::Utc;
use sqlx::{sqlite::{SqliteConnectOptions, SqlitePoolOptions}, Pool, Row, Sqlite};
use std::path::Path;
use std::str::FromStr;

use crate::dedup;
use crate::models::{
    AppStat, ContextChunk, DailyStat, HourlyStat, OverviewStats, QueryRequest, QueryResponse,
    RecentEvent, SourceMetadata, WindowStat,
};

/// Wraps a SQLite connection pool and provides high-level CRUD for events and vectors.
pub struct Database {
    sqlite: Pool<Sqlite>,
}

impl Database {
    /// Open (or create) the SQLite database at `{data_dir}/metadata.db`.
    ///
    /// Tables are created automatically if they don't exist.
    pub async fn open(data_dir: &Path) -> Result<Self> {
        let abs_dir = if data_dir.is_absolute() {
            data_dir.to_path_buf()
        } else {
            std::env::current_dir()?.join(data_dir)
        };
        std::fs::create_dir_all(&abs_dir)?;
        let sqlite_path = abs_dir.join("metadata.db");
        let sqlite_url = format!("sqlite:{}", sqlite_path.display());

        let opts = SqliteConnectOptions::from_str(&sqlite_url)?
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await?;

        Self::init_sqlite(&pool).await?;

        Ok(Self { sqlite: pool })
    }

    async fn init_sqlite(pool: &Pool<Sqlite>) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS context_events (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                app_name    TEXT NOT NULL,
                window_title TEXT NOT NULL,
                start_time  INTEGER NOT NULL,
                end_time    INTEGER,
                content_hash TEXT,
                source_type TEXT CHECK(source_type IN ('text', 'vision')) NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_start_time ON context_events(start_time);
            CREATE INDEX IF NOT EXISTS idx_content_hash ON context_events(content_hash);

            CREATE TABLE IF NOT EXISTS text_vectors (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                metadata_id INTEGER NOT NULL,
                vector      BLOB NOT NULL,
                timestamp   INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_text_timestamp ON text_vectors(timestamp);

            CREATE TABLE IF NOT EXISTS image_vectors (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                metadata_id INTEGER NOT NULL,
                vector      BLOB NOT NULL,
                timestamp   INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_image_timestamp ON image_vectors(timestamp);
            "#,
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Insert a new event into `context_events` and return its row ID.
    pub async fn insert_event(
        &self,
        app_name: &str,
        window_title: &str,
        source_type: &str,
        content_hash: Option<&str>,
    ) -> Result<i64> {
        let now = Utc::now().timestamp_millis();
        let result = sqlx::query(
            r#"
            INSERT INTO context_events (app_name, window_title, start_time, end_time, content_hash, source_type)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(app_name)
        .bind(window_title)
        .bind(now)
        .bind(now)
        .bind(content_hash)
        .bind(source_type)
        .execute(&self.sqlite)
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Update `end_time` to `now` for the event with the given ID.
    pub async fn update_end_time(&self, id: i64) -> Result<()> {
        let now = Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE context_events SET end_time = ?1 WHERE id = ?2",
        )
        .bind(now)
        .bind(id)
        .execute(&self.sqlite)
        .await?;
        Ok(())
    }

    /// Fetch the last `n` vectors from a vector table, ordered by most recent first.
    pub async fn get_last_n_vectors(&self, table: &str, n: usize) -> Result<Vec<(i64, Vec<f32>)>> {
        let rows = sqlx::query(&format!(
            "SELECT metadata_id, vector FROM {} ORDER BY timestamp DESC LIMIT ?1",
            table
        ))
        .bind(n as i64)
        .fetch_all(&self.sqlite)
        .await?;

        let mut out = Vec::new();
        for row in rows {
            let id: i64 = row.get("metadata_id");
            let blob: Vec<u8> = row.get("vector");
            let vec = bytes_to_f32_vec(&blob);
            out.push((id, vec));
        }
        Ok(out)
    }

    /// Insert a vector BLOB into a vector table.
    pub async fn insert_vector(
        &self,
        table: &str,
        metadata_id: i64,
        vector: &[f32],
    ) -> Result<()> {
        let blob = f32_vec_to_bytes(vector);
        let timestamp = Utc::now().timestamp_millis();

        sqlx::query(&format!(
            "INSERT INTO {} (metadata_id, vector, timestamp) VALUES (?1, ?2, ?3)",
            table
        ))
        .bind(metadata_id)
        .bind(blob)
        .bind(timestamp)
        .execute(&self.sqlite)
        .await?;

        Ok(())
    }

    /// Check if `new_vec` is similar (≥ `threshold`) to any of the last `last_n` stored vectors.
    ///
    /// Returns `Some(metadata_id)` of the most recent matching vector, or `None` if all are distant.
    pub async fn find_similar_and_dedup(
        &self,
        table: &str,
        new_vec: &[f32],
        threshold: f32,
        last_n: usize,
    ) -> Result<Option<i64>> {
        let recent = self.get_last_n_vectors(table, last_n).await?;
        if recent.is_empty() {
            return Ok(None);
        }

        let recent_vecs: Vec<Vec<f32>> = recent.iter().map(|(_, v)| v.clone()).collect();
        if dedup::should_dedup(new_vec, &recent_vecs, threshold) {
            return Ok(Some(recent[0].0));
        }
        Ok(None)
    }

    /// Brute-force KNN search over all vectors in a table.
    ///
    /// Returns up to `k` pairs of `(metadata_id, cosine_similarity)`.
    pub async fn knn_search(
        &self,
        table: &str,
        vector: &[f32],
        k: usize,
    ) -> Result<Vec<(i64, f32)>> {
        let rows = sqlx::query(&format!(
            "SELECT metadata_id, vector FROM {}",
            table
        ))
        .fetch_all(&self.sqlite)
        .await?;

        let mut scored: Vec<(i64, f32)> = Vec::new();
        for row in rows {
            let id: i64 = row.get("metadata_id");
            let blob: Vec<u8> = row.get("vector");
            let candidate = bytes_to_f32_vec(&blob);
            let sim = dedup::cosine_similarity(vector, &candidate);
            scored.push((id, sim));
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        scored.truncate(k);
        Ok(scored)
    }

    /// Answer a user query by searching both text and image vector stores,
    /// merging results, and returning enriched context chunks.
    pub async fn query(&self, req: QueryRequest, text_vec: &[f32], image_vec: &[f32]) -> Result<QueryResponse> {
        let text_results = self.knn_search("text_vectors", text_vec, req.top_k).await?;
        let image_results = self.knn_search("image_vectors", image_vec, req.top_k).await?;

        let mut merged: Vec<(i64, f32, &str)> = Vec::new();
        for (id, sim) in &text_results {
            merged.push((*id, *sim, "text"));
        }
        for (id, sim) in &image_results {
            merged.push((*id, *sim, "image"));
        }

        merged.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        merged.truncate(req.top_k);

        let mut context = Vec::new();
        let mut sources = Vec::new();

        for (id, score, _source) in merged {
            let row = sqlx::query(
                "SELECT * FROM context_events WHERE id = ?1",
            )
            .bind(id)
            .fetch_optional(&self.sqlite)
            .await?;

            if let Some(row) = row {
                let meta = SourceMetadata {
                    id: row.get("id"),
                    app_name: row.get("app_name"),
                    window_title: row.get("window_title"),
                    start_time: row.get("start_time"),
                    end_time: row.get("end_time"),
                    source_type: row.get::<String, _>("source_type"),
                };
                sources.push(meta.clone());
                context.push(ContextChunk {
                    text: None,
                    metadata: meta,
                    score,
                });
            }
        }

        Ok(QueryResponse { context, sources })
    }

    // -----------------------------------------------------------------------
    // Dashboard / Analytics queries
    // -----------------------------------------------------------------------

    pub async fn get_overview(&self) -> Result<OverviewStats> {
        let total_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM context_events")
            .fetch_one(&self.sqlite).await?;
        let total_apps: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT app_name) FROM context_events",
        ).fetch_one(&self.sqlite).await?;
        let total_ms: Option<i64> = sqlx::query_scalar(
            "SELECT COALESCE(SUM(COALESCE(end_time, start_time) - start_time), 0) FROM context_events",
        ).fetch_one(&self.sqlite).await?;
        let today_events: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM context_events WHERE start_time >= unixepoch('now', 'start of day') * 1000",
        ).fetch_one(&self.sqlite).await?;
        let most_used: Option<String> = sqlx::query_scalar(
            "SELECT app_name FROM context_events GROUP BY app_name ORDER BY COUNT(*) DESC LIMIT 1",
        ).fetch_optional(&self.sqlite).await?;
        Ok(OverviewStats {
            total_events,
            total_apps,
            total_tracked_hours: (total_ms.unwrap_or(0) as f64) / 3_600_000.0,
            today_events,
            most_used_app: most_used.unwrap_or_default(),
        })
    }

    pub async fn get_hourly_stats(&self) -> Result<Vec<HourlyStat>> {
        #[derive(sqlx::FromRow)]
        struct Row { hour: i64, count: i64 }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT CAST(strftime('%H', start_time / 1000, 'unixepoch') AS INTEGER) AS hour, COUNT(*) AS count
             FROM context_events GROUP BY hour ORDER BY hour",
        ).fetch_all(&self.sqlite).await?;
        Ok(rows.into_iter().map(|r| HourlyStat { hour: r.hour, count: r.count }).collect())
    }

    pub async fn get_daily_stats(&self) -> Result<Vec<DailyStat>> {
        #[derive(sqlx::FromRow)]
        struct Row { day: String, count: i64, duration: i64 }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT date(start_time / 1000, 'unixepoch') AS day,
                    COUNT(*) AS count,
                    COALESCE(SUM(COALESCE(end_time, start_time) - start_time), 0) AS duration
             FROM context_events GROUP BY day ORDER BY day DESC LIMIT 30",
        ).fetch_all(&self.sqlite).await?;
        Ok(rows.into_iter().map(|r| DailyStat { date: r.day, count: r.count, duration_ms: r.duration }).collect())
    }

    pub async fn get_app_stats(&self) -> Result<Vec<AppStat>> {
        #[derive(sqlx::FromRow)]
        struct Row { app_name: String, count: i64, duration: i64 }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT app_name, COUNT(*) AS count,
                    COALESCE(SUM(COALESCE(end_time, start_time) - start_time), 0) AS duration
             FROM context_events GROUP BY app_name ORDER BY duration DESC",
        ).fetch_all(&self.sqlite).await?;
        Ok(rows.into_iter().map(|r| AppStat { app_name: r.app_name, count: r.count, duration_ms: r.duration }).collect())
    }

    pub async fn get_window_stats(&self, limit: i64) -> Result<Vec<WindowStat>> {
        #[derive(sqlx::FromRow)]
        struct Row { app_name: String, window_title: String, count: i64, duration: i64 }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT app_name, window_title, COUNT(*) AS count,
                    COALESCE(SUM(COALESCE(end_time, start_time) - start_time), 0) AS duration
             FROM context_events GROUP BY app_name, window_title ORDER BY duration DESC LIMIT ?1",
        ).bind(limit).fetch_all(&self.sqlite).await?;
        Ok(rows.into_iter().map(|r| WindowStat {
            app_name: r.app_name, window_title: r.window_title, count: r.count, duration_ms: r.duration,
        }).collect())
    }

    pub async fn get_recent_events(&self, limit: i64) -> Result<Vec<RecentEvent>> {
        #[derive(sqlx::FromRow)]
        struct Row { id: i64, app_name: String, window_title: String, start_time: i64, end_time: Option<i64>, source_type: String }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, app_name, window_title, start_time, end_time, source_type
             FROM context_events ORDER BY start_time DESC LIMIT ?1",
        ).bind(limit).fetch_all(&self.sqlite).await?;
        Ok(rows.into_iter().map(|r| {
            let duration_ms = r.end_time.unwrap_or(r.start_time) - r.start_time;
            RecentEvent { id: r.id, app_name: r.app_name, window_title: r.window_title, start_time: r.start_time, end_time: r.end_time, duration_ms, source_type: r.source_type }
        }).collect())
    }

    /// Keyword search over window titles and app names (case-insensitive).
    /// Returns up to `limit` matching events ordered by recency.
    pub async fn keyword_search(&self, query: &str, limit: i64) -> Result<Vec<RecentEvent>> {
        #[derive(sqlx::FromRow)]
        struct Row { id: i64, app_name: String, window_title: String, start_time: i64, end_time: Option<i64>, source_type: String }
        let pattern = format!("%{}%", query);
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, app_name, window_title, start_time, end_time, source_type
             FROM context_events
             WHERE window_title LIKE ?1 OR app_name LIKE ?1
             ORDER BY start_time DESC LIMIT ?2",
        )
        .bind(&pattern)
        .bind(limit)
        .fetch_all(&self.sqlite)
        .await?;
        Ok(rows.into_iter().map(|r| {
            let duration_ms = r.end_time.unwrap_or(r.start_time) - r.start_time;
            RecentEvent { id: r.id, app_name: r.app_name, window_title: r.window_title, start_time: r.start_time, end_time: r.end_time, duration_ms, source_type: r.source_type }
        }).collect())
    }
}

// ---------------------------------------------------------------------------
// Helpers: f32 <-> bytes for SQLite BLOB storage
// ---------------------------------------------------------------------------

fn f32_vec_to_bytes(vec: &[f32]) -> Vec<u8> {
    vec.iter()
        .flat_map(|&f| f.to_le_bytes())
        .collect()
}

fn bytes_to_f32_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}
