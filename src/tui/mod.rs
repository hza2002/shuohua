pub mod config_actions;
pub mod configure;
pub mod history;
pub mod keybindings;
pub mod page;
pub mod panes;
pub mod settings;
pub mod status;

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
use crate::ipc::protocol::{Command, Event};
use crate::tui::configure::ConfigurePage;
use crate::tui::history::HistoryPage;
use crate::tui::page::Page as _;
use crate::tui::status::StatusPage;

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
    pub status_page: StatusPage,
    pub history: HistoryPage,
    pub status: String,
    pub theme: crate::config::theme::TuiTheme,
    pub configure: ConfigurePage,
}

impl App {
    fn new() -> Self {
        let config_path = crate::config::default_path();
        let theme = crate::config::load_from(&config_path)
            .map(|cfg| crate::config::theme::load_effective(&cfg, &config_path).tui)
            .unwrap_or_default();
        Self {
            page: Page::Status,
            status_page: StatusPage::new(),
            history: HistoryPage::new(),
            status: "connected".to_string(),
            theme,
            configure: ConfigurePage::new(),
        }
    }

    fn apply_event(&mut self, event: Event) {
        match event {
            Event::HistoryAppended { .. } | Event::History { .. } => {
                self.history.apply_event(&event, self.page == Page::History);
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
            _ => {
                self.status_page
                    .apply_event(&event, self.page == Page::Status);
            }
        }
    }
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
        app.status_page.meter_width = terminal
            .size()
            .map(|area| StatusPage::meter_capacity_for_terminal_width(area.width))
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
        app.status_page.on_enter();
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

        super::init_i18n_from_config();

        assert_eq!(crate::i18n::tr("tui.tab_settings", &[]), "3 配置");
        match old {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        let _ = std::fs::remove_dir_all(home);
    }
}
