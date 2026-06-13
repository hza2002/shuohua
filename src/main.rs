//! shuohua daemon entry.
//!
//! M2.f status: F16 toggle → record → DoubaoProvider 流式 → 剪贴板 → Cmd+V。
//!
//!   * tokio multi-thread runtime
//!   * hotkey CGEventTap CFRunLoop 专用 OS 线程 → os_pipe → 桥到 tokio mpsc
//!   * Tracker (M1 纯函数状态机) 消化 RawKey → HotkeyEvent
//!   * F16 第一次按 = 起录音 (spawn voice::finish::run_recording)；
//!     第二次按 = 发 stop oneshot 让 task 收尾
//!   * 录音 task 跟主循环解耦：第二次 F16 之后用户立刻能开新录音 (前一次
//!     finalize 在 background 跑)
//!
//! Next:
//!   M2.5: RuleBased filler 去口语词（已完成）
//!   M3:   StateStore + history.jsonl + AppKit overlay

mod asr;
mod autotype_darwin;
mod clipboard_darwin;
mod config;
mod hotkey;
mod post;
mod voice;

use anyhow::{Context, Result};
use std::io::Read;
use std::sync::Arc;
use std::thread;

use hotkey::{HotkeyEvent, RawKey, Tracker};
use voice::finish::SessionParams;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let cfg_path = config::default_path();
    let cfg = config::load_from(&cfg_path).context("load config")?;
    let trigger_code = hotkey::parse::parse(&cfg.hotkey.trigger)
        .with_context(|| format!("parse [hotkey] trigger = {:?}", cfg.hotkey.trigger))?;

    let provider: Arc<dyn asr::AsrProvider> = match cfg.asr.provider.as_str() {
        "doubao" => Arc::new(
            asr::providers::doubao::DoubaoProvider::new().context("init doubao provider")?,
        ),
        other => anyhow::bail!(
            "未知 ASR provider {other:?}。M2 仅支持 \"doubao\""
        ),
    };

    eprintln!(
        "[shuo] config {} loaded:\n         trigger={} (code=0x{:02X})\n         \
         asr.provider={} (caps multilingual={})\n         voice.auto_paste={}  \
         voice.record_audio={}  voice.stop_delay_ms={}",
        cfg_path.display(),
        cfg.hotkey.trigger,
        trigger_code,
        provider.name(),
        provider.caps().multilingual,
        cfg.voice.auto_paste,
        cfg.voice.record_audio,
        cfg.voice.stop_delay_ms,
    );
    eprintln!("[shuo] {} hotwords loaded", cfg.asr.hotwords.len());

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

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<RawKey>();
    thread::Builder::new()
        .name("hotkey-pipe-bridge".into())
        .spawn(move || pipe_to_mpsc(pipe_reader, tx))
        .context("spawn hotkey bridge thread")?;

    eprintln!("[shuo] M2.f ready. Press {} to toggle recording.", cfg.hotkey.trigger);

    let mut tracker = Tracker::new(trigger_code);
    // toggle 状态：Some = 当前在录，stop sender 等着；None = 空闲
    let mut active_stop: Option<tokio::sync::oneshot::Sender<()>> = None;

    while let Some(raw) = rx.recv().await {
        if !matches!(tracker.on_raw(raw), Some(HotkeyEvent::TriggerRecord)) {
            continue;
        }
        match active_stop.take() {
            None => {
                // 起新录音
                let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
                let params = SessionParams {
                    auto_paste: cfg.voice.auto_paste,
                    record_audio: cfg.voice.record_audio,
                    stop_delay_ms: cfg.voice.stop_delay_ms,
                    hotwords: cfg.asr.hotwords.clone(),
                    segment_separator: cfg.voice.segment_separator.clone(),
                };
                let provider = provider.clone();
                tokio::spawn(async move {
                    voice::finish::run_recording(provider.as_ref(), params, stop_rx).await;
                });
                active_stop = Some(stop_tx);
            }
            Some(stop_tx) => {
                // 通知正在录的 task 收尾。它 background 跑完 finalize + dispatch。
                let _ = stop_tx.send(());
                // 不 await：主循环继续接受下一次 F16（用户可立刻开始新一段）。
            }
        }
    }
    anyhow::bail!("hotkey bridge channel closed");
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
