use anyhow::Result;
use chrono::DateTime;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};
use ratatui::{Frame, Terminal};
use std::io::stdout;
use std::time::Duration;

const API_BASE: &str = "http://127.0.0.1:2198";

#[derive(PartialEq)]
enum Tab {
    Dashboard,
    Settings,
}

struct DashboardData {
    total_events: i64,
    total_apps: i64,
    total_hours: f64,
    today_events: i64,
    most_used: String,
    top_apps: Vec<(String, i64, i64)>,
    recent: Vec<(String, String, String, String, String)>,
    config: String,
    llm_provider: String,
    llm_model: String,
    api_ok: bool,
}

impl Default for DashboardData {
    fn default() -> Self {
        Self {
            total_events: 0, total_apps: 0, total_hours: 0.0, today_events: 0,
            most_used: String::new(), top_apps: Vec::new(), recent: Vec::new(),
            config: String::new(), llm_provider: "ollama".into(), llm_model: "?".into(), api_ok: false,
        }
    }
}

fn fetch_json(path: &str) -> Result<serde_json::Value> {
    let body = ureq::get(&format!("{}{}", API_BASE, path)).call()?.into_body().read_to_string()?;
    Ok(serde_json::from_str(&body)?)
}

fn fetch_dashboard_data() -> DashboardData {
    let mut data = DashboardData::default();
    if let Ok(v) = fetch_json("/api/stats/overview") {
        data.total_events = v["total_events"].as_i64().unwrap_or(0);
        data.total_apps = v["total_apps"].as_i64().unwrap_or(0);
        data.total_hours = v["total_tracked_hours"].as_f64().unwrap_or(0.0);
        data.today_events = v["today_events"].as_i64().unwrap_or(0);
        data.most_used = v["most_used_app"].as_str().unwrap_or("").to_string();
        data.api_ok = true;
    }
    if let Ok(v) = fetch_json("/api/stats/apps") {
        if let Some(arr) = v.as_array() {
            for app in arr.iter().take(8) {
                data.top_apps.push((
                    app["app_name"].as_str().unwrap_or("?").to_string(),
                    app["count"].as_i64().unwrap_or(0),
                    app["duration_ms"].as_i64().unwrap_or(0),
                ));
            }
        }
    }
    if let Ok(v) = fetch_json("/api/stats/recent") {
        if let Some(arr) = v.as_array() {
            for ev in arr.iter().take(12) {
                let t = ev["start_time"].as_i64().unwrap_or(0);
                let time = DateTime::from_timestamp_millis(t)
                    .map(|dt| dt.format("%H:%M").to_string()).unwrap_or_else(|| "??:??".into());
                let dur_ms = ev["duration_ms"].as_i64().unwrap_or(0);
                let dur = if dur_ms < 60_000 { format!("{}s", dur_ms / 1000) } else { format!("{}m", dur_ms / 60_000) };
                data.recent.push((
                    time,
                    ev["app_name"].as_str().unwrap_or("?").to_string(),
                    ev["window_title"].as_str().unwrap_or("?").to_string(),
                    ev["source_type"].as_str().unwrap_or("text").to_string(),
                    dur,
                ));
            }
        }
    }
    if let Ok(v) = fetch_json("/api/settings") {
        data.llm_provider = v["llm"]["active_provider"].as_str().unwrap_or("?").to_string();
        if let Some(p) = v["llm"]["providers"][&data.llm_provider].as_object() {
            data.llm_model = p["model"].as_str().unwrap_or("?").to_string();
        }
        if let Ok(s) = serde_json::to_string_pretty(&v) {
            data.config = s;
        }
    }
    data
}

pub fn run_dashboard() -> Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(ratatui::backend::CrosstermBackend::new(stdout()))?;
    let mut tab = Tab::Dashboard;

    loop {
        let data = fetch_dashboard_data();
        terminal.draw(|f| draw(f, &tab, &data))?;
        if event::poll(Duration::from_secs(3))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('1') => tab = Tab::Dashboard,
                        KeyCode::Char('2') => tab = Tab::Settings,
                        KeyCode::Tab => tab = match tab { Tab::Dashboard => Tab::Settings, Tab::Settings => Tab::Dashboard },
                        _ => {}
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn draw(f: &mut Frame, tab: &Tab, data: &DashboardData) {
    let size = f.area();
    if size.width < 80 || size.height < 20 { return; }

    let ac = Color::Rgb(0, 212, 170);
    let dim = Color::Rgb(90, 90, 120);

    let top = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(5), Constraint::Min(1), Constraint::Length(3)])
        .split(size);

    // === TOP BAR ===
    let logo_text = vec![
        Line::from(Span::styled("  ▄▄▄       ██▓    ▓█████   ██▓███   ██░ ██", Style::default().fg(ac))),
        Line::from(Span::styled("  ██▀▀█▄    ▓██▒    ▓█   ▀  ▓██░  ██▒▓██░ ██▒", Style::default().fg(ac))),
        Line::from(Span::styled("  ██    ██  ▒██░    ▒███    ▓██░ ██▓▒▒██▀▀██░", Style::default().fg(Color::Rgb(0, 180, 150)))),
    ];
    let title = vec![
        Line::from(Span::styled("Aleph Context Store", Style::default().fg(ac).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("Local-First Desktop Memory", Style::default().fg(dim))),
    ];
    let llm_color = if data.api_ok { ac } else { Color::Rgb(239, 68, 68) };
    let llm_dot = if data.api_ok { "●" } else { "○" };
    let llm_line = Line::from(vec![
        Span::styled(format!(" {} ", llm_dot), Style::default().fg(llm_color)),
        Span::styled(format!("{}", data.llm_provider), Style::default().fg(Color::Rgb(200, 200, 220))),
        Span::styled(format!(" ({})", data.llm_model), Style::default().fg(dim)),
    ]);

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(35), Constraint::Min(1), Constraint::Length(24)])
        .split(top[0]);
    f.render_widget(Paragraph::new(logo_text), top_chunks[0]);
    f.render_widget(Paragraph::new(title).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(ac))), top_chunks[1]);
    f.render_widget(Paragraph::new(llm_line).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(llm_color))), top_chunks[2]);

    // === STATS ROW ===
    let cards = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25); 4])
        .split(top[1]);

    let stats = [
        ("Total Events", &data.total_events.to_string(), Color::Rgb(0, 212, 170)),
        ("Applications", &data.total_apps.to_string(), Color::Rgb(168, 85, 247)),
        ("Hours Tracked", &format!("{:.1}", data.total_hours), Color::Rgb(59, 130, 246)),
        ("Today", &data.today_events.to_string(), Color::Rgb(245, 158, 11)),
    ];

    for (i, (label, value, color)) in stats.iter().enumerate() {
        let card = Paragraph::new(vec![
            Line::from(Span::styled(*label, Style::default().fg(dim))),
            Line::from(Span::styled(format!(" {}", value), Style::default().fg(*color).add_modifier(Modifier::BOLD))),
        ]);
        f.render_widget(card, cards[i]);
    }

    // === CONTENT ===
    match tab {
        Tab::Dashboard => draw_dashboard(f, top[2], data),
        Tab::Settings => draw_settings(f, top[2], data),
    }

    // === FOOTER ===
    let help = format!(" [1] Dashboard  [2] Settings  [Tab] Switch  [q] Quit  ▲ {} events  refresh: 3s", data.total_events);
    let footer = Paragraph::new(Line::from(Span::styled(help, Style::default().fg(dim))))
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(ac)));
    f.render_widget(footer, top[3]);
}

