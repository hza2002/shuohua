//! Voice 子系统：cpal 流式录音 + ASR session orchestration + dispatch。
//!
//! Canonical PCM = 16kHz s16le mono；所有下游（ASR provider、wav 留存、
//! 将来 VAD）都消费这份归一化格式（docs/DESIGN.md §2.9）。
//!
//! 顶层入口：[`finish::run_recording`]，一次按 F16 起停的完整生命周期。

pub mod dispatch;
pub mod finish;
pub mod recorder;
