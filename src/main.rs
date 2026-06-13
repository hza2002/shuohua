//! shuohua M1 binary.
//!
//! Roadmap (modules wired in as milestones complete; files for future modules
//! exist in src/ with a single TODO line, not yet in the mod tree):
//!
//!   M1  : hotkey + voice (this milestone)
//!   M2  : asr, config, autotype_darwin, clipboard_darwin, voice::finish/dispatch
//!   M2.5: voice::vad, post::filler, i18n
//!   M3  : state, overlay
//!   M4  : ipc, tui
//!   M5  : cli, doctor

mod hotkey;
mod voice;

use anyhow::{Context, Result};
use std::io::Read;
use std::path::PathBuf;
use std::thread;

use hotkey::{HotkeyEvent, RawKey, Tracker};

/// macOS virtual keycode for F16 (HIToolbox/Events.h `kVK_F16 = 0x6A`).
/// M2: this constant disappears; trigger is parsed from `config.toml`.
const M1_TRIGGER_KEYCODE: u16 = 0x6A;

fn main() -> Result<()> {
    let (reader, writer) = os_pipe::pipe().context("create pipe")?;

    thread::Builder::new()
        .name("hotkey-eventtap".into())
        .spawn(move || {
            if let Err(e) = hotkey::provider_darwin::run(writer) {
                eprintln!("[hotkey] event tap exited: {e:#}");
                std::process::exit(2);
            }
        })
        .context("spawn hotkey thread")?;

    eprintln!("[shuo] M1 ready. Press F16 to record 3 seconds.");
    eprintln!("[shuo] WAV will be written to ./tmp/m1-<n>.wav");

    let mut tracker = Tracker::new(M1_TRIGGER_KEYCODE);
    let mut buf = [0u8; 4];
    let mut counter: u32 = 0;
    let mut pipe_reader = reader;

    loop {
        pipe_reader.read_exact(&mut buf).context("read from pipe")?;
        let raw = RawKey::decode(buf);
        if let Some(HotkeyEvent::TriggerRecord) = tracker.on_raw(raw) {
            counter += 1;
            let out = PathBuf::from(format!("tmp/m1-{counter}.wav"));
            eprintln!("[shuo] recording 3s → {}", out.display());
            match voice::record_three_seconds(&out) {
                Ok(()) => eprintln!("[shuo] wrote {}", out.display()),
                Err(e) => eprintln!("[shuo] record failed: {e:#}"),
            }
        }
    }
}
