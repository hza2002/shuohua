//! shuohua daemon entry.
//!
//! M3.f status: F16 toggle → record → DoubaoProvider 流式 → 剪贴板 → Cmd+V，
//! 加 AppKit overlay + 完整 M5 配置热重载（reload 机制）。
//!
//!   * tokio multi-thread runtime
//!   * hotkey CGEventTap CFRunLoop 专用 OS 线程 → os_pipe → 桥到 tokio mpsc
//!   * Tracker (M1 纯函数状态机) 消化 RawKey → HotkeyEvent；trigger 可热替换
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

use hotkey::{HotkeyEvent, RawKey, Suppressor, Tracker};
use overlay::OverlayHandle;
use state::StateStore;
use voice::finish::SessionParams;
use voice::SessionControl;

const KEY_ESCAPE: u16 = 0x35;

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
    let cfg_path = config::default_path();
    let (cfg_rx, reload_handle) =
        reload::watch_with_handle(cfg_path.clone()).context("start config watcher")?;
    let cfg: Arc<config::Config> = cfg_rx.borrow().clone();
    i18n::init(&cfg.ui.language);
    let trigger_code = hotkey::parse::parse(&cfg.hotkey.trigger)
        .with_context(|| format!("parse [hotkey] trigger = {:?}", cfg.hotkey.trigger))?;

    let provider = build_provider(&cfg.asr.provider)?;

    eprintln!(
        "[shuo] config {} loaded:\n         trigger={} (code=0x{:02X})\n         \
         asr.provider={} (caps multilingual={})\n         voice.auto_paste={}  \
         voice.record_audio={}  voice.stop_delay_ms={}  ui.language={}",
        cfg_path.display(),
        cfg.hotkey.trigger,
        trigger_code,
        provider.name(),
        provider.caps().multilingual,
        cfg.voice.auto_paste,
        cfg.voice.record_audio,
        cfg.voice.stop_delay_ms,
        cfg.ui.language,
    );
    eprintln!("[shuo] {} hotwords loaded", cfg.asr.hotwords.len());
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
                trigger_code,
                overlay_for_daemon,
                state_for_daemon,
            )) {
                eprintln!("[shuo] daemon exited: {e:#}");
                std::process::exit(2);
            }
        })
        .context("spawn tokio daemon thread")?;

    overlay::view::run(_overlay_rx, cfg.overlay.clone());
    Ok(())
}

async fn run_daemon(
    cfg_rx: reload::Rx,
    reload_handle: reload::Handle,
    initial_trigger_code: u16,
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
    let (trigger_tx, mut trigger_rx) = tokio::sync::mpsc::unbounded_channel::<u16>();
    reload::spawn_hotkey(cfg_rx.clone(), trigger_tx);

    let (pipe_reader, pipe_writer) = os_pipe::pipe().context("create hotkey pipe")?;

    // Suppressor is shared between the CGEventTap callback (decides whether to
    // drop the event for the foreground app) and the daemon main loop (updates
    // the trigger code on `[hotkey].trigger` reload). Lock contention is
    // human-rate; std Mutex is fine.
    let suppressor = Arc::new(Mutex::new(Suppressor::new(initial_trigger_code)));
    let suppressor_for_tap = suppressor.clone();

    thread::Builder::new()
        .name("hotkey-eventtap".into())
        .spawn(move || {
            if let Err(e) = hotkey::provider_darwin::run(pipe_writer, suppressor_for_tap) {
                eprintln!("[hotkey] event tap exited: {e:#}");
                std::process::exit(2);
            }
        })
        .context("spawn hotkey thread")?;

    let (raw_tx, mut raw_rx) = tokio::sync::mpsc::unbounded_channel::<RawKey>();
    thread::Builder::new()
        .name("hotkey-pipe-bridge".into())
        .spawn(move || pipe_to_mpsc(pipe_reader, raw_tx))
        .context("spawn hotkey bridge thread")?;

    eprintln!(
        "[shuo] M4 ready. UDS={} Press {} to toggle recording.",
        socket_path.display(),
        cfg_rx.borrow().hotkey.trigger
    );

    let mut tracker = Tracker::new(initial_trigger_code);
    struct ActiveSession {
        control: tokio::sync::watch::Sender<SessionControl>,
        join: tokio::task::JoinHandle<()>,
    }

    let mut active: Option<ActiveSession> = None;

    loop {
        tokio::select! {
            Some(new_code) = trigger_rx.recv() => {
                // 重 trigger：换 Tracker（pressed 状态归零）+ 同步给 CGEventTap callback
                // 里的 Suppressor。CGEventTap 不动——它本来就抓所有键。Suppressor 的
                // `held` 不清，旧 trigger 已按下的物理键 keyup 仍会被正确吞掉（§5 不变量 8）。
                tracker = Tracker::new(new_code);
                if let Ok(mut s) = suppressor.lock() {
                    s.set_trigger(new_code);
                }
                continue;
            }
            maybe_raw = raw_rx.recv() => {
                let Some(raw) = maybe_raw else {
                    anyhow::bail!("hotkey bridge channel closed");
                };
                if raw.down && raw.code == KEY_ESCAPE {
                    if let Some(session) = active.as_ref() {
                        let _ = session.control.send(SessionControl::Cancel);
                    }
                    continue;
                }
                if !matches!(tracker.on_raw(raw), Some(HotkeyEvent::TriggerRecord)) {
                    continue;
                }
                if active.as_ref().is_some_and(|session| session.join.is_finished()) {
                    active = None;
                }
                match active.as_ref() {
                    None => {
                        // 新 session 起来时从 cfg_rx 取最新 voice/asr 配置。
                        let cfg = cfg_rx.borrow().clone();
                        let provider = match build_provider(&cfg.asr.provider) {
                            Ok(provider) => provider,
                            Err(e) => {
                                eprintln!("[asr] provider init failed: {e:#}");
                                state_store.set_error(None);
                                continue;
                            }
                        };
                        let (control_tx, control_rx) = tokio::sync::watch::channel(SessionControl::Idle);
                        let params = SessionParams {
                            auto_paste: cfg.voice.auto_paste,
                            record_audio: cfg.voice.record_audio,
                            stop_delay_ms: cfg.voice.stop_delay_ms,
                            hotwords: cfg.asr.hotwords.clone(),
                            overlay: Some(overlay.clone()),
                            state: state_store.clone(),
                        };
                        let join = tokio::spawn(async move {
                            voice::finish::run_recording(provider.as_ref(), params, control_rx).await;
                        });
                        active = Some(ActiveSession { control: control_tx, join });
                    }
                    Some(session) => {
                        let _ = session.control.send(SessionControl::Stop);
                    }
                }
            }
        }
    }
}

fn build_provider(name: &str) -> Result<Arc<dyn asr::AsrProvider>> {
    match name {
        "doubao" => Ok(Arc::new(
            asr::providers::doubao::DoubaoProvider::new().context("init doubao provider")?,
        )),
        other => anyhow::bail!("未知 ASR provider {other:?}。M5 仅支持 \"doubao\""),
    }
}

fn pipe_to_mpsc(mut reader: os_pipe::PipeReader, tx: tokio::sync::mpsc::UnboundedSender<RawKey>) {
    let mut buf = [0u8; 4];
    loop {
        if let Err(e) = reader.read_exact(&mut buf) {
            eprintln!("[hotkey] pipe read failed: {e}");
            return;
        }
        if tx.send(RawKey::decode(buf)).is_err() {
            return;
        }
    }
}
