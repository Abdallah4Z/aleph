use crate::{Database, TextEncoder};
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;

/// Background task that periodically checks for similar past sessions
/// and creates memory notifications. Runs every 5 minutes.
pub async fn run_memory_engine(
    db: Database,
    text_encoder: Arc<dyn TextEncoder + Send + Sync>,
) -> Result<()> {
    let mut interval = tokio::time::interval(Duration::from_secs(300));
    let mut last_event_id: i64 = 0;

    loop {
        interval.tick().await;

        let events = match db.get_recent_events(10).await {
            Ok(e) => e,
            Err(e) => { tracing::warn!("Memory: fetch events failed: {}", e); continue; }
        };

        if events.is_empty() || events[0].id == last_event_id { continue; }
        last_event_id = events[0].id;

        let context: Vec<String> = events.iter()
            .take(5)
            .map(|e| format!("{}: {}", e.app_name, e.window_title))
            .collect();
        let context_text = context.join(" | ");
        if context_text.len() < 20 { continue; }

        let embedding = match text_encoder.encode(&context_text) {
            Ok(v) => v,
            Err(e) => { tracing::warn!("Memory: encode failed: {}", e); continue; }
        };

        let recent = db.get_last_n_vectors("text_vectors", 100).await.unwrap_or_default();
        let recent_images = db.get_last_n_vectors("image_vectors", 100).await.unwrap_or_default();
        let all_vectors: Vec<(i64, Vec<f32>)> = recent.into_iter().chain(recent_images).collect();

        let mut best_match: Option<(i64, f32)> = None;
        for (meta_id, vec) in &all_vectors {
            let sim = crate::dedup::cosine_similarity(&embedding, vec);
            if sim > 0.75 && (best_match.is_none() || sim > best_match.unwrap().1) {
                if let Ok(Some(ev)) = db.get_event_by_id(*meta_id).await {
                    let now_ms = chrono::Utc::now().timestamp_millis();
                    if now_ms - ev.start_time > 3600_000 {
                        best_match = Some((*meta_id, sim));
                    }
                }
            }
        }

        if let Some((match_id, sim)) = best_match {
            let existing = db.get_memories(5).await.unwrap_or_default();
            let already_exists = existing.iter().any(|(_, content, _, _, _)| {
                content.contains(&format!("id={}", match_id))
            });
            if !already_exists {
                let content = format!(
                    "You were in a similar context before (match id={}, similarity={:.2}): {}",
                    match_id, sim, context_text
                );
                let source_ids = events.iter().map(|e| e.id.to_string()).collect::<Vec<_>>().join(",");
                if let Ok(mem_id) = db.insert_memory(&content, &source_ids, sim).await {
                    tracing::info!("Memory created: id={}, event={}, sim={:.2}", mem_id, match_id, sim);
                }
            }
        }
    }
}
