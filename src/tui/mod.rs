pub mod audio;
pub mod keybindings;
pub mod panes;
pub mod settings;

use std::collections::HashMap;
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
use crate::state::{AudioMeter, SessionMeta, SessionPhase};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryDetail {
    Details,
    Asr,
    Pipeline,
    Sessions,
    Error,
    Json,
}

impl HistoryDetail {
    fn next(self) -> Self {
        match self {
            Self::Details => Self::Asr,
            Self::Asr => Self::Pipeline,
            Self::Pipeline => Self::Sessions,
            Self::Sessions => Self::Error,
            Self::Error => Self::Json,
            Self::Json => Self::Details,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Details => Self::Json,
            Self::Asr => Self::Details,
            Self::Pipeline => Self::Asr,
            Self::Sessions => Self::Pipeline,
            Self::Error => Self::Sessions,
            Self::Json => Self::Error,
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
    pub words: u32,
    pub segments: Vec<String>,
    pub partial: String,
    pub pipeline: Vec<String>,
    pub session_meta: Option<SessionMeta>,
    pub session_phase: Option<SessionPhase>,
    pub meters: Vec<AudioMeter>,
    pub history: Vec<HistoryRecord>,
    pub selected_history: usize,
    pub history_detail: HistoryDetail,
    pub search: String,
    pub searching: bool,
    pub status: String,
    pub config_path: String,
    pub settings_rows: Vec<settings::SettingsRow>,
    pub meter_width: usize,
    pub audio_cache: HashMap<String, audio::AudioInfo>,
    pub confirm: Option<Confirm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirm {
    DeleteAudio { record_id: String },
}

impl App {
    fn new() -> Self {
        let config_path = crate::config::default_path();
        let settings_rows = settings::load_rows();
        Self {
            page: Page::Status,
            state: WireState::Idle,
            recording_id: None,
            started_at: None,
            app: None,
            app_name: None,
            dur_ms: 0,
            words: 0,
            segments: Vec::new(),
            partial: String::new(),
            pipeline: Vec::new(),
            session_meta: None,
            session_phase: None,
            meters: Vec::new(),
            history: Vec::new(),
            selected_history: 0,
            history_detail: HistoryDetail::Details,
            search: String::new(),
            searching: false,
            status: "connected".to_string(),
            config_path: config_path.display().to_string(),
            settings_rows,
            meter_width: 160,
            audio_cache: HashMap::new(),
            confirm: None,
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
                    record.asr.text.as_str(),
                    &record.text,
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
                    self.session_meta = None;
                    self.session_phase = None;
                    self.meters.clear();
                    self.app = None;
                    self.app_name = None;
                    self.dur_ms = 0;
                    self.words = 0;
                }
            }
            Event::AppChanged { app, app_name } => {
                self.app = app;
                self.app_name = app_name;
            }
            Event::StatsChanged { dur_ms, words } => {
                self.dur_ms = dur_ms;
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
            Event::AudioMeter { meter, .. } => {
                if self.page == Page::Status {
                    self.meters.push(meter);
                    trim_meters_to_capacity(&mut self.meters);
                }
            }
            Event::SessionMeta { meta, .. } => {
                self.session_meta = Some(meta);
            }
            Event::SessionPhase { phase, .. } => {
                self.session_phase = Some(phase);
            }
            Event::HistoryAppended { record } => {
                self.refresh_audio_cache_for_record(&record);
                self.history.insert(0, *record);
                self.selected_history = self
                    .selected_history
                    .min(self.history.len().saturating_sub(1));
            }
            Event::History { records } => {
                self.history = records;
                self.refresh_audio_cache_for_history();
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

    fn refresh_audio_cache_for_history(&mut self) {
        self.audio_cache.clear();
        let records = self.history.clone();
        for record in &records {
            self.refresh_audio_cache_for_record(record);
        }
    }

    fn refresh_audio_cache_for_record(&mut self, record: &HistoryRecord) {
        self.audio_cache
            .insert(record.id.clone(), audio::audio_info_for_record(record));
    }

    pub fn audio_info_for_record(&self, record: &HistoryRecord) -> audio::AudioInfo {
        self.audio_cache
            .get(&record.id)
            .cloned()
            .unwrap_or_else(|| audio::missing_audio_info_for_record(record))
    }
}

const MAX_METER_HISTORY: usize = 1024;

fn trim_meters_to_capacity(meters: &mut Vec<crate::state::AudioMeter>) {
    if meters.len() > MAX_METER_HISTORY {
        meters.drain(..meters.len() - MAX_METER_HISTORY);
    }
}

fn meter_capacity_for_terminal_width(width: u16) -> usize {
    (width.saturating_sub(11).max(16) as usize).min(MAX_METER_HISTORY)
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
    loop {
        app.meter_width = terminal
            .size()
            .map(|area| meter_capacity_for_terminal_width(area.width))
            .unwrap_or(160);
        terminal.draw(|frame| panes::render(frame, &app))?;

        let deadline = tokio::time::Instant::now()
            + Duration::from_millis(crate::voice::meter::METER_INTERVAL_MS);
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                maybe_key = key_rx.recv() => {
                    let Some(key) = maybe_key else { return Ok(()); };
                    if handle_key(&mut app, key)? {
                        return Ok(());
                    }
                }
                event = client.recv() => {
                    match event.context("read IPC event")? {
                        Some(e) => app.apply_event(e),
                        None => return Ok(()),
                    }
                }
            }
        }
    }
}

fn handle_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    use keybindings::Action;
    if handle_confirm_key(app, key)? {
        return Ok(false);
    }
    match keybindings::action_for(key, app.searching) {
        Action::Quit => return Ok(true),
        Action::NextPage => {
            app.page = match app.page {
                Page::Status => Page::History,
                Page::History => Page::Settings,
                Page::Settings => Page::Status,
            };
            on_page_changed(app);
        }
        Action::PrevPage => {
            app.page = match app.page {
                Page::Status => Page::Settings,
                Page::History => Page::Status,
                Page::Settings => Page::History,
            };
            on_page_changed(app);
        }
        Action::SetPage(page) => {
            if app.page != page {
                app.page = page;
                on_page_changed(app);
            }
        }
        Action::MoveDown => {
            let len = app.filtered_history().len();
            if len > 0 {
                app.selected_history = (app.selected_history + 1).min(len - 1);
            }
        }
        Action::MoveUp => {
            app.selected_history = app.selected_history.saturating_sub(1);
        }
        Action::MoveTop => {
            app.selected_history = 0;
        }
        Action::MoveBottom => {
            let len = app.filtered_history().len();
            app.selected_history = len.saturating_sub(1);
        }
        Action::NextDetail => {
            app.history_detail = app.history_detail.next();
        }
        Action::PrevDetail => {
            app.history_detail = app.history_detail.prev();
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
            app.confirm = None;
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
                    .map(|record| record.text.clone());
                if let Some(text) = text {
                    crate::clipboard_darwin::write_string(&text)?;
                    app.status = "copied selected history text".to_string();
                }
            }
        }
        Action::CopySelectedRaw => {
            if app.page == Page::History {
                let text = app
                    .filtered_history()
                    .get(app.selected_history)
                    .map(|record| record.asr.text.clone());
                if let Some(text) = text {
                    crate::clipboard_darwin::write_string(&text)?;
                    app.status = "copied selected ASR text".to_string();
                }
            }
        }
        Action::OpenAudio => run_audio_action(app, audio::open_audio, "tui.history.audio.opening"),
        Action::RevealAudio => {
            run_audio_action(app, audio::reveal_audio, "tui.history.audio.revealing")
        }
        Action::DeleteAudio => {
            if app.page == Page::History {
                if let Some(record_id) =
                    selected_history_record(app).map(|record| record.id.clone())
                {
                    let info = selected_history_record(app)
                        .map(|record| app.audio_info_for_record(record))
                        .expect("selected record exists");
                    if info.exists() {
                        app.confirm = Some(Confirm::DeleteAudio { record_id });
                        app.status = crate::t!("tui.confirm.delete_audio");
                    } else {
                        app.status = crate::t!("tui.history.audio.missing_status");
                    }
                }
            }
        }
        Action::None => {}
    }
    Ok(false)
}

fn on_page_changed(app: &mut App) {
    if app.page == Page::Status {
        app.meters.clear();
    }
}

fn handle_confirm_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    use crossterm::event::{KeyCode, KeyEventKind};
    if key.kind != KeyEventKind::Press || app.confirm.is_none() {
        return Ok(false);
    }
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            confirm_yes(app)?;
            Ok(true)
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            app.confirm = None;
            app.status = crate::t!("tui.confirm.cancelled");
            Ok(true)
        }
        _ => Ok(true),
    }
}

