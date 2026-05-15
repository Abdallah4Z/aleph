use anyhow::Result;
use chrono::DateTime;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::io::stdout;
use std::time::Duration;

const API_BASE: &str = "http://127.0.0.1:2198";
const LOGO: &str = r"
▄▄▄       ██▓    ▓█████  ██▓███   ██░ ██
▒████▄    ▓██▒    ▓█   ▀ ▓██░  ██▒▓██░ ██▒
▒██  ▀█▄  ▒██░    ▒███   ▓██░ ██▓▒▒██▀▀██░
░██▄▄▄▄██ ▒██░    ▒▓█  ▄ ▒██▄█▓▒ ░██▓ ░██
 ▓█   ▓██▒░██████▒░▒████▒▒██▒ ░  ░██▓ ░██▓
 ▒▒   ▓▒█░░ ▒░▓  ░░░ ▒░ ░▒▓▒░ ░  ░▒ ░ ░▒ ▒
  ▒   ▒▒ ░░ ░ ▒  ░ ░ ░  ░░▒ ░     ░█  ░░ ░
  ░   ▒     ░ ░      ░   ░░       ░█  ░░
      ░  ░    ░  ░   ░  ░          ░";

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
    llm_url: String,
    api_ok: bool,
}

impl Default for DashboardData {
    fn default() -> Self {
        Self {
            total_events: 0,
            total_apps: 0,
            total_hours: 0.0,
            today_events: 0,
            most_used: String::new(),
            top_apps: Vec::new(),
            recent: Vec::new(),
            config: String::new(),
            llm_provider: "ollama".into(),
            llm_model: "qwen2.5:0.5b".into(),
            llm_url: "http://localhost:11434".into(),
            api_ok: false,
        }
    }
}

fn fetch_json(path: &str) -> Result<serde_json::Value> {
    let body = ureq::get(&format!("{}{}", API_BASE, path))
        .call()?
        .into_body()
        .read_to_string()?;
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
                let name = app["app_name"].as_str().unwrap_or("?").to_string();
                let count = app["count"].as_i64().unwrap_or(0);
                let duration = app["duration_ms"].as_i64().unwrap_or(0);
                data.top_apps.push((name, count, duration));
            }
        }
    }

    if let Ok(v) = fetch_json("/api/stats/recent") {
        if let Some(arr) = v.as_array() {
            for ev in arr.iter().take(10) {
                let app = ev["app_name"].as_str().unwrap_or("?").to_string();
                let title = ev["window_title"].as_str().unwrap_or("?").to_string();
                let st = ev["source_type"].as_str().unwrap_or("text").to_string();
                let t = ev["start_time"].as_i64().unwrap_or(0);
                let time = DateTime::from_timestamp_millis(t)
                    .map(|dt| dt.format("%H:%M").to_string())
                    .unwrap_or_else(|| "??:??".into());
                let dur_ms = ev["duration_ms"].as_i64().unwrap_or(0);
                let dur = if dur_ms < 60_000 { format!("{}s", dur_ms / 1000) } else { format!("{}m", dur_ms / 60_000) };
                data.recent.push((time, app, title, st, dur));
            }
        }
    }

    if let Ok(v) = fetch_json("/api/settings") {
        data.llm_provider = v["llm"]["provider"].as_str().unwrap_or("ollama").to_string();
        data.llm_model = v["llm"]["model"].as_str().unwrap_or("?").to_string();
        data.llm_url = v["llm"]["base_url"].as_str().unwrap_or("?").to_string();
        if let Ok(s) = serde_json::to_string_pretty(&v) {
            data.config = s;
        }
    }

    data
}

fn logo_text() -> Text<'static> {
    let lines: Vec<Line> = LOGO.lines().map(|l| {
        Line::from(Span::styled(l, Style::default().fg(Color::Rgb(0, 212, 170)).add_modifier(Modifier::BOLD)))
    }).collect();

    let mut text = Text::from(lines);
    text.extend(Text::from(Line::from(vec![
        Span::raw("  "),
        Span::styled("Context Store", Style::default().fg(Color::Rgb(100, 100, 140))),
        Span::styled("  │  ", Style::default().fg(Color::Rgb(60, 60, 80))),
        Span::styled("Local-First Desktop Memory", Style::default().fg(Color::Rgb(100, 100, 140))),
    ])));
    text
}

pub fn run_dashboard() -> Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(ratatui::backend::CrosstermBackend::new(stdout()))?;

    let mut tab = Tab::Dashboard;

    let result = loop {
        let data = fetch_dashboard_data();

        terminal.draw(|f| draw(f, &tab, &data))?;

        if event::poll(Duration::from_secs(3))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                        KeyCode::Char('1') => tab = Tab::Dashboard,
                        KeyCode::Char('2') => tab = Tab::Settings,
                        KeyCode::Char('r') => continue,
                        KeyCode::Tab => {
                            tab = match tab {
                                Tab::Dashboard => Tab::Settings,
                                Tab::Settings => Tab::Dashboard,
                            };
                        }
                        _ => {}
                    }
                }
            }
        }
    };

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    result
}

