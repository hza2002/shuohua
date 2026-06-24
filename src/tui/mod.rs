// Adding a new page:
//   1. Create tui/<name>.rs with a `pub struct <Name>Page` plus `impl Page`
//      (see tui/page.rs for the trait).
//   2. Add `pub mod <name>;` below and a field to `App`.
//   3. Add a variant to `Page`, update `Page::index`, and pick a digit
//      shortcut in tui/keybindings.rs (`SetPage` arm).
//   4. Dispatch the new variant in panes::render and in handle_key's
//      `Action::Forward` match.
//   5. Add a footer hint key under `tui.footer_<name>` in i18n strings.

pub mod audio;
pub mod config_actions;
pub mod configure;
pub mod history;
pub mod keybindings;
pub mod page;
pub mod panes;
pub mod settings;
pub mod status;
pub mod ui;

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

use crate::client_api::DaemonClient;
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
            status: crate::t!("tui.status.connected"),
            theme,
            configure: ConfigurePage::new(),
        }
    }

    fn apply_event(&mut self, event: Event) {
        match event {
            Event::HistoryAppended { .. }
            | Event::HistoryChanged
            | Event::History { .. }
            | Event::HistoryStats { .. }
            | Event::HistoryAnalytics { .. }
            | Event::AudioDeleted { .. }
            | Event::HistoryDeleted { .. } => {
                self.history.apply_event(&event, self.page == Page::History);
            }
            Event::DaemonStatus { .. } => {}
            Event::ConfigReloaded { ref path } => {
                self.status =
                    crate::i18n::tr("tui.status.config_reloaded", &[("path", path.to_string())]);
                self.configure.apply_event(&event, true);
                let config_path = std::path::Path::new(path);
                self.theme = crate::config::load_from(config_path)
                    .map(|cfg| {
                        crate::i18n::init(&cfg.ui.language);
                        crate::config::theme::load_effective(&cfg, config_path).tui
                    })
                    .unwrap_or_default();
            }
            Event::Error {
                ref kind, ref msg, ..
            } => {
                if matches!(
                    kind.as_str(),
                    "history_read" | "history_stats" | "history_analytics"
                ) {
                    self.history.apply_event(&event, self.page == Page::History);
                }
                self.status = crate::i18n::tr(
                    "tui.error.daemon_event",
                    &[("kind", kind.clone()), ("error", msg.clone())],
                );
            }
            _ => {
                self.status_page
                    .apply_event(&event, self.page == Page::Status);
            }
        }
    }
}

fn startup_commands() -> Vec<Command> {
    vec![crate::client_api::subscribe_command()]
}

pub async fn run() -> Result<()> {
    init_i18n_from_config();
    let mut client = DaemonClient::connect_default().await?;
    for command in startup_commands() {
        client.send(&command).await?;
    }

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
                        Some(e) => {
                            app.apply_event(e);
                            send_pending_history_refresh(&mut app, &mut client).await?;
                        }
                        None => return Ok(()),
                    }
                }
            }
        }
    }
}

fn init_i18n_from_config() {
    init_i18n_from_config_path(&crate::config::default_path());
}

fn init_i18n_from_config_path(path: &std::path::Path) {
    let language = crate::config::load_from(path)
        .map(|cfg| cfg.ui.language)
        .unwrap_or_else(|_| "auto".to_string());
    crate::i18n::init(&language);
}

async fn handle_key(app: &mut App, client: &mut DaemonClient, key: KeyEvent) -> Result<bool> {
    use keybindings::Action;

    if app.configure.is_wizard_active() {
        if let Some(status) = app.configure.feed_wizard_key(key) {
            app.status = status;
        }
        return Ok(false);
    }
    if app.page == Page::History {
        if let Some(outcome) = app.history.feed_confirm_key(key) {
            if let Some(cmd) = outcome.command {
                client.send(&cmd).await?;
            }
            if let Some(status) = outcome.status {
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
            if app.page == Page::History {
                send_history_enter_commands(app, client).await?;
                send_pending_history_refresh(app, client).await?;
            }
        }
        Action::PrevPage => {
            app.page = match app.page {
                Page::Status => Page::Settings,
                Page::History => Page::Status,
                Page::Settings => Page::History,
            };
            on_page_changed(app);
            if app.page == Page::History {
                send_history_enter_commands(app, client).await?;
                send_pending_history_refresh(app, client).await?;
            }
        }
        Action::SetPage(page) => {
            if app.page != page {
                app.page = page;
                on_page_changed(app);
                if app.page == Page::History {
                    send_history_enter_commands(app, client).await?;
                    send_pending_history_refresh(app, client).await?;
                }
            }
        }
        Action::StartSearch => {
            app.page = Page::History;
            app.history.start_search();
            send_history_enter_commands(app, client).await?;
        }
        Action::Forward(key) => {
            let outcome = match app.page {
                Page::Status => app.status_page.on_key(key),
                Page::History => app.history.on_key(key),
                Page::Settings => app.configure.on_key(key),
            };
            if let Some(cmd) = outcome.command {
                client.send(&cmd).await?;
            }
            if let Some(status) = outcome.status {
                app.status = status;
            }
        }
        Action::None => {}
    }
    Ok(false)
}

async fn send_pending_history_refresh(app: &mut App, client: &mut DaemonClient) -> Result<()> {
    if app.page != Page::History {
        return Ok(());
    }
    for command in app.history.refresh_commands() {
        client.send(&command).await?;
    }
    Ok(())
}

async fn send_history_enter_commands(app: &mut App, client: &mut DaemonClient) -> Result<()> {
    for command in app.history.enter_commands() {
        client.send(&command).await?;
    }
    Ok(())
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
    use crate::ipc::protocol::{Command, Event};

    fn error(kind: &str) -> Event {
        Event::Error {
            recording_id: None,
            kind: kind.to_string(),
            msg: "boom".to_string(),
        }
    }

    #[test]
    fn tui_startup_does_not_request_history() {
        assert_eq!(super::startup_commands(), vec![Command::Subscribe]);
    }

    #[test]
    fn history_errors_reach_history_page_and_unblock_refresh() {
        let mut app = super::App::new();
        app.page = super::Page::History;
        app.history.enter_commands();
        app.apply_event(Event::HistoryChanged);

        app.apply_event(error("history_read"));
        app.apply_event(error("history_stats"));
        app.apply_event(error("history_analytics"));

        assert!(!app.history.refresh_in_flight);
        assert_eq!(app.history.refresh_commands().len(), 3);
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

        super::init_i18n_from_config_path(&root.join("config.toml"));

        assert_eq!(crate::i18n::tr("tui.tab_settings", &[]), "3 配置");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn config_reloaded_updates_running_tui_language() {
        let home =
            std::env::temp_dir().join(format!("shuohua-tui-reload-i18n-{}", ulid::Ulid::new()));
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
        crate::i18n::init("en-US");
        let mut app = super::App::new();

        app.apply_event(crate::ipc::protocol::Event::ConfigReloaded {
            path: root.join("config.toml").display().to_string(),
        });

        assert_eq!(crate::i18n::tr("tui.tab_settings", &[]), "3 配置");
        let _ = std::fs::remove_dir_all(home);
    }
}
