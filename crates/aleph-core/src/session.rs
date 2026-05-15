use crate::models::RecentEvent;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: usize,
    pub start_time: i64,
    pub end_time: i64,
    pub duration_ms: i64,
    pub apps: Vec<String>,
    pub category: String,
    pub event_count: usize,
    pub gap_before_ms: i64,
}

const GAP_THRESHOLD_MS: i64 = 15 * 60 * 1000; // 15 minutes

/// Group events into sessions based on time gaps.
pub fn detect_sessions(events: &[RecentEvent]) -> Vec<Session> {
    if events.is_empty() {
        return vec![];
    }

    let mut sorted: Vec<&RecentEvent> = events.iter().collect();
    sorted.sort_by_key(|e| e.start_time);

    let mut sessions: Vec<Session> = Vec::new();
    let mut session_events: Vec<&RecentEvent> = Vec::new();

    for ev in &sorted {
        if session_events.is_empty() {
            session_events.push(ev);
            continue;
        }

        let last = session_events[session_events.len() - 1];
        let gap = ev.start_time - last.end_time.unwrap_or(last.start_time);

        if gap <= GAP_THRESHOLD_MS {
            // Same session
            session_events.push(ev);
        } else {
            // Flush current session
            sessions.push(build_session(&session_events, &sessions));
            session_events.clear();
            session_events.push(ev);
        }
    }

    // Flush last session
    if !session_events.is_empty() {
        sessions.push(build_session(&session_events, &sessions));
    }

    sessions
}

fn build_session(events: &[&RecentEvent], existing: &[Session]) -> Session {
    let start_time = events.first().unwrap().start_time;
    let end_time = events.last().unwrap().end_time.unwrap_or(events.last().unwrap().start_time);
    let duration_ms = end_time - start_time;

    let mut apps: Vec<String> = events.iter().map(|e| e.app_name.clone()).collect();
    apps.sort();
    apps.dedup();

    // Most common category
    let mut cat_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for ev in events {
        let cat = ev.category.as_deref().unwrap_or("other");
        *cat_counts.entry(cat).or_insert(0) += 1;
    }
    let category = cat_counts.into_iter()
        .max_by_key(|&(_, c)| c)
        .map(|(cat, _)| cat.to_string())
        .unwrap_or_else(|| "other".to_string());

    let id = existing.len();

    // Gap before this session
    let gap_before_ms = if id > 0 {
        let prev = &existing[id - 1];
        start_time - prev.end_time
    } else {
        0
    };

    Session {
        id,
        start_time,
        end_time,
        duration_ms,
        apps,
        category,
        event_count: events.len(),
        gap_before_ms,
    }
}
