use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Wrap};
use ratatui::Frame;

use crate::ipc::protocol::WireState;
use crate::tui::{App, Page};

pub fn render(frame: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let tabs = Tabs::new([
        crate::t!("tui.tab_status"),
        crate::t!("tui.tab_history"),
        crate::t!("tui.tab_settings"),
    ])
    .select(app.page.index())
    .style(Style::default().fg(Color::Gray))
    .highlight_style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
    .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(tabs, root[0]);

    match app.page {
        Page::Status => render_status(frame, app, root[1]),
        Page::History => render_history(frame, app, root[1]),
        Page::Settings => render_settings(frame, app, root[1]),
    }

    frame.render_widget(
        Paragraph::new(crate::t!("tui.footer", status = app.status.clone())),
        root[2],
    );
}

fn render_status(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(5),
            Constraint::Length(7),
        ])
        .split(area);

    let state_color = match app.state {
        WireState::Idle => Color::Green,
        WireState::Recording => Color::Red,
        WireState::Stopping => Color::Yellow,
        WireState::Error => Color::LightRed,
    };
    let elapsed_ms = app.current_elapsed_ms();
    let app_label = app
        .app_name
        .clone()
        .or_else(|| app.app.clone())
        .unwrap_or_else(|| crate::t!("tui.no_active_app"));
    let header = vec![
        Line::from(vec![
            Span::styled(
                state_label(app.state),
                Style::default()
                    .fg(state_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::raw(
                app.recording_id
                    .clone()
                    .unwrap_or_else(|| crate::t!("tui.no_active_recording")),
            ),
        ]),
        Line::from(crate::t!(
            "tui.current_line",
            app = app_label,
            elapsed = format_duration(elapsed_ms),
            words = app.words,
            history = app.history.len(),
        )),
        Line::from(crate::t!(
            "tui.bundle_line",
            bundle = app.app.clone().unwrap_or_else(|| "-".to_string()),
        )),
    ];
    frame.render_widget(
        Paragraph::new(header).block(
            Block::default()
                .title(crate::t!("tui.current"))
                .borders(Borders::ALL),
        ),
        chunks[0],
    );

    let live_text = format!("{}{}", app.segments.join(""), app.partial);
    frame.render_widget(
        Paragraph::new(live_text).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(crate::t!("tui.live_speech"))
                .borders(Borders::ALL),
        ),
        chunks[1],
    );

    let pipeline = if app.pipeline.is_empty() {
        crate::t!("tui.no_pipeline_steps")
    } else {
        app.pipeline.join("\n")
    };
    frame.render_widget(
        Paragraph::new(pipeline).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(crate::t!("tui.pipeline"))
                .borders(Borders::ALL),
        ),
        chunks[2],
    );
}

fn render_history(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(0),
            Constraint::Length(6),
        ])
        .split(area);
    let summary = HistorySummary::from(app);
    let search = if app.searching {
        format!("/{}_", app.search)
    } else if app.search.is_empty() {
        crate::t!("tui.search_prompt")
    } else {
        format!("/{}", app.search)
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(search),
            Line::from(crate::t!(
                "tui.history_stats_line",
                shown = summary.shown,
                total = summary.total,
                duration = format_duration(summary.total_duration_ms),
                words = summary.total_words,
                avg = format_duration(summary.avg_duration_ms),
            )),
        ])
        .block(
            Block::default()
                .title(crate::t!("tui.history_stats"))
                .borders(Borders::ALL),
        ),
        chunks[0],
    );

    let records = app.filtered_history();
    let items: Vec<ListItem> = records
        .iter()
        .enumerate()
        .map(|(idx, record)| {
            let marker = if idx == app.selected_history {
                "> "
            } else {
                "  "
            };
            let app_name = record.app.as_deref().unwrap_or("-");
            ListItem::new(format!(
                "{marker}{}  {}  {}ms  {}",
                record.started_at,
                app_name,
                record.duration_ms,
                record.text.replace('\n', " ")
            ))
        })
        .collect();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(crate::t!("tui.history_newest_first"))
                .borders(Borders::ALL),
        ),
        chunks[1],
    );

    let selected = records
        .get(app.selected_history)
        .map(|record| record.text.clone())
        .unwrap_or_else(|| crate::t!("tui.no_history_selected"));
    frame.render_widget(
        Paragraph::new(selected).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(crate::t!("tui.selected_final_text"))
                .borders(Borders::ALL),
        ),
        chunks[2],
    );
}

struct HistorySummary {
    total: usize,
    shown: usize,
    total_duration_ms: u64,
    avg_duration_ms: u64,
    total_words: usize,
}

impl HistorySummary {
    fn from(app: &App) -> Self {
        let filtered = app.filtered_history();
        let total_duration_ms = app
            .history
            .iter()
            .map(|record| record.duration_ms)
            .sum::<u64>();
        let total_words = app
            .history
            .iter()
            .map(|record| record.text_stats().words)
            .sum::<usize>();
        let avg_duration_ms = if app.history.is_empty() {
            0
        } else {
            total_duration_ms / app.history.len() as u64
        };
        Self {
            total: app.history.len(),
            shown: filtered.len(),
            total_duration_ms,
            avg_duration_ms,
            total_words,
        }
    }
}

fn state_label(state: WireState) -> String {
    match state {
        WireState::Idle => crate::t!("tui.state_idle"),
        WireState::Recording => crate::t!("tui.state_recording"),
        WireState::Stopping => crate::t!("tui.state_stopping"),
        WireState::Error => crate::t!("tui.state_error"),
    }
}

fn format_duration(ms: u64) -> String {
    let seconds = ms / 1000;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    if hours > 0 {
        format!("{hours}:{:02}:{:02}", minutes % 60, seconds % 60)
    } else {
        format!("{:02}:{:02}", minutes, seconds % 60)
    }
}

fn render_settings(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(4),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(app.config_path.as_str()).block(
            Block::default()
                .title(crate::t!("tui.config_path"))
                .borders(Borders::ALL),
        ),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(app.config_body.as_str())
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(crate::t!("tui.current_config"))
                    .borders(Borders::ALL),
            ),
        chunks[1],
    );
    frame.render_widget(
        Paragraph::new(crate::t!("tui.doctor_m5"))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(crate::t!("tui.doctor"))
                    .borders(Borders::ALL),
            ),
        chunks[2],
    );
}
