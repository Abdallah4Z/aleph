use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use chrono::DateTime;
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
    top_apps: Vec<(String, i64)>,
    recent: Vec<String>,
    config: String,
}

impl Default for DashboardData {
    fn default() -> Self {
        Self {
            total_events: 0,
            total_apps: 0,
            total_hours: 0.0,
            today_events: 0,
            top_apps: Vec::new(),
            recent: Vec::new(),
            config: String::new(),
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

    // Overview
    if let Ok(v) = fetch_json("/api/stats/overview") {
        data.total_events = v["total_events"].as_i64().unwrap_or(0);
        data.total_apps = v["total_apps"].as_i64().unwrap_or(0);
        data.total_hours = v["total_tracked_hours"].as_f64().unwrap_or(0.0);
        data.today_events = v["today_events"].as_i64().unwrap_or(0);
    }

    // Top apps
    if let Ok(v) = fetch_json("/api/stats/apps") {
        if let Some(arr) = v.as_array() {
            for app in arr.iter().take(8) {
                let name = app["app_name"].as_str().unwrap_or("?").to_string();
                let count = app["count"].as_i64().unwrap_or(0);
                data.top_apps.push((name, count));
            }
        }
    }

    // Recent events
    if let Ok(v) = fetch_json("/api/stats/recent") {
        if let Some(arr) = v.as_array() {
            for ev in arr.iter().take(10) {
                let app = ev["app_name"].as_str().unwrap_or("?");
                let title = ev["window_title"].as_str().unwrap_or("?");
                let t = ev["start_time"].as_i64().unwrap_or(0);
                let time = DateTime::from_timestamp_millis(t)
                    .map(|dt| dt.format("%H:%M").to_string())
                    .unwrap_or_else(|| "??:??".into());
                let duration_ms = ev["duration_ms"].as_i64().unwrap_or(0);
                let dur = if duration_ms < 60_000 {
                    format!("{}s", duration_ms / 1000)
                } else {
                    format!("{}m", duration_ms / 60_000)
                };
                data.recent.push(format!("{} {} — {} ({})", time, app, title, dur));
            }
        }
    }

    // Config
    if let Ok(v) = fetch_json("/api/settings") {
        if let Ok(s) = serde_json::to_string_pretty(&v) {
            data.config = s;
        }
    }

    data
}

fn logo_text() -> Text<'static> {
    Text::from(vec![
        Line::from(vec![
            Span::styled("  ╔═══╗", Style::default().fg(Color::Rgb(0, 212, 170)).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  ║ A ║", Style::default().fg(Color::Rgb(0, 212, 170)).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  ╚═══╝", Style::default().fg(Color::Rgb(0, 212, 170)).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled("  Aleph", Style::default().fg(Color::Rgb(0, 212, 170)).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("  Context Store", Style::default().fg(Color::Rgb(100, 100, 140)))),
    ])
}

pub fn run_dashboard() -> Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(ratatui::backend::CrosstermBackend::new(stdout()))?;

    let mut tab = Tab::Dashboard;
    let mut data: DashboardData;

    let result = loop {
        data = fetch_dashboard_data();

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

    // Main layout: logo, content, footer
    let main = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(size);

    // Logo area
    f.render_widget(
        Paragraph::new(logo_text()).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(0, 212, 170)))),
        main[0],
    );

    // Content area
    match tab {
        Tab::Dashboard => draw_dashboard(f, main[1], data),
        Tab::Settings => draw_settings(f, main[1], data),
    }

    // Footer help
    let help_text = match tab {
        Tab::Dashboard => "  [1] Dashboard  [2] Settings  [r] Refresh  [q] Quit  ",
        Tab::Settings => "  [1] Dashboard  [2] Settings  [r] Refresh  [q] Quit  ",
    };
    let footer = Paragraph::new(Line::from(Span::styled(
        help_text,
        Style::default().fg(Color::Rgb(80, 80, 120)),
    )))
    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(0, 212, 170))));
    f.render_widget(footer, main[2]);
}

fn draw_dashboard(f: &mut Frame, area: Rect, data: &DashboardData) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(1)])
        .split(area);

    // Stats cards row
    let cards = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(chunks[0]);

    let stats = [
        ("Total Events", &data.total_events.to_string(), Color::Rgb(0, 212, 170)),
        ("Applications", &data.total_apps.to_string(), Color::Rgb(168, 85, 247)),
        ("Hours Tracked", &format!("{:.1}", data.total_hours), Color::Rgb(59, 130, 246)),
        ("Today", &data.today_events.to_string(), Color::Rgb(245, 158, 11)),
    ];

    for (i, (label, value, color)) in stats.iter().enumerate() {
        let card = Paragraph::new(vec![
            Line::from(Span::styled(
                *label,
                Style::default().fg(Color::Rgb(100, 100, 140)),
            )),
            Line::from(Span::styled(
                format!(" {}", value),
                Style::default().fg(*color).add_modifier(Modifier::BOLD),
            )),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(*color)),
        );
        f.render_widget(card, cards[i]);
    }

    // Apps + Recent
    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    // Top apps list
    let app_items: Vec<ListItem> = data
        .top_apps
        .iter()
        .map(|(name, count)| {
            let max_count = data.top_apps.first().map(|(_, c)| *c).unwrap_or(1);
            let bar_width = ((*count as f64 / max_count as f64) * 20.0) as usize;
            let bar = "█".repeat(bar_width);
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {:<12}", name),
                    Style::default().fg(Color::Rgb(200, 200, 220)),
                ),
                Span::styled(
                    format!(" {} ", count),
                    Style::default()
                        .fg(Color::Rgb(0, 212, 170))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(bar, Style::default().fg(Color::Rgb(0, 212, 170))),
            ]))
        })
        .collect();
    let apps_list = List::new(app_items)
        .block(
            Block::default()
                .title(" Top Applications ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(168, 85, 247))),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));
    f.render_widget(apps_list, bottom[0]);

    // Recent events list
    let recent_items: Vec<ListItem> = data
        .recent
        .iter()
        .map(|line| ListItem::new(Line::from(Span::styled(
            format!(" {}", line),
            Style::default().fg(Color::Rgb(180, 180, 200)),
        ))))
        .collect();
    let recent_list = List::new(recent_items)
        .block(
            Block::default()
                .title(" Recent Activity ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(59, 130, 246))),
        );
    f.render_widget(recent_list, bottom[1]);
}

fn draw_settings(f: &mut Frame, area: Rect, data: &DashboardData) {
    let lines: Vec<Line> = data
        .config
        .lines()
        .map(|l| {
            if l.contains('=') || l.contains('[') {
                Line::from(Span::styled(
                    format!(" {}", l),
                    Style::default().fg(Color::Rgb(0, 212, 170)),
                ))
            } else {
                Line::from(Span::styled(
                    format!(" {}", l),
                    Style::default().fg(Color::Rgb(180, 180, 200)),
                ))
            }
        })
        .collect();

    let settings = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Settings  (edit via http://localhost:2198/settings) ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(245, 158, 11))),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(settings, area);
}
