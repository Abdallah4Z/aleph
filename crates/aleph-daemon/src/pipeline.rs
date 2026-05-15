use aleph_core::models::WindowContent;
use aleph_core::{Database, TextEncoder, VisionEncoder};
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tracing::{info, warn};
use xxhash_rust::xxh3::xxh3_64;

pub struct Pipeline {
    db: Database,
    text_encoder: Box<dyn TextEncoder>,
    vision_encoder: Box<dyn VisionEncoder>,
    last_text_hash: Option<u64>,
    dedup_threshold: f32,
    last_n: usize,
}

impl Pipeline {
    pub async fn new(
        data_dir: PathBuf,
        text_encoder: Box<dyn TextEncoder>,
        vision_encoder: Box<dyn VisionEncoder>,
    ) -> Result<Self> {
        let db = Database::open(&data_dir).await?;
        Ok(Self {
            db,
            text_encoder,
            vision_encoder,
            last_text_hash: None,
            dedup_threshold: 0.95,
            last_n: 5,
        })
    }

    pub async fn run(&mut self, mut rx: mpsc::Receiver<aleph_core::models::WindowEvent>) -> Result<()> {
        info!("Pipeline running. Listening for window focus events...");

        while let Some(event) = rx.recv().await {
            match event.content {
                WindowContent::Text(text) => {
                    if let Err(e) = self.handle_text(&event.app_name, &event.window_title, &text).await {
                        tracing::error!("Text handling error: {}", e);
                    }
                }
                WindowContent::ImageRequired => {
                    if let Err(e) = self.handle_vision_fallback(&event.app_name, &event.window_title).await {
                        tracing::error!("Vision handling error: {}", e);
                    }
                }
                WindowContent::Screenshot(png) => {
                    if let Err(e) = self.handle_screenshot(&event.app_name, &event.window_title, &png).await {
                        tracing::error!("Screenshot handling error: {}", e);
                    }
                }
            }
        }

        Ok(())
    }

    fn classify(app: &str, title: &str) -> &'static str {
        let al = app.to_lowercase();
        let tl = title.to_lowercase();
        if al.contains("zen") || al.contains("firefox") || al.contains("chrome") || al.contains("brave") || al.contains("edge") || al.contains("browser") || tl.contains("google search") || tl.contains("zen browser") || tl.contains("firefox") {
            "browsing"
        } else if al.contains("discord") || al.contains("telegram") || al.contains("whatsapp") || al.contains("slack") || al.contains("signal") {
            "communication"
        } else if al.contains("code") || al.contains("zed") || al.contains("vim") || al.contains("neovim") || al.contains("intellij") || al.contains("pycharm") || al.contains("rustrover") || al.contains("goland") || al.contains("cursor") || al.contains("vscodium") {
            "coding"
        } else if al.contains("terminal") || al.contains("gnome-terminal") || al.contains("alacritty") || al.contains("kitty") || al.contains("wezterm") || al.contains("konsole") || al.contains("tilix") {
            "terminal"
        } else if al.contains("spotify") || al.contains("vlc") || al.contains("mpv") || al.contains("youtube") || al.contains("netflix") {
            "entertainment"
        } else {
            "other"
        }
    }

    async fn handle_text(&mut self, app: &str, title: &str, text: &str) -> Result<()> {
        let hash = xxh3_64(text.as_bytes());

        if self.last_text_hash == Some(hash) {
            return Ok(());
        }
        self.last_text_hash = Some(hash);

        let embedding = self.text_encoder.encode(text)?;

        if let Some(existing_id) = self
            .db
            .find_similar_and_dedup("text_vectors", &embedding, self.dedup_threshold, self.last_n)
            .await?
        {
            self.db.update_end_time(existing_id).await?;
            info!("Text dedup: updated end_time for id={}", existing_id);
            return Ok(());
        }

        let category = Self::classify(app, title);
        let code = aleph_core::codecontext::parse_code_context(app, title);
        let meta_id = self
            .db
            .insert_event(app, title, "text", Some(&format!("{:x}", hash)), Some(category),
                code.as_ref().and_then(|c| c.file.as_deref()),
                code.as_ref().and_then(|c| c.project.as_deref()),
                code.as_ref().and_then(|c| c.branch.as_deref()))
            .await?;

        self.db
            .insert_vector("text_vectors", meta_id, &embedding)
            .await?;

        info!("Text inserted: id={}, app={}, title={} [{}]", meta_id, app, title, category);
        Ok(())
    }

    async fn handle_screenshot(&mut self, app: &str, title: &str, png_bytes: &[u8]) -> Result<()> {
        let img = image::load_from_memory(png_bytes)?;

        let embedding = self.vision_encoder.encode_image(&img)?;

        let hash = xxh3_64(png_bytes);

        if let Some(existing_id) = self
            .db
            .find_similar_and_dedup("image_vectors", &embedding, self.dedup_threshold, self.last_n)
            .await?
        {
            self.db.update_end_time(existing_id).await?;
            info!("Vision dedup: updated end_time for id={}", existing_id);
            return Ok(());
        }

        let category = Self::classify(app, title);
        let code = aleph_core::codecontext::parse_code_context(app, title);
        let meta_id = self
            .db
            .insert_event(app, title, "vision", Some(&format!("{:x}", hash)), Some(category),
                code.as_ref().and_then(|c| c.file.as_deref()),
                code.as_ref().and_then(|c| c.project.as_deref()),
                code.as_ref().and_then(|c| c.branch.as_deref()))
            .await?;

        // Store screenshot PNG
        self.db.insert_screenshot(meta_id, png_bytes).await?;

        self.db
            .insert_vector("image_vectors", meta_id, &embedding)
            .await?;

        info!("Vision inserted: id={}, app={}, title={} [{}]", meta_id, app, title, category);
        Ok(())
    }

    async fn handle_vision_fallback(&mut self, app: &str, title: &str) -> Result<()> {
        warn!("Vision fallback (no screenshot) for app={}, title={}.", app, title);
        Ok(())
    }
}
