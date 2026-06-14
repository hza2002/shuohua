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
mod clipboard_darwin;
mod config;
mod focused_window_darwin;
mod hotkey;
mod i18n;
mod log;
mod overlay;
mod post;
mod reload;
mod state;
mod voice;

use anyhow::{Context, Result};
use std::io::Read;
use std::sync::Arc;
use std::thread;

use hotkey::{HotkeyEvent, RawKey, Tracker};
use overlay::OverlayHandle;
use state::StateStore;
use voice::finish::SessionParams;
use voice::SessionControl;

const KEY_ESCAPE: u16 = 0x35;

fn main() -> Result<()> {
    let cfg_path = config::default_path();
    let cfg_rx = reload::watch(cfg_path.clone()).context("start config watcher")?;
    let cfg: Arc<config::Config> = cfg_rx.borrow().clone();
    i18n::init(&cfg.ui.language);
    let trigger_code = hotkey::parse::parse(&cfg.hotkey.trigger)
        .with_context(|| format!("parse [hotkey] trigger = {:?}", cfg.hotkey.trigger))?;

    let provider: Arc<dyn asr::AsrProvider> = match cfg.asr.provider.as_str() {
        "doubao" => {
            Arc::new(asr::providers::doubao::DoubaoProvider::new().context("init doubao provider")?)
        }
        other => anyhow::bail!("未知 ASR provider {other:?}。M2 仅支持 \"doubao\""),
    };

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

    let provider_for_daemon = provider.clone();
    let cfg_rx_for_daemon = cfg_rx.clone();
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
                trigger_code,
                provider_for_daemon,
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
    initial_trigger_code: u16,
    provider: Arc<dyn asr::AsrProvider>,
    overlay: OverlayHandle,
    state_store: StateStore,
) -> Result<()> {
    // 三个 subscriber，跟主循环解耦。每个都在 reload 模块里实现。
    reload::spawn_overlay(cfg_rx.clone(), overlay.clone());
    reload::spawn_i18n(cfg_rx.clone(), overlay.clone());
    let (trigger_tx, mut trigger_rx) = tokio::sync::mpsc::unbounded_channel::<u16>();
    reload::spawn_hotkey(cfg_rx.clone(), trigger_tx);

    let (pipe_reader, pipe_writer) = os_pipe::pipe().context("create hotkey pipe")?;

    thread::Builder::new()
        .name("hotkey-eventtap".into())
        .spawn(move || {
            if let Err(e) = hotkey::provider_darwin::run(pipe_writer) {
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
        "[shuo] M3.f ready. Press {} to toggle recording.",
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
                // 重 trigger：换 Tracker（pressed 状态归零），CGEventTap 不动——它本来就抓所有键。
                tracker = Tracker::new(new_code);
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
                        let (control_tx, control_rx) = tokio::sync::watch::channel(SessionControl::Idle);
                        let params = SessionParams {
                            auto_paste: cfg.voice.auto_paste,
                            record_audio: cfg.voice.record_audio,
                            stop_delay_ms: cfg.voice.stop_delay_ms,
                            hotwords: cfg.asr.hotwords.clone(),
                            overlay: Some(overlay.clone()),
                            state: state_store.clone(),
                        };
                        let provider = provider.clone();
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
