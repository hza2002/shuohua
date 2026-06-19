//! shuohua daemon entry.
//!
//!   * tokio multi-thread runtime
//!   * hotkey CGEventTap CFRunLoop 专用 OS 线程 → os_pipe → 桥到 tokio mpsc
//!   * Tracker 纯函数状态机消化 RawEvent → HotkeyEvent；trigger/cancel 可热替换
//!   * F16 第一次按 = 起录音；第二次按 = 发 stop oneshot 让 task 收尾
//!   * Session 起来时从 `cfg_rx.borrow()` 取**最新** voice/asr 配置，做到
//!     "下次录音用新值"。
//!   * 配置热重载靠 `reload` 模块（独立）：watcher 在 `~/.config/shuohua/`
//!     上跑 notify；各 subscriber 自取所需。

mod app_context_darwin;
mod asr;
mod autotype_darwin;
mod cli;
mod clipboard_darwin;
mod config;
mod focused_window_darwin;
mod hotkey;
mod i18n;
mod ipc;
mod log;
mod overlay;
mod post;
mod reload;
mod state;
mod text_stats;
mod tui;
mod voice;

use anyhow::{Context, Result};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hotkey::{Bindings, HotkeyAction, RawEvent, Suppressor, TrackerSet};
use overlay::OverlayHandle;
use state::StateStore;
use voice::finish::SessionParams;
use voice::SessionControl;

fn main() -> Result<()> {
    let args = cli::parse();
    if args.daemon {
        return run_daemon_process();
    }
    if let Some(command) = args.command {
        return cli::run_command(command);
    }
    run_smart_fallback()
}

fn run_smart_fallback() -> Result<()> {
    let socket = ipc::server::default_socket_path();
    if !socket_accepts_connections(&socket) {
        let stderr = smart_fallback_log("smart.stderr.log")?;
        let stdout = smart_fallback_log("smart.stdout.log")?;
        let child = Command::new(std::env::current_exe().context("resolve current exe")?)
            .arg("--daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .context("spawn shuo --daemon")?;
        drop(child);
        wait_for_socket(&socket, Duration::from_secs(2))?;
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create TUI runtime")?;
    rt.block_on(tui::run())
}

fn smart_fallback_log(name: &str) -> Result<std::fs::File> {
    let dir = state::history::state_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("create state dir {}", dir.display()))?;
    let path = dir.join(name);
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))
}

