//! shuohua daemon entry.
//!
//! M2.b status:
//!   * tokio multi-thread runtime in place (M2.a)
//!   * hotkey thread (CGEventTap CFRunLoop) writes RawKey frames into an
//!     os_pipe; a bridge std thread forwards them through a tokio mpsc to the
//!     async main loop (M2.a)
//!   * trigger keycode + ASR provider selection now loaded from
//!     ~/.config/shuohua/config.toml (M2.b)
//!   * recording is still the blocking M1 path wrapped in spawn_blocking;
//!     streaming recorder lands in M2.f
//!
//! Next:
//!   M2.c: asr trait skeleton (asr/{mod,types})
//!   M2.d: DoubaoProvider (asr/providers/doubao)
//!   M2.e: clipboard_darwin, autotype_darwin, voice/dispatch
//!   M2.f: streaming recorder + voice::finish + end-to-end toggle

mod asr;
mod config;
mod hotkey;
mod voice;

use anyhow::{Context, Result};
use std::io::Read;
use std::path::PathBuf;
use std::thread;

use hotkey::{HotkeyEvent, RawKey, Tracker};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let cfg_path = config::default_path();
    let cfg = config::load_from(&cfg_path).context("load config")?;
    let trigger_code = hotkey::parse::parse(&cfg.hotkey.trigger)
        .with_context(|| format!("parse [hotkey] trigger = {:?}", cfg.hotkey.trigger))?;
    eprintln!(
        "[shuo] config {} loaded: trigger={} (code=0x{:02X}) asr.provider={}",
        cfg_path.display(),
        cfg.hotkey.trigger,
        trigger_code,
        cfg.asr.provider
    );

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

    eprintln!("[shuo] M2.b ready. Press {} to record 3 seconds.", cfg.hotkey.trigger);
    eprintln!("[shuo] WAV will be written to ./tmp/m1-<n>.wav");

    let mut tracker = Tracker::new(trigger_code);
    let mut counter: u32 = 0;

    while let Some(raw) = rx.recv().await {
        if let Some(HotkeyEvent::TriggerRecord) = tracker.on_raw(raw) {
            counter += 1;
            let out = PathBuf::from(format!("tmp/m1-{counter}.wav"));
            eprintln!("[shuo] recording 3s → {}", out.display());
            let out_for_task = out.clone();
            let result =
                tokio::task::spawn_blocking(move || voice::record_three_seconds(&out_for_task))
                    .await
                    .context("recorder task join")?;
            match result {
                Ok(()) => eprintln!("[shuo] wrote {}", out.display()),
                Err(e) => eprintln!("[shuo] record failed: {e:#}"),
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