fn confirm_yes(app: &mut App) -> Result<()> {
    let Some(confirm) = app.confirm.take() else {
        return Ok(());
    };
    match confirm {
        Confirm::DeleteAudio { record_id } => {
            let Some(record) = app.history.iter().find(|record| record.id == record_id) else {
                app.status = crate::t!("tui.history.audio.record_missing");
                return Ok(());
            };
            let path = audio::audio_path_for_record(record);
            match audio::delete_audio_path(&path)? {
                audio::DeleteAudioResult::Deleted => {
                    let info = audio::missing_audio_info_for_record(record);
                    app.audio_cache.insert(record.id.clone(), info);
                    app.status = crate::t!("tui.history.audio.deleted", path = path.display());
                }
                audio::DeleteAudioResult::Missing => {
                    let info = audio::missing_audio_info_for_record(record);
                    app.audio_cache.insert(record.id.clone(), info);
                    app.status = crate::t!("tui.history.audio.missing_status");
                }
            }
        }
    }
    Ok(())
}

fn run_audio_action(app: &mut App, action: fn(&std::path::Path) -> Result<()>, status_key: &str) {
    if app.page != Page::History {
        return;
    }
    let Some(record) = selected_history_record(app) else {
        app.status = crate::t!("tui.no_history_selected");
        return;
    };
    let info = app.audio_info_for_record(record);
    if !info.exists() {
        app.status = crate::t!("tui.history.audio.missing_status");
        return;
    }
    match action(&info.path) {
        Ok(()) => {
            app.status = crate::i18n::tr(status_key, &[("path", info.path.display().to_string())])
        }
        Err(e) => app.status = crate::t!("tui.error.audio_action", error = e),
    }
}

fn selected_history_record(app: &App) -> Option<&HistoryRecord> {
    app.filtered_history().get(app.selected_history).copied()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn meter(peak: f32) -> AudioMeter {
        AudioMeter {
            rms: peak,
            peak,
            clipped: false,
            vad_probability: None,
            vad_speech: None,
        }
    }

    #[test]
    fn trim_meters_to_capacity_keeps_large_tail() {
        let mut meters = (0..1100).map(|idx| meter(idx as f32)).collect::<Vec<_>>();

        trim_meters_to_capacity(&mut meters);

        assert_eq!(meters.len(), MAX_METER_HISTORY);
        assert_eq!(meters.first().unwrap().peak, 76.0);
        assert_eq!(meters.last().unwrap().peak, 1099.0);
    }

    #[test]
    fn meter_capacity_tracks_terminal_width_with_minimum_and_4k_cap() {
        assert_eq!(meter_capacity_for_terminal_width(200), 189);
        assert_eq!(meter_capacity_for_terminal_width(20), 16);
        assert_eq!(meter_capacity_for_terminal_width(3840), MAX_METER_HISTORY);
    }
}