fn socket_accepts_connections(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

fn wait_for_socket(path: &Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if socket_accepts_connections(path) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!("daemon did not accept UDS connections within {:?}", timeout)
}

struct DaemonLock(std::fs::File);

impl DaemonLock {
    fn acquire() -> Result<Self> {
        let dir = state::history::state_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create state dir {}", dir.display()))?;
        let path = dir.join("daemon.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("open daemon lock {}", path.display()))?;
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            anyhow::bail!("another shuo daemon is already starting or running");
        }
        Ok(Self(file))
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        let _ = unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

fn run_daemon_process() -> Result<()> {
    let _lock = DaemonLock::acquire()?;
    let _log_guard = log::init_daemon().context("initialize daemon logger")?;
    let cfg_path = config::default_path();
    let (cfg_rx, reload_handle) =
        reload::watch_with_handle(cfg_path.clone()).context("start config watcher")?;
    let runtime_cfg = cfg_rx.borrow().clone();
    let cfg = &runtime_cfg.config;
    i18n::init(&cfg.ui.language);
    let hotkeys = Bindings::parse(&cfg.hotkey.trigger, &cfg.hotkey.cancel)?;
    let parsed_trigger = hotkeys
        .combo_for(HotkeyAction::ToggleRecord)
        .context("missing toggle-record hotkey binding")?;
    let parsed_cancel = hotkeys
        .combo_for(HotkeyAction::CancelRecord)
        .context("missing cancel-record hotkey binding")?;

    tracing::info!(
        config_path = %cfg_path.display(),
        trigger = %cfg.hotkey.trigger,
        cancel = %cfg.hotkey.cancel,
        parsed_trigger = %parsed_trigger,
        parsed_cancel = %parsed_cancel,
        post_timeout_ms = cfg.post.timeout_ms,
        auto_paste = cfg.voice.auto_paste,
        record_audio = %cfg.voice.record_audio,
        stop_delay_ms = cfg.voice.stop_delay_ms,
        vad_trace = cfg.dev.vad_trace,
        language = %cfg.ui.language,
        "daemon config loaded"
    );
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let state_store = StateStore::new();

    let cfg_rx_for_daemon = cfg_rx.clone();
    let reload_for_daemon = reload_handle.clone();
    let overlay_for_daemon = overlay.clone();
    let state_for_daemon = state_store.clone();
    thread::Builder::new()
        .name("tokio-daemon".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("create tokio runtime");
            if let Err(e) = rt.block_on(run_daemon(
                cfg_rx_for_daemon,
                reload_for_daemon,
                hotkeys,
                overlay_for_daemon,
                state_for_daemon,
            )) {
                tracing::error!(error = ?e, "daemon exited");
                std::process::exit(2);
            }
        })
        .context("spawn tokio daemon thread")?;

    overlay::run(_overlay_rx, runtime_cfg.theme.overlay.clone());
    Ok(())
}

async fn run_daemon(
    cfg_rx: reload::Rx,
    reload_handle: reload::Handle,
    initial_hotkeys: Bindings,
    overlay: OverlayHandle,
    state_store: StateStore,
) -> Result<()> {
    let listener = ipc::server::bind_default().await?;
    let socket_path = ipc::server::default_socket_path();
    tokio::spawn(ipc::server::run(
        listener,
        state_store.clone(),
        ipc::server::ServerControl {
            reload: reload_handle,
            started_at: Instant::now(),
        },
    ));

    // 三个 subscriber，跟主循环解耦。每个都在 reload 模块里实现。
    reload::spawn_overlay(cfg_rx.clone(), overlay.clone());
    reload::spawn_i18n(cfg_rx.clone(), overlay.clone());
    let (hotkey_tx, mut hotkey_rx) = tokio::sync::mpsc::unbounded_channel::<Bindings>();
    reload::spawn_hotkey(cfg_rx.clone(), hotkey_tx);

    let (pipe_reader, pipe_writer) = os_pipe::pipe().context("create hotkey pipe")?;

    // Suppressor is shared between the CGEventTap callback (decides whether to
    // drop the event for the foreground app) and the daemon main loop (updates
    // the trigger code on `[hotkey].trigger` reload). Lock contention is
    // human-rate; std Mutex is fine.
    let initial_trigger = initial_hotkeys
        .combo_for(HotkeyAction::ToggleRecord)
        .context("missing toggle-record hotkey binding")?
        .clone();
    let initial_cancel = initial_hotkeys
        .combo_for(HotkeyAction::CancelRecord)
        .context("missing cancel-record hotkey binding")?
        .clone();
    let mut initial_suppressor = Suppressor::new(initial_trigger);
    initial_suppressor.set_cancel(initial_cancel);
    let suppressor = Arc::new(Mutex::new(initial_suppressor));
    let suppressor_for_tap = suppressor.clone();

    thread::Builder::new()
        .name("hotkey-eventtap".into())
        .spawn(move || {
            if let Err(e) = hotkey::provider_darwin::run(pipe_writer, suppressor_for_tap) {
                tracing::error!(error = ?e, "hotkey event tap exited");
                std::process::exit(2);
            }
        })
        .context("spawn hotkey thread")?;

    let (raw_tx, mut raw_rx) = tokio::sync::mpsc::unbounded_channel::<RawEvent>();
    thread::Builder::new()
        .name("hotkey-pipe-bridge".into())
        .spawn(move || pipe_to_mpsc(pipe_reader, raw_tx))
        .context("spawn hotkey bridge thread")?;

    tracing::info!(
        uds = %socket_path.display(),
        trigger = %cfg_rx.borrow().config.hotkey.trigger,
        "daemon ready"
    );

    let mut hotkey_trackers = TrackerSet::new(&initial_hotkeys);
    struct ActiveSession {
        control: tokio::sync::watch::Sender<SessionControl>,
        join: tokio::task::JoinHandle<()>,
    }

    let mut active: Option<ActiveSession> = None;

    loop {
        tokio::select! {
            Some(new_hotkeys) = hotkey_rx.recv() => {
                // 重 trigger：换 Tracker（清掉 in-flight tap 候选）+ 同步给 CGEventTap
                // callback 里的 Suppressor。CGEventTap 不动——它本来就抓所有键。
                // Suppressor 的 `held` 不清，旧 trigger 已按下的物理键 keyup 仍会被
                // 正确吞掉（§5 不变量 8）。
                hotkey_trackers.set_bindings(&new_hotkeys);
                let new_trigger = new_hotkeys
                    .combo_for(HotkeyAction::ToggleRecord)
                    .context("missing toggle-record hotkey binding")?
                    .clone();
                let new_cancel = new_hotkeys
                    .combo_for(HotkeyAction::CancelRecord)
                    .context("missing cancel-record hotkey binding")?
                    .clone();
                if let Ok(mut s) = suppressor.lock() {
                    s.set_trigger(new_trigger);
                    s.set_cancel(new_cancel);
                }
                continue;
            }
            maybe_raw = raw_rx.recv() => {
                let Some(ev) = maybe_raw else {
                    anyhow::bail!("hotkey bridge channel closed");
                };
                match hotkey_trackers.on_event(ev, Instant::now()) {
                    Some(HotkeyAction::CancelRecord) => {
                    // 先清掉已经结束的 session，避免对死 watch 发 Cancel。
                    if active.as_ref().is_some_and(|session| session.join.is_finished()) {
                        active = None;
                        if let Ok(mut s) = suppressor.lock() {
                            s.set_cancel_active(false);
                        }
                    }
                    if let Some(session) = active.as_ref() {
                        let _ = session.control.send(SessionControl::Cancel);
                    }
                    // ESC 也用来快速关掉 error / 残留 notice 的 lingering overlay。
                    // Dismiss 绕过所有延期，立即 hide。
                    overlay.send(overlay::OverlayCmd::Dismiss);
                    continue;
                    }
                    Some(HotkeyAction::ToggleRecord) => {}
                    None => continue,
                }
                if active.as_ref().is_some_and(|session| session.join.is_finished()) {
                    active = None;
                    if let Ok(mut s) = suppressor.lock() {
                        s.set_cancel_active(false);
                    }
                }
                match active.as_ref() {
                    None => {
                        // 新 session 起来时从 cfg_rx 取最新 voice/profile/post 配置。
                        let runtime_cfg = cfg_rx.borrow().clone();
                        let cfg = &runtime_cfg.config;
                        let start_app_context = post::app_context::frontmost_app();
                        let profile_dir = config::profile::default_dir();
                        let profile = match config::profile::load_for_app(
                            &profile_dir,
                            &cfg.profile,
                            start_app_context.bundle_id.as_deref(),
                        ) {
                            Ok(profile) => profile,
                            Err(e) => {
                                tracing::warn!(error = ?e, "profile load failed");
                                state_store.set_error(None);
                                continue;
                            }
                        };
                        let post_dir = config::post::default_dir();
                        let post_chain = match config::post::load_components(
                            &profile.post.chain,
                            &config::post::PostDirs {
                                rule: post_dir.join("rule"),
                                llm: post_dir.join("llm"),
                            },
                            &profile.post.llm,
                        ) {
                            Ok(chain) => chain,
                            Err(e) => {
                                tracing::warn!(error = ?e, "post chain load failed");
                                state_store.set_error(None);
                                continue;
                            }
                        };
                        let runtime =
                            match build_provider(&profile.asr.provider, &profile.asr.overrides) {
                            Ok(runtime) => runtime,
                            Err(e) => {
                                tracing::error!(error = ?e, "ASR provider init failed");
                                state_store.set_error(None);
                                continue;
                            }
                        };
                        let (control_tx, control_rx) = tokio::sync::watch::channel(SessionControl::Idle);
                        let params = SessionParams {
                            auto_paste: cfg.voice.auto_paste,
                            record_audio: cfg.voice.record_audio,
                            vad_trace: cfg.dev.vad_trace,
                            idle_pause: runtime.idle_pause,
                            finalize_timeout_ms: runtime.finalize_timeout_ms,
                            vad: cfg.voice.vad.clone(),
                            stop_delay_ms: cfg.voice.stop_delay_ms,
                            hotwords: profile.asr.hotwords.clone(),
                            start_app_context,
                            post_chain,
                            post_timeout_ms: cfg.post.timeout_ms,
                            overlay: Some(overlay.clone()),
                            state: state_store.clone(),
                        };
                        let provider = runtime.provider;
                        let suppressor_for_task = suppressor.clone();
                        let join = tokio::spawn(async move {
                            voice::finish::run_recording(provider.as_ref(), params, control_rx).await;
                            if let Ok(mut s) = suppressor_for_task.lock() {
                                s.set_cancel_active(false);
                            }
                        });
                        active = Some(ActiveSession { control: control_tx, join });
                        if let Ok(mut s) = suppressor.lock() {
                            s.set_cancel_active(true);
                        }
                    }
                    Some(session) => {
                        let _ = session.control.send(SessionControl::Stop);
                    }
                }
            }
        }
    }
}

struct ProviderRuntime {
    provider: Arc<dyn asr::AsrProvider>,
    idle_pause: bool,
    finalize_timeout_ms: u64,
}

fn build_provider(name: &str, overrides: &toml::value::Table) -> Result<ProviderRuntime> {
    match name {
        "doubao" => {
            let p = asr::providers::doubao::DoubaoProvider::new_with_overrides(Some(overrides))
                .context("init doubao provider")?;
            let idle_pause = p.idle_pause();
            let finalize_timeout_ms = p.finalize_timeout_ms();
            Ok(ProviderRuntime {
                provider: Arc::new(p),
                idle_pause,
                finalize_timeout_ms,
            })
        }
        "apple" => {
            let p = asr::providers::apple::AppleProvider::new_with_overrides(Some(overrides))
                .context("init apple provider")?;
            let idle_pause = p.idle_pause();
            let finalize_timeout_ms = p.finalize_timeout_ms();
            Ok(ProviderRuntime {
                provider: Arc::new(p),
                idle_pause,
                finalize_timeout_ms,
            })
        }
        other => anyhow::bail!("未知 ASR provider {other:?}。支持 \"doubao\" / \"apple\""),
    }
}

fn pipe_to_mpsc(mut reader: os_pipe::PipeReader, tx: tokio::sync::mpsc::UnboundedSender<RawEvent>) {
    let mut buf = [0u8; 4];
    loop {
        if let Err(e) = reader.read_exact(&mut buf) {
            tracing::error!(error = %e, "hotkey pipe read failed");
            return;
        }
        // Unknown kind byte = corrupt frame; skip it rather than crash.
        let Some(ev) = RawEvent::decode(buf) else {
            tracing::warn!(frame = ?buf, "dropped unknown hotkey frame");
            continue;
        };
        if tx.send(ev).is_err() {
            return;
        }
    }
}
