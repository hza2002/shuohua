//! Voice: cpal capture → resample to canonical 16k mono s16le → WAV.
//!
//! M1 scope: one-shot 3-second recording per trigger. No streaming ASR, no
//! VAD, no async runtime. The 16k s16le canonical format defined here is
//! the same one M2+ ASR providers will consume (see docs/DESIGN.md §2.9).

mod recorder;

use anyhow::Result;
use std::path::Path;

const M1_RECORD_SECS: f64 = 3.0;

pub fn record_three_seconds(out_path: &Path) -> Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    recorder::record_to_wav(out_path, M1_RECORD_SECS)
}
