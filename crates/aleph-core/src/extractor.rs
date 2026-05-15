//! Window-focus extractors.
//!
//! The [`ContextExtractor`] trait defines a common interface for all focus-event sources.
//! Two implementations are provided:
//!
//! - [`MockExtractor`] — Reads events from a JSON file; used in headless / CI environments.
//! - `AtSpiExtractor` (in the `context-daemon` crate) — Listens to AT-SPI D-Bus signals on Linux.

use crate::models::WindowEvent;
use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;
use tokio::sync::mpsc;

/// Anything that can produce a stream of [`WindowEvent`]s from window-focus changes.
#[async_trait]
pub trait ContextExtractor: Send + Sync {
    /// Subscribe to focus events. Returns a channel receiver that yields events as they happen.
    async fn subscribe_focus(&self) -> Result<mpsc::Receiver<WindowEvent>>;
}

// ---------------------------------------------------------------------------
// MockExtractor — reads from a JSON file for headless / CI testing
// ---------------------------------------------------------------------------

/// Reads a sequence of [`WindowEvent`]s from a JSON array file and replays them
/// every 3 seconds.
pub struct MockExtractor {
    events: Vec<WindowEvent>,
}

impl MockExtractor {
    /// Load events from a JSON file. The file must contain a JSON array of [`WindowEvent`] objects.
    pub fn from_json<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = std::fs::read_to_string(path)?;
        let events: Vec<WindowEvent> = serde_json::from_str(&data)?;
        Ok(Self { events })
    }

    /// Create an extractor from an in-memory vector of events.
    pub fn from_events(events: Vec<WindowEvent>) -> Self {
        Self { events }
    }
}

#[async_trait]
impl ContextExtractor for MockExtractor {
    async fn subscribe_focus(&self) -> Result<mpsc::Receiver<WindowEvent>> {
        let (tx, rx) = mpsc::channel(16);
        let events = self.events.clone();

        tokio::spawn(async move {
            // Loop forever — the daemon is long-lived
            loop {
                for event in &events {
                    if tx.send(event.clone()).await.is_err() {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
                tracing::debug!("Mock cycle complete, restarting...");
            }
        });

        Ok(rx)
    }
}
