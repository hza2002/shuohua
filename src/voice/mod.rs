//! Voice 子系统：cpal 流式录音 + ASR session orchestration + dispatch。
//!
//! 不变量、边界与扩展见 docs/modules/voice.md。
//!
//! Canonical PCM = 16kHz s16le mono。
//!
//! 顶层入口：[`finish::run_recording`]，一次快捷键起停的完整生命周期。

pub(crate) mod audio;
pub(crate) mod capture;
pub mod dispatch;
pub(crate) mod engine;
#[cfg(test)]
mod engine_lifecycle_tests;
pub(crate) mod finalize;
pub mod finish;
pub(crate) mod history_build;
pub mod meter;
pub mod observer;
pub(crate) mod post_dispatch;
pub mod recorder;
pub mod silero;
pub mod timeline;
pub mod vad;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionControl {
    Idle,
    Stop,
    Cancel,
}
