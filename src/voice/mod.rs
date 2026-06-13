//! Voice: cpal capture → resample to canonical 16k mono s16le → (M1: WAV / M2.f: streaming).
//!
//! Canonical PCM 格式 16kHz s16le mono 由 recorder 模块归一化。所有下游
//! （ASR provider、留存 wav、VAD）都直接消费这份格式，docs/DESIGN.md §2.9。

pub mod dispatch;
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
