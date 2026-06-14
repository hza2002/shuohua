pub mod keybindings;
pub mod panes;

use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event as CrosstermEvent, KeyEvent};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::ipc::client::IpcClient;
use crate::ipc::protocol::{Command, Event, WireState};
use crate::state::history::HistoryRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Status,
    History,
    Settings,
}

impl Page {
    fn index(self) -> usize {
        match self {
            Page::Status => 0,
            Page::History => 1,
            Page::Settings => 2,
        }
    }
}

#[derive(Debug)]
pub struct App {
    pub page: Page,
    pub state: WireState,
    pub recording_id: Option<String>,
    pub started_at: Option<time::OffsetDateTime>,
    pub app: Option<String>,
    pub app_name: Option<String>,
    pub dur_ms: u64,
    pub chars: u32,
    pub words: u32,
    pub segments: Vec<String>,
    pub partial: String,
    pub pipeline: Vec<String>,
    pub history: Vec<HistoryRecord>,
    pub selected_history: usize,
    pub search: String,
    pub searching: bool,
    pub status: String,
    pub config_path: String,
    pub config_body: String,
}

impl App {
    fn new() -> Self {
        let config_path = crate::config::default_path();
        let config_body = std::fs::read_to_string(&config_path)
            .unwrap_or_else(|e| format!("Failed to read config: {e}"));
        Self {
            page: Page::Status,
            state: WireState::Idle,
            recording_id: None,
            started_at: None,
            app: None,
            app_name: None,
            dur_ms: 0,
            chars: 0,
            words: 0,
            segments: Vec::new(),
            partial: String::new(),
            pipeline: Vec::new(),
            history: Vec::new(),
            selected_history: 0,
            search: String::new(),
            searching: false,
            status: "connected".to_string(),
            config_path: config_path.display().to_string(),
            config_body,
        }
    }

    pub fn filtered_history(&self) -> Vec<&HistoryRecord> {
        if self.search.is_empty() {
            return self.history.iter().collect();
        }
        let query = self.search.to_lowercase();
        self.history
            .iter()
            .filter(|record| {
                [
                    record.id.as_str(),
                    record.app.as_deref().unwrap_or_default(),
                    record.asr.raw.as_str(),
                    record.final_text(),
                ]
                .join("\n")
                .to_lowercase()
                .contains(&query)
            })
            .collect()
    }

    pub fn current_elapsed_ms(&self) -> u64 {
        if matches!(self.state, WireState::Recording | WireState::Stopping) {
            if let Some(started_at) = self.started_at {
                if let Ok(duration) = (time::OffsetDateTime::now_utc() - started_at).try_into() {
                    let duration: std::time::Duration = duration;
                    return duration.as_millis() as u64;
                }
            }
        }
        self.dur_ms
    }

    fn apply_event(&mut self, event: Event) {
        match event {
            Event::Snapshot {
                state,
                recording,
                started_at,
                app,
                app_name,
                dur_ms,
                chars,
                words,
                segments,
                partial,
                ..
            } => {
                self.state = state;
                self.recording_id = recording;
                self.started_at = parse_time(started_at.as_deref());
                self.app = app;
                self.app_name = app_name;
                self.dur_ms = dur_ms;
                self.chars = chars;
                self.words = words;
                self.segments = segments;
                self.partial = partial;
            }
            Event::StateChanged {
                state,
                recording_id,
                started_at,
                ..
            } => {
                self.state = state;
                self.recording_id = recording_id;
                self.started_at = parse_time(started_at.as_deref());
                if state == WireState::Idle {
                    self.segments.clear();
                    self.partial.clear();
                    self.pipeline.clear();
                    self.app = None;
                    self.app_name = None;
                    self.dur_ms = 0;
                    self.chars = 0;
                    self.words = 0;
                }
            }
            Event::AppChanged { app, app_name } => {
                self.app = app;
                self.app_name = app_name;
            }
            Event::StatsChanged {
                dur_ms,
                chars,
                words,
            } => {
                self.dur_ms = dur_ms;
                self.chars = chars;
                self.words = words;
            }
            Event::Partial { text, .. } => self.partial = text,
            Event::Segment { text, .. } => {
                self.segments.push(text);
                self.partial.clear();
            }
            Event::PipelineStep {
                name,
                status,
                duration_ms,
                text,
                error,
                ..
            } => {
                let detail = text.or(error).unwrap_or_default();
                self.pipeline
                    .push(format!("{name} {status} {duration_ms:.1}ms  {detail}"));
            }
            Event::HistoryAppended { record } => {
                self.history.insert(0, *record);
                self.selected_history = self
                    .selected_history
                    .min(self.history.len().saturating_sub(1));
            }
            Event::History { records } => {
                self.history = records;
                self.selected_history = 0;
            }
            Event::DaemonStatus { .. } => {}
            Event::ConfigReloaded { path } => {
                self.status = format!("config reloaded: {path}");
            }
            Event::Error { kind, msg, .. } => {
                self.status = format!("{kind}: {msg}");
            }
        }
    }
}

