pub mod config_actions;
pub mod configure;
pub mod history;
pub mod keybindings;
pub mod page;
pub mod panes;
pub mod settings;

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
use crate::state::{AudioMeter, SessionMeta, SessionPhase};
use crate::tui::configure::ConfigurePage;
use crate::tui::history::HistoryPage;
use crate::tui::page::Page as _;

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
    pub words: u32,
    pub segments: Vec<String>,
    pub partial: String,
    pub pipeline: Vec<String>,
    pub session_meta: Option<SessionMeta>,
    pub session_phase: Option<SessionPhase>,
    pub meters: Vec<AudioMeter>,
    pub history: HistoryPage,
    pub status: String,
    pub theme: crate::config::theme::TuiTheme,
    pub configure: ConfigurePage,
    pub meter_width: usize,
}

impl App {
    fn new() -> Self {
        let config_path = crate::config::default_path();
        let theme = crate::config::load_from(&config_path)
            .map(|cfg| crate::config::theme::load_effective(&cfg, &config_path).tui)
            .unwrap_or_default();
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
            history: HistoryPage::new(),
            status: "connected".to_string(),
            theme,
            configure: ConfigurePage::new(),
            meter_width: 160,
        }
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
            ref event @ (Event::HistoryAppended { .. } | Event::History { .. }) => {
                self.history.apply_event(event, self.page == Page::History);
            }
            Event::DaemonStatus { .. } => {}
            Event::ConfigReloaded { ref path } => {
                self.status = format!("config reloaded: {path}");
                self.configure.apply_event(&event, true);
                self.theme = crate::config::load_from(&crate::config::default_path())
                    .map(|cfg| {
                        crate::config::theme::load_effective(&cfg, &crate::config::default_path())
                            .tui
                    })
                    .unwrap_or_default();
            }
            Event::Error { kind, msg, .. } => {
                self.status = format!("{kind}: {msg}");
            }
        }
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
    init_i18n_from_config();
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
                    if handle_key(&mut app, &mut client, key).await? {
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

fn init_i18n_from_config() {
    let language = crate::config::load_from(&crate::config::default_path())
        .map(|cfg| cfg.ui.language)
        .unwrap_or_else(|_| "auto".to_string());
    crate::i18n::init(&language);
}

async fn handle_key(app: &mut App, client: &mut IpcClient, key: KeyEvent) -> Result<bool> {
    use keybindings::Action;

    if app.configure.is_wizard_active() {
        if let Some(status) = app.configure.feed_wizard_key(key) {
            app.status = status;
        }
        return Ok(false);
    }
    if app.page == Page::History {
        if let Some(status) = app.history.feed_confirm_key(key) {
            if !status.is_empty() {
                app.status = status;
            }
            return Ok(false);
        }
    }
    match keybindings::action_for(key, app.history.searching) {
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
            if app.page == Page::Settings {
                app.configure.move_selection(1);
            } else {
                app.history.move_down();
            }
        }
        Action::MoveUp => {
            if app.page == Page::Settings {
                app.configure.move_selection(-1);
            } else {
                app.history.move_up();
            }
        }
        Action::MoveTop => {
            if app.page == Page::Settings {
                app.configure.move_top();
            } else {
                app.history.move_top();
            }
        }
        Action::MoveBottom => {
            if app.page == Page::Settings {
                app.configure.move_bottom();
            } else {
                app.history.move_bottom();
            }
        }
        Action::NextFocus => {
            if app.page == Page::Settings {
                app.configure.move_focus(1);
            } else {
                app.history.next_detail();
            }
        }
        Action::PrevFocus => {
            if app.page == Page::Settings {
                app.configure.move_focus(-1);
            } else {
                app.history.prev_detail();
            }
        }
        Action::StartSearch => {
            app.page = Page::History;
            app.history.start_search();
        }
        Action::CancelSearch => {
            app.history.cancel_search();
        }
        Action::ClearSearch => {
            app.history.clear_search();
        }
        Action::SearchChar(ch) => {
            app.history.search_char(ch);
        }
        Action::Backspace => {
            app.history.search_backspace();
        }
        Action::CopySelected => {
            if app.page == Page::History {
                if let Some(text) = app.history.copy_selected_text() {
                    crate::clipboard_darwin::write_string(&text)?;
                    app.status = "copied selected history text".to_string();
                }
            }
        }
        Action::CopySelectedRaw => {
            if app.page == Page::History {
                if let Some(text) = app.history.copy_selected_asr() {
                    crate::clipboard_darwin::write_string(&text)?;
                    app.status = "copied selected ASR text".to_string();
                }
            }
        }
        Action::OpenAudio => {
            if app.page == Page::Settings {
                app.status = app.configure.open_editor();
            } else if app.page == Page::History {
                app.status = app.history.open_selected_audio();
            }
        }
        Action::RevealAudio => {
            if app.page == Page::Settings {
                app.status = app.configure.reveal_in_finder();
            } else if app.page == Page::History {
                app.status = app.history.reveal_selected_audio();
            }
        }
        Action::DeleteAudio => {
            if app.page == Page::History {
                app.status = app.history.request_delete_audio();
            }
        }
        Action::ValidateConfig => {
            if app.page == Page::Settings {
                app.status = app.configure.validate();
            }
        }
        Action::ReloadConfig => {
            if app.page == Page::Settings {
                let (cmd, status) = app.configure.request_reload();
                client.send(&cmd).await?;
                app.status = status;
            }
        }
        Action::NewConfig => {
            if app.page == Page::Settings
                && app.configure.module == crate::tui::configure::ConfigureModule::PostProcessor
            {
                app.status = app.configure.start_wizard();
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
    if app.page == Page::Settings {
        app.configure.on_enter();
    }
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

    #[test]
    fn init_i18n_from_config_uses_configured_language() {
        let home = std::env::temp_dir().join(format!("shuohua-tui-i18n-{}", ulid::Ulid::new()));
        let config_home = home.join("config");
        let root = config_home.join("shuohua");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("config.toml"),
            r#"
[hotkey]
trigger = "f16"

[ui]
language = "zh-CN"
"#,
        )
        .unwrap();
        let old = std::env::var("XDG_CONFIG_HOME").ok();
        std::env::set_var("XDG_CONFIG_HOME", &config_home);

        init_i18n_from_config();

        assert_eq!(crate::i18n::tr("tui.tab_settings", &[]), "3 配置");
        match old {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        let _ = std::fs::remove_dir_all(home);
    }
}