fn draw(f: &mut Frame, tab: &Tab, data: &DashboardData) {
    let size = f.area();
    let ac = Color::Rgb(0, 212, 170);
    let dim = Color::Rgb(80, 80, 120);
    let fg = Color::Rgb(200, 200, 220);

    // Logo block
    let logo_h = LOGO.lines().count() as u16 + 1;
    let main = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(logo_h + 2),
            Constraint::Length(6),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(size);

    // Logo
    f.render_widget(
        Paragraph::new(logo_text())
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(ac))),
        main[0],
    );

    // Stats cards
    let cards = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(20); 4])
        .split(main[1]);

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
        ])
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(*color)));
        f.render_widget(card, cards[i]);
    }

    // Content area
    match tab {
        Tab::Dashboard => draw_dashboard(f, main[2], data),
        Tab::Settings => draw_settings(f, main[2], data),
    }

    // Footer = LLM status + help
    let llm_status = format!(" {} ({})", data.llm_provider, data.llm_model);
    let llm_color = if data.api_ok { Color::Rgb(0, 212, 170) } else { Color::Rgb(239, 68, 68) };
    let llm_dot = if data.api_ok { "◉" } else { "○" };

    let footer_text = vec![
        Line::from(vec![
            Span::styled(" LLM: ", Style::default().fg(dim)),
            Span::styled(llm_dot, Style::default().fg(llm_color)),
            Span::styled(llm_status, Style::default().fg(fg)),
            Span::styled("  │  ", Style::default().fg(Color::Rgb(60, 60, 80))),
            Span::styled("[1] Dashboard  [2] Settings  [r] Refresh  [q] Quit", Style::default().fg(dim)),
        ]),
    ];
    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(ac)));
    f.render_widget(footer, main[3]);
}

fn draw_dashboard(f: &mut Frame, area: Rect, data: &DashboardData) {
    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    // Top apps
    let max_app_count = data.top_apps.first().map(|(_, c, _)| *c).unwrap_or(1);
    let app_items: Vec<ListItem> = data.top_apps.iter().map(|(name, count, dur)| {
        let bar_len = ((*count as f64 / max_app_count as f64) * 20.0) as usize;
        let bar = "█".repeat(bar_len);
        let dur_str = if *dur > 0 { format!("{}h", *dur as f64 / 3_600_000.0) } else { "".to_string() };
        ListItem::new(Line::from(vec![
            Span::styled(format!(" {:<14}", name), Style::default().fg(Color::Rgb(200, 200, 220))),
            Span::styled(format!("{:>4}", count), Style::default().fg(Color::Rgb(0, 212, 170)).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled(bar, Style::default().fg(Color::Rgb(0, 212, 170))),
            Span::styled(format!(" {}", dur_str), Style::default().fg(Color::Rgb(100, 100, 140))),
        ]))
    }).collect();

    f.render_widget(
        List::new(app_items)
            .block(Block::default().title(" Top Applications ").borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(168, 85, 247)))),
        bottom[0],
    );

    // Recent events
    let recent_items: Vec<ListItem> = data.recent.iter().map(|(time, app, title, st, dur)| {
        let dot = if st == "vision" { "●" } else { "○" };
        let dot_color = if st == "vision" { Color::Rgb(168, 85, 247) } else { Color::Rgb(0, 212, 170) };
        ListItem::new(Line::from(vec![
            Span::styled(format!(" {}", time), Style::default().fg(Color::Rgb(100, 100, 140))),
            Span::raw(" "),
            Span::styled(dot, Style::default().fg(dot_color)),
            Span::raw(" "),
            Span::styled(format!("{:<12}", app), Style::default().fg(Color::Rgb(180, 180, 200))),
            Span::styled(format!(" {}", title), Style::default().fg(Color::Rgb(140, 140, 160))),
            Span::styled(format!(" ({})", dur), Style::default().fg(Color::Rgb(80, 80, 120))),
        ]))
    }).collect();

    f.render_widget(
        List::new(recent_items)
            .block(Block::default().title(" Recent Activity ").borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(59, 130, 246)))),
        bottom[1],
    );
}

fn draw_settings(f: &mut Frame, area: Rect, data: &DashboardData) {
    let lines: Vec<Line> = data.config.lines().map(|l| {
        let color = if l.contains('[') || l.contains('=') {
            Color::Rgb(0, 212, 170)
        } else {
            Color::Rgb(180, 180, 200)
        };
        Line::from(Span::styled(format!(" {}", l), Style::default().fg(color)))
    }).collect();

    let settings = Paragraph::new(lines)
        .block(Block::default()
            .title(" Settings  (edit via http://localhost:2198/settings) ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(245, 158, 11))))
        .wrap(Wrap { trim: false });
    f.render_widget(settings, area);
}
