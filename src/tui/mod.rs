// Adding a new page:
//   1. Create tui/<name>.rs with a `pub struct <Name>Page` plus `impl Page`
//      (see tui/page.rs for the trait).
//   2. Add `pub mod <name>;` below and a field to `App`.
//   3. Add a variant to `Page`, update `Page::index`, and pick a digit
//      shortcut in tui/keybindings.rs (`SetPage` arm).
//   4. Dispatch the new variant in panes::render and in handle_key's
//      `Action::Forward` match.
//   5. Implement `Page::key_hints()` next to the page's `on_key` so its
//      keybindings show up in the footer automatically.

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

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::ipc::client::IpcClient;
use crate::ipc::protocol::{Command, Event, WireState};
use crate::tui::configure::ConfigurePage;
use crate::tui::history::HistoryPage;
use crate::tui::page::Page as _;
use crate::tui::status::StatusPage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Status,
    History,
    Configure,
}

impl Page {
    fn index(self) -> usize {
        match self {
            Page::Status => 0,
            Page::History => 1,
            Page::Configure => 2,
        }
    }
}

#[derive(Debug)]
pub struct App {
    pub page: Page,
    pub status_page: StatusPage,
    pub history: HistoryPage,
    pub status: String,
    /// When the transient footer status should auto-clear back to the resting
    /// state. `None` = a persistent/resting status that never expires.
    pub status_expires_at: Option<Instant>,
    pub theme: crate::config::theme::TuiTheme,
    pub configure: ConfigurePage,
    /// Sender for delivering background task results back to the event loop.
    ui_tx: Option<mpsc::UnboundedSender<UiEvent>>,
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
            status_expires_at: None,
            theme,
            configure: ConfigurePage::new(),
            ui_tx: None,
        }
    }

    /// Set a transient footer status that auto-clears after [`status_ttl`].
    fn set_status(&mut self, msg: String, is_error: bool) {
        self.status = msg;
        self.status_expires_at = Some(Instant::now() + status_ttl(is_error));
    }

    /// Revert the footer to the resting state once a transient status expires.
    fn clear_status(&mut self) {
        self.status = crate::t!("tui.status.connected");
        self.status_expires_at = None;
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
                self.set_status(
                    crate::i18n::tr("tui.status.config_reloaded", &[("path", path.to_string())]),
                    false,
                );
                self.configure.apply_event(&event, true);
                self.theme = crate::config::load_from(&crate::config::default_path())
                    .map(|cfg| {
                        crate::i18n::init(&cfg.ui.language);
                        crate::config::theme::load_effective(&cfg, &crate::config::default_path())
                            .tui
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
                self.set_status(
                    crate::i18n::tr(
                        "tui.error.daemon_event",
                        &[("kind", kind.clone()), ("error", msg.clone())],
                    ),
                    true,
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
    vec![Command::Subscribe]
}

#[derive(Debug)]
enum UiEvent {
    Key(KeyEvent),
    Mouse(crossterm::event::MouseEvent),
    Paste(String),
    /// Terminal resized (e.g. tmux zoom): wake the loop so it repaints. The
    /// idle path has no timer, so without this a resize would not redraw.
    Resize,
    /// Result of a background runtime test (the `t` key).
    TestResult(RuntimeTestKind, Result<(), String>),
    /// Result of fetching provider model ids for the LLM draft (the `m` key).
    ModelListResult(Result<Vec<String>, String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeTestKind {
    Llm,
    Asr,
}

pub async fn run() -> Result<()> {
    init_i18n_from_config();
    let mut client = IpcClient::connect(crate::ipc::server::default_socket_path()).await?;
    for command in startup_commands() {
        client.send(&command).await?;
    }

    let _terminal = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<UiEvent>();
    let event_tx = ui_tx.clone();
    std::thread::spawn(move || loop {
        match event::read() {
            Ok(CrosstermEvent::Key(key)) => {
                if event_tx.send(UiEvent::Key(key)).is_err() {
                    return;
                }
            }
            Ok(CrosstermEvent::Mouse(mouse)) => {
                if event_tx.send(UiEvent::Mouse(mouse)).is_err() {
                    return;
                }
            }
            Ok(CrosstermEvent::Paste(text)) => {
                if event_tx.send(UiEvent::Paste(text)).is_err() {
                    return;
                }
            }
            Ok(CrosstermEvent::Resize(_, _)) => {
                if event_tx.send(UiEvent::Resize).is_err() {
                    return;
                }
            }
            Ok(_) => {}
            Err(_) => return,
        }
    });

    let mut app = App::new();
    app.ui_tx = Some(ui_tx.clone());
    loop {
        terminal.draw(|frame| panes::render(frame, &app))?;

        tokio::select! {
            _ = tick_or_idle(animation_tick(app.status_page.state)) => {}
            _ = status_expiry(app.status_expires_at) => { app.clear_status(); }
            maybe_event = ui_rx.recv() => {
                let Some(event) = maybe_event else { return Ok(()); };
                if handle_ui_event(&mut app, &mut client, event).await? {
                    return Ok(());
                }
            }
            event = client.recv() => {
                match event.context("read IPC event")? {
                    Some(e) => {
                        app.apply_event(e);
                        // Coalesce: a VAD transition emits a burst of events at
                        // once; drain everything already buffered so the frame is
                        // drawn once, not once per event (which caused stutter).
                        loop {
                            tokio::select! {
                                biased;
                                more = client.recv() => match more.context("read IPC event")? {
                                    Some(e) => app.apply_event(e),
                                    None => return Ok(()),
                                },
                                _ = std::future::ready(()) => break,
                            }
                        }
                        send_pending_history_refresh(&mut app, &mut client).await?;
                    }
                    None => return Ok(()),
                }
            }
        }
    }
}

/// Periodic redraw interval while capturing, or `None` when idle so the UI
/// is purely event-driven and never wakes the CPU while nothing changes.
fn animation_tick(state: WireState) -> Option<Duration> {
    matches!(state, WireState::Recording | WireState::Stopping)
        .then(|| Duration::from_millis(crate::voice::meter::METER_INTERVAL_MS))
}

/// Resolve after `interval` when animating; never resolve when idle.
async fn tick_or_idle(interval: Option<Duration>) {
    match interval {
        Some(interval) => tokio::time::sleep(interval).await,
        None => std::future::pending::<()>().await,
    }
}

/// How long a transient footer status stays before auto-clearing. Errors
/// linger longer than routine info so there is time to read them.
fn status_ttl(is_error: bool) -> Duration {
    if is_error {
        Duration::from_secs(10)
    } else {
        Duration::from_secs(4)
    }
}

/// Resolve when the current transient status should be cleared; never resolve
/// when the status is persistent (`None`) so idle CPU stays at zero.
async fn status_expiry(expires_at: Option<Instant>) {
    match expires_at {
        Some(at) => tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await,
        None => std::future::pending::<()>().await,
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

    // 顺序关键：编辑某个 draft 字段时 is_editing() 与 draft_active() 同时为真，
    // 须让编辑器先拿到键；编辑结束后再回到 draft 导航。
    if app.page == Page::Configure && app.configure.is_editing() {
        let outcome = if app.configure.modal.is_some() {
            app.configure.feed_modal_key(key)
        } else {
            app.configure.feed_edit_key(key)
        };
        if outcome.reload_config {
            client.send(&Command::ReloadConfig).await?;
        }
        if let Some(status) = outcome.status {
            app.set_status(status, false);
        }
        return Ok(false);
    }
    if app.page == Page::Configure && app.configure.draft_active() {
        // `m` fetches model ids from providers that expose a Models API.
        if key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('m')
            && key.modifiers.is_empty()
            && app.configure.draft_supports_test()
        {
            if let (Some(cfg), Some(tx)) = (app.configure.draft_test_config(), app.ui_tx.clone()) {
                app.set_status(crate::t!("tui.configure.llm_create.models_fetching"), false);
                tokio::spawn(async move {
                    let _ = tx.send(UiEvent::ModelListResult(fetch_models(cfg).await));
                });
            }
            return Ok(false);
        }
        // `t` 触发后台连通性测试（非阻塞：结果经 UiEvent::TestResult 回到事件循环）。
        if key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('t')
            && key.modifiers.is_empty()
            && app.configure.draft_supports_test()
        {
            if let (Some(cfg), Some(tx)) = (app.configure.draft_test_config(), app.ui_tx.clone()) {
                app.configure.set_draft_testing();
                tokio::spawn(async move {
                    let _ = tx.send(UiEvent::TestResult(
                        RuntimeTestKind::Llm,
                        run_connectivity_test(cfg).await,
                    ));
                });
            }
            return Ok(false);
        }
        // ^S 保存：成功后自动跑一次连通性测试（先抓当前配置，因为保存会清空 draft）。
        let saving = key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('s')
            && key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL);
        let saved_cfg = if saving {
            app.configure.draft_test_config()
        } else {
            None
        };
        let outcome = app.configure.feed_draft_key(key);
        if outcome.reload_config {
            client.send(&Command::ReloadConfig).await?;
        }
        if let Some(status) = outcome.status {
            app.set_status(status, false);
        }
        // draft 已清空 = 保存成功。
        if saving && !app.configure.draft_active() {
            if let Some(cfg) = saved_cfg {
                spawn_connectivity_test(app, cfg);
            }
        }
        return Ok(false);
    }
    // 普通模式：选中一个已存在的 llm 组件时，`t` 测试它的连通性。
    if app.page == Page::Configure
        && !app.configure.is_editing()
        && key.kind == KeyEventKind::Press
        && key.code == KeyCode::Char('t')
        && key.modifiers.is_empty()
    {
        if let Some(id) = app.configure.selected_llm_component_id() {
            if let Some(tx) = app.ui_tx.clone() {
                app.set_status(crate::t!("tui.configure.llm_create.test_testing"), false);
                tokio::spawn(async move {
                    let _ = tx.send(UiEvent::TestResult(
                        RuntimeTestKind::Llm,
                        test_saved_component(id).await,
                    ));
                });
            }
            return Ok(false);
        }
        if let Some(id) = app.configure.selected_asr_instance_id() {
            if let Some(tx) = app.ui_tx.clone() {
                app.set_status(crate::t!("tui.configure.asr_create.test_testing"), false);
                tokio::spawn(async move {
                    let _ = tx.send(UiEvent::TestResult(
                        RuntimeTestKind::Asr,
                        test_saved_asr_instance(id).await,
                    ));
                });
            }
            return Ok(false);
        }
    }
    if app.page == Page::History {
        if let Some(outcome) = app.history.feed_confirm_key(key) {
            if let Some(cmd) = outcome.command {
                client.send(&cmd).await?;
            }
            if let Some(status) = outcome.status {
                app.set_status(status, false);
            }
            return Ok(false);
        }
    }
    match keybindings::action_for(key, app.history.searching, app.page) {
        Action::Quit => return Ok(true),
        Action::NextPage => {
            app.page = match app.page {
                Page::Status => Page::History,
                Page::History => Page::Configure,
                Page::Configure => Page::Status,
            };
            on_page_changed(app);
            if app.page == Page::History {
                send_history_enter_commands(app, client).await?;
                send_pending_history_refresh(app, client).await?;
            }
        }
        Action::PrevPage => {
            app.page = match app.page {
                Page::Status => Page::Configure,
                Page::History => Page::Status,
                Page::Configure => Page::History,
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
            // Gated to the History page in `action_for`, so we are already here.
            app.history.start_search();
            send_history_enter_commands(app, client).await?;
        }
        Action::Forward(key) => {
            let outcome = match app.page {
                Page::Status => app.status_page.on_key(key),
                Page::History => app.history.on_key(key),
                Page::Configure => app.configure.on_key(key),
            };
            if let Some(cmd) = outcome.command {
                client.send(&cmd).await?;
            }
            if let Some(status) = outcome.status {
                app.set_status(status, false);
            }
        }
        Action::None => {}
    }
    Ok(false)
}

/// Run one LLM connectivity check with a timeout. Reuses the same runtime check
/// as `shuo doctor`; a success means base_url + api_key + model all work.
async fn run_connectivity_test(cfg: crate::post::llm::LlmCleanupConfig) -> Result<(), String> {
    let checker = crate::post::llm::LlmCleanup::new(cfg);
    match tokio::time::timeout(Duration::from_secs(15), checker.check_runtime()).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(crate::t!("tui.configure.llm_create.test_timeout")),
    }
}

async fn fetch_models(cfg: crate::post::llm::LlmCleanupConfig) -> Result<Vec<String>, String> {
    let checker = crate::post::llm::LlmCleanup::new(cfg);
    match tokio::time::timeout(Duration::from_secs(15), checker.list_models()).await {
        Ok(Ok(models)) => Ok(models),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(crate::t!("tui.configure.llm_create.models_timeout")),
    }
}

/// Build the test config for an already-saved llm component (bare id) the same
/// way `shuo doctor` does, then run the connectivity check.
async fn test_saved_component(id: String) -> Result<(), String> {
    let dir = crate::config::post::PostDir {
        dir: crate::config::post::default_dir(),
    };
    let cfg = crate::config::post::load_llm_config(&id, &dir, &toml::value::Table::new())
        .and_then(crate::post::build_llm_cleanup_config)
        .map_err(|e| format!("{e:#}"))?;
    run_connectivity_test(cfg).await
}

async fn test_saved_asr_instance(id: String) -> Result<(), String> {
    use crate::config::asr::instance::AsrKind;
    let ctx = crate::asr::types::SessionCtx {
        language: crate::asr::types::LanguageMode::Multilingual {
            hint: vec!["zh-CN".to_string(), "en-US".to_string()],
        },
        hotwords: Vec::new(),
    };
    let instance =
        crate::config::asr::instance::resolve_instance(&id).map_err(|e| format!("{e:#}"))?;
    match instance.kind {
        AsrKind::Doubao => {
            let provider =
                crate::asr::providers::doubao::DoubaoProvider::new_from_path_with_overrides(
                    &instance.path,
                    None,
                )
                .map_err(|e| format!("{e:#}"))?;
            match tokio::time::timeout(Duration::from_secs(15), provider.check_runtime(ctx)).await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e.to_string()),
                Err(_) => Err(crate::t!("tui.configure.asr_create.test_timeout")),
            }
        }
        AsrKind::Tencent => {
            let provider =
                crate::asr::providers::tencent::TencentProvider::new_from_path_with_overrides(
                    &instance.path,
                    None,
                )
                .map_err(|e| format!("{e:#}"))?;
            match tokio::time::timeout(Duration::from_secs(15), provider.check_runtime(ctx)).await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e.to_string()),
                Err(_) => Err(crate::t!("tui.configure.asr_create.test_timeout")),
            }
        }
        AsrKind::Apple => check_apple_asr_instance(instance.path, ctx).await,
    }
}

#[cfg(target_os = "macos")]
async fn check_apple_asr_instance(
    path: std::path::PathBuf,
    ctx: crate::asr::types::SessionCtx,
) -> Result<(), String> {
    let provider =
        crate::asr::providers::apple::AppleProvider::new_from_path_with_overrides(&path, None)
            .map_err(|e| format!("{e:#}"))?;
    match tokio::time::timeout(Duration::from_secs(15), provider.check_runtime(ctx)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(crate::t!("tui.configure.asr_create.test_timeout")),
    }
}

#[cfg(not(target_os = "macos"))]
async fn check_apple_asr_instance(
    _path: std::path::PathBuf,
    _ctx: crate::asr::types::SessionCtx,
) -> Result<(), String> {
    Err("Apple ASR provider is only implemented on macOS".to_string())
}

/// Kick off a connectivity test in the background; the result comes back as
/// `UiEvent::TestResult`. Sets a "testing…" footer status right away.
fn spawn_connectivity_test(app: &mut App, cfg: crate::post::llm::LlmCleanupConfig) {
    if let Some(tx) = app.ui_tx.clone() {
        app.set_status(crate::t!("tui.configure.llm_create.test_testing"), false);
        tokio::spawn(async move {
            let _ = tx.send(UiEvent::TestResult(
                RuntimeTestKind::Llm,
                run_connectivity_test(cfg).await,
            ));
        });
    }
}

async fn handle_ui_event(app: &mut App, client: &mut IpcClient, event: UiEvent) -> Result<bool> {
    match event {
        UiEvent::Key(key) => handle_key(app, client, key).await,
        // The outer loop redraws on every wake, so a resize just needs to wake it.
        UiEvent::Resize => Ok(false),
        UiEvent::ModelListResult(result) => {
            let (msg, is_error) = match result {
                Ok(models) => {
                    let count = models.len();
                    if app.configure.draft_active() {
                        app.configure.set_draft_model_options(models);
                    }
                    (
                        crate::i18n::tr(
                            "tui.configure.llm_create.models_ok",
                            &[("count", count.to_string())],
                        ),
                        false,
                    )
                }
                Err(error) => (
                    crate::i18n::tr(
                        "tui.configure.llm_create.models_failed",
                        &[("error", error)],
                    ),
                    true,
                ),
            };
            app.set_status(msg, is_error);
            Ok(false)
        }
        UiEvent::TestResult(kind, result) => {
            // draft 里在 detail 面板显示；普通模式/保存后走底部状态栏。
            if app.configure.draft_active() {
                app.configure.set_draft_test_result(result);
            } else {
                let (ok_key, failed_key) = match kind {
                    RuntimeTestKind::Llm => (
                        "tui.configure.llm_create.test_ok",
                        "tui.configure.llm_create.test_failed",
                    ),
                    RuntimeTestKind::Asr => (
                        "tui.configure.asr_create.test_ok",
                        "tui.configure.asr_create.test_failed",
                    ),
                };
                let (msg, is_error) = match &result {
                    Ok(()) => (crate::i18n::tr(ok_key, &[]), false),
                    Err(e) => (crate::i18n::tr(failed_key, &[("error", e.clone())]), true),
                };
                app.set_status(msg, is_error);
            }
            Ok(false)
        }
        UiEvent::Paste(text) => {
            if app.page == Page::Configure && app.configure.modal.is_some() {
                let outcome = app.configure.feed_modal_paste(&text);
                if outcome.reload_config {
                    client.send(&Command::ReloadConfig).await?;
                }
                if let Some(status) = outcome.status {
                    app.set_status(status, false);
                }
            } else if app.page == Page::Configure && app.configure.editing.is_some() {
                let outcome = app.configure.feed_edit_paste(&text);
                if outcome.reload_config {
                    client.send(&Command::ReloadConfig).await?;
                }
                if let Some(status) = outcome.status {
                    app.set_status(status, false);
                }
            }
            Ok(false)
        }
        UiEvent::Mouse(mouse) => {
            // Global: clicking the tab bar switches pages from any page.
            if let MouseEventKind::Down(_) = mouse.kind {
                if let Some(page) = panes::tab_at(mouse.column, mouse.row) {
                    if app.page != page {
                        app.page = page;
                        on_page_changed(app);
                        if app.page == Page::History {
                            send_history_enter_commands(app, client).await?;
                            send_pending_history_refresh(app, client).await?;
                        }
                    }
                    return Ok(false);
                }
            }
            let kind = match mouse.kind {
                MouseEventKind::Down(_) => Some(crate::tui::page::MouseKind::Down),
                MouseEventKind::ScrollDown => Some(crate::tui::page::MouseKind::ScrollDown),
                MouseEventKind::ScrollUp => Some(crate::tui::page::MouseKind::ScrollUp),
                _ => None,
            };
            if let Some(kind) = kind {
                let outcome = match app.page {
                    Page::Configure => app.configure.on_mouse(mouse.column, mouse.row, kind),
                    Page::History => app.history.on_mouse(mouse.column, mouse.row, kind),
                    Page::Status => crate::tui::page::KeyOutcome::none(),
                };
                if let Some(cmd) = outcome.command {
                    client.send(&cmd).await?;
                }
                if let Some(status) = outcome.status {
                    app.set_status(status, false);
                }
            }
            Ok(false)
        }
    }
}

async fn send_pending_history_refresh(app: &mut App, client: &mut IpcClient) -> Result<()> {
    if app.page != Page::History {
        return Ok(());
    }
    for command in app.history.refresh_commands() {
        client.send(&command).await?;
    }
    Ok(())
}

async fn send_history_enter_commands(app: &mut App, client: &mut IpcClient) -> Result<()> {
    for command in app.history.enter_commands() {
        client.send(&command).await?;
    }
    Ok(())
}

fn on_page_changed(app: &mut App) {
    if app.page == Page::Status {
        app.status_page.on_enter();
    }
    if app.page == Page::Configure {
        app.configure.on_enter();
    }
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(
            std::io::stdout(),
            EnterAlternateScreen,
            // Disable line wrap (DECAWM): without it, printing a wide (CJK) glyph
            // in the last column makes the terminal wrap+scroll the whole screen,
            // leaving residue. ratatui positions every cell explicitly, so wrap
            // is never wanted here.
            DisableLineWrap,
            EnableMouseCapture,
            EnableBracketedPaste
        )?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            std::io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            EnableLineWrap
        );
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use crate::ipc::protocol::{Command, Event, WireState};

    #[test]
    fn animation_tick_only_runs_while_capturing() {
        // Idle / Error must not schedule a periodic redraw (zero idle CPU wakeups).
        assert_eq!(super::animation_tick(WireState::Idle), None);
        assert_eq!(super::animation_tick(WireState::Error), None);
        // Recording / Stopping animate the duration counter + meters.
        assert!(super::animation_tick(WireState::Recording).is_some());
        assert!(super::animation_tick(WireState::Stopping).is_some());
    }

    #[test]
    fn error_status_lingers_longer_than_info() {
        assert!(super::status_ttl(true) > super::status_ttl(false));
    }

    #[test]
    fn transient_status_records_expiry_and_clears_to_idle() {
        let mut app = super::App::new();
        // Resting startup status carries no expiry — it must never auto-clear.
        assert!(app.status_expires_at.is_none());

        app.set_status("copied".to_string(), false);
        assert_eq!(app.status, "copied");
        assert!(app.status_expires_at.is_some());

        app.clear_status();
        assert!(app.status_expires_at.is_none());
        assert_ne!(app.status, "copied");
    }

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
        let old = std::env::var("XDG_CONFIG_HOME").ok();
        std::env::set_var("XDG_CONFIG_HOME", &config_home);

        super::init_i18n_from_config();

        assert_eq!(crate::i18n::tr("tui.tab_configure", &[]), "3 配置");
        match old {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
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
        let old = std::env::var("XDG_CONFIG_HOME").ok();
        std::env::set_var("XDG_CONFIG_HOME", &config_home);
        crate::i18n::init("en-US");
        let mut app = super::App::new();

        app.apply_event(crate::ipc::protocol::Event::ConfigReloaded {
            path: root.join("config.toml").display().to_string(),
        });

        assert_eq!(crate::i18n::tr("tui.tab_configure", &[]), "3 配置");
        match old {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        let _ = std::fs::remove_dir_all(home);
    }
}