fn draw_dashboard(f: &mut Frame, area: Rect, data: &DashboardData) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    // Top apps as a table
    let max_count = data.top_apps.first().map(|(_, c, _)| *c).unwrap_or(1).max(1);
    let app_rows: Vec<Row> = data.top_apps.iter().map(|(name, count, dur)| {
        let pct = (*count as f64 / max_count as f64) * 100.0;
        let bar = "█".repeat((pct / 5.0) as usize);
        let dur_str = if *dur > 0 { format!("{:.1}h", *dur as f64 / 3_600_000.0) } else { "".to_string() };
        Row::new(vec![
            Cell::from(Span::styled(format!(" {}", name), Style::default().fg(Color::Rgb(200, 200, 220)))),
            Cell::from(Span::styled(format!("{}", count), Style::default().fg(Color::Rgb(0, 212, 170)).add_modifier(Modifier::BOLD))),
            Cell::from(Span::styled(bar, Style::default().fg(Color::Rgb(0, 212, 170)))),
            Cell::from(Span::styled(dur_str, Style::default().fg(dim(100)))),
        ])
    }).collect();

    let apps_table = Table::new(app_rows, [Constraint::Length(14), Constraint::Length(6), Constraint::Min(1), Constraint::Length(8)])
        .header(Row::new(vec![" App", "Count", "", "Duration"].iter().map(|h| Cell::from(Span::styled(*h, Style::default().fg(dim(120)))))))
        .block(Block::default().title(" Top Applications ").borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(168, 85, 247))));
    f.render_widget(apps_table, chunks[0]);

    // Recent events as a table
    let recent_rows: Vec<Row> = data.recent.iter().map(|(time, app, title, st, dur)| {
        let dot = if st == "vision" { "●" } else { "○" };
        let dot_color = if st == "vision" { Color::Rgb(168, 85, 247) } else { Color::Rgb(0, 212, 170) };
        Row::new(vec![
            Cell::from(Span::styled(format!(" {}", time), Style::default().fg(dim(120)))),
            Cell::from(Span::styled(format!(" {}", dot), Style::default().fg(dot_color))),
            Cell::from(Span::styled(format!(" {}", app), Style::default().fg(Color::Rgb(180, 180, 200)))),
            Cell::from(Span::styled(format!(" {}", title), Style::default().fg(Color::Rgb(140, 140, 160)))),
            Cell::from(Span::styled(format!(" {}", dur), Style::default().fg(dim(100)))),
        ])
    }).collect();

    let recent_table = Table::new(recent_rows, [
        Constraint::Length(6), Constraint::Length(2), Constraint::Length(12),
        Constraint::Min(1), Constraint::Length(6),
    ])
    .header(Row::new(vec![" Time", "", "App", "Window", "Dur"].iter().map(|h| Cell::from(Span::styled(*h, Style::default().fg(dim(120)))))))
    .block(Block::default().title(" Recent Activity ").borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(59, 130, 246))));
    f.render_widget(recent_table, chunks[1]);
}

fn draw_settings(f: &mut Frame, area: Rect, data: &DashboardData) {
    let lines: Vec<Line> = data.config.lines().map(|l| {
        let color = if l.contains('[') || l.contains('=') { Color::Rgb(0, 212, 170) } else { Color::Rgb(180, 180, 200) };
        Line::from(Span::styled(format!(" {}", l), Style::default().fg(color)))
    }).collect();

    let p = Paragraph::new(lines)
        .block(Block::default()
            .title(" Settings  (edit via http://localhost:2198/settings) ")
            .borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(245, 158, 11))))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn dim(v: u8) -> Color { Color::Rgb(v, v, v + 20) }
