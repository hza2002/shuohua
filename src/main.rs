//! shuohua daemon entry.
//!
//! M2.a status:
//!   * tokio multi-thread runtime in place
//!   * hotkey thread (CGEventTap CFRunLoop) still writes RawKey frames into an
//!     os_pipe — this part doesn't change across milestones, the C callback
//!     needs a lock-free async-signal-safe sink and a file descriptor is the
//!     simplest such sink we get on macOS
//!   * a small bridge thread reads the pipe and forwards each RawKey through a
//!     tokio mpsc into the async main loop
//!   * recording is still the blocking M1 path (`record_three_seconds`); wrapped
//!     in spawn_blocking so the runtime keeps progressing. Streaming recorder
//!     lands in M2.f
//!
//! Roadmap (modules wired in as milestones complete):
//!
//!   M2.b: config, hotkey/{parse,registry}
//!   M2.c: asr trait skeleton (asr/{mod,types})
//!   M2.d: DoubaoProvider (asr/providers/doubao)
//!   M2.e: clipboard_darwin, autotype_darwin, voice/dispatch
//!   M2.f: streaming recorder + voice::finish + end-to-end toggle

mod hotkey;
mod voice;

use anyhow::{Context, Result};
use std::io::Read;
use std::path::PathBuf;
use std::thread;

use hotkey::{HotkeyEvent, RawKey, Tracker};

/// macOS virtual keycode for F16 (HIToolbox/Events.h `kVK_F16 = 0x6A`).
/// M2.b: this constant disappears; trigger is parsed from `config.toml`.
const M1_TRIGGER_KEYCODE: u16 = 0x6A;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
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

    // Bridge: blocking pipe reads → tokio mpsc. A dedicated std thread so we
    // don't burn an async runtime worker on a forever-blocking fd read.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<RawKey>();
    thread::Builder::new()
        .name("hotkey-pipe-bridge".into())
        .spawn(move || pipe_to_mpsc(pipe_reader, tx))
        .context("spawn hotkey bridge thread")?;

    eprintln!("[shuo] M2.a ready. Press F16 to record 3 seconds.");
    eprintln!("[shuo] WAV will be written to ./tmp/m1-<n>.wav");

    let mut tracker = Tracker::new(M1_TRIGGER_KEYCODE);
    let mut counter: u32 = 0;

    while let Some(raw) = rx.recv().await {
        if let Some(HotkeyEvent::TriggerRecord) = tracker.on_raw(raw) {
            counter += 1;
            let out = PathBuf::from(format!("tmp/m1-{counter}.wav"));
            eprintln!("[shuo] recording 3s → {}", out.display());
            // Blocking cpal recorder runs on a dedicated blocking thread so the
            // runtime stays responsive. Streaming recorder in M2.f removes this.
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
    // Sender side dropped means the bridge thread died; treat as fatal.
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
            // Receiver gone; main loop is exiting.
            return;
        }
    }
}