fn parse_time(value: Option<&str>) -> Option<time::OffsetDateTime> {
    value.and_then(|value| {
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
    })
}

pub async fn run() -> Result<()> {
    let mut client = IpcClient::connect(crate::ipc::server::default_socket_path()).await?;
    client.send(&Command::Subscribe).await?;
    client
        .send(&Command::GetHistory {
            limit: 50,
            before: None,
            query: None,
        })
        .await?;

    let _terminal = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let (key_tx, mut key_rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || loop {
        match event::read() {
            Ok(CrosstermEvent::Key(key)) => {
                if key_tx.send(key).is_err() {
                    return;
                }
            }
            Ok(_) => {}
            Err(_) => return,
        }
    });

    let mut app = App::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        terminal.draw(|frame| panes::render(frame, &app))?;
        tokio::select! {
            _ = tick.tick() => {}
            maybe_key = key_rx.recv() => {
                let Some(key) = maybe_key else { break; };
                if handle_key(&mut app, key)? {
                    break;
                }
            }
            event = client.recv() => {
                match event.context("read IPC event")? {
                    Some(event) => app.apply_event(event),
                    None => break,
                }
            }
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    use keybindings::Action;
    match keybindings::action_for(key, app.searching) {
        Action::Quit => return Ok(true),
        Action::NextPage => {
            app.page = match app.page {
                Page::Status => Page::History,
                Page::History => Page::Settings,
                Page::Settings => Page::Status,
            }
        }
        Action::PrevPage => {
            app.page = match app.page {
                Page::Status => Page::Settings,
                Page::History => Page::Status,
                Page::Settings => Page::History,
            }
        }
        Action::SetPage(page) => app.page = page,
        Action::MoveDown => {
            let len = app.filtered_history().len();
            if len > 0 {
                app.selected_history = (app.selected_history + 1).min(len - 1);
            }
        }
        Action::MoveUp => {
            app.selected_history = app.selected_history.saturating_sub(1);
        }
        Action::StartSearch => {
            app.page = Page::History;
            app.searching = true;
        }
        Action::CancelSearch => {
            app.searching = false;
        }
        Action::ClearSearch => {
            app.search.clear();
            app.searching = false;
            app.selected_history = 0;
        }
        Action::SearchChar(ch) => {
            app.search.push(ch);
            app.selected_history = 0;
        }
        Action::Backspace => {
            app.search.pop();
            app.selected_history = 0;
        }
        Action::CopySelected => {
            if app.page == Page::History {
                let text = app
                    .filtered_history()
                    .get(app.selected_history)
                    .map(|record| record.final_text().to_string());
                if let Some(text) = text {
                    crate::clipboard_darwin::write_string(&text)?;
                    app.status = "copied selected history text".to_string();
                }
            }
        }
        Action::None => {}
    }
    Ok(false)
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(std::io::stdout(), EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}
