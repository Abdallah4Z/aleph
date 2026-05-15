use aleph_core::models::{WindowContent, WindowEvent};
use aleph_core::ContextExtractor;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::sync::mpsc;
use xcap::Window;

pub struct AtSpiExtractor;

#[async_trait]
impl ContextExtractor for AtSpiExtractor {
    async fn subscribe_focus(&self) -> Result<mpsc::Receiver<WindowEvent>> {
        let (tx, rx) = mpsc::channel(64);

        tokio::spawn(async move {
            let last_id = AtomicI64::new(0);

            loop {
                match poll_focused_window(&tx, &last_id).await {
                    Ok(_) => break,
                    Err(e) => {
                        tracing::debug!("Focus polling unavailable ({}). Retrying in 5s.", e);
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            }
        });

        Ok(rx)
    }
}

async fn poll_focused_window(tx: &mpsc::Sender<WindowEvent>, last_id: &AtomicI64) -> Result<()> {
    loop {
        let windows = Window::all()?;
        if let Some(focused) = windows.iter().find(|w| w.is_focused().unwrap_or(false)) {
            let current_id = focused.id()? as i64;
            let prev = last_id.swap(current_id, Ordering::Relaxed);

            if prev != current_id {
                let app_name = focused.app_name().unwrap_or_else(|_| "unknown".into());
                let window_title = focused.title().unwrap_or_else(|_| "unknown".into());

                match focused.capture_image() {
                    Ok(rgba) => {
                        let dyn_img = image::DynamicImage::from(rgba);
                        let mut png_bytes = Vec::new();
                        dyn_img.write_to(
                            &mut std::io::Cursor::new(&mut png_bytes),
                            image::ImageFormat::Png,
                        )?;

                        tracing::info!(
                            "Focus changed: app={}, title={} (screenshot captured)",
                            app_name,
                            window_title
                        );
                        let event = WindowEvent {
                            app_name,
                            window_title,
                            content: WindowContent::Screenshot(png_bytes),
                        };
                        if tx.send(event).await.is_err() {
                            return Ok(());
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Failed to capture screenshot: {}", e);
                        let event = WindowEvent {
                            app_name,
                            window_title,
                            content: WindowContent::ImageRequired,
                        };
                        if tx.send(event).await.is_err() {
                            return Ok(());
                        }
                    }
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}
