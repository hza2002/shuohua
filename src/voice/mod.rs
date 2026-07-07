//! Voice 子系统：cpal 流式录音 + ASR session orchestration + dispatch。
//!
//! 不变量、边界与扩展见 docs/modules/voice.md。
//!
//! Canonical PCM = 16kHz s16le mono。
//!
//! 顶层入口：[`finish::run_recording`]，一次快捷键起停的完整生命周期。

#[cfg(target_os = "macos")]
pub(crate) mod apple_source;
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
pub(crate) mod resume;
pub mod silero;
pub mod timeline;
pub mod vad;
pub(crate) mod webrtc_apm;

use tokio_util::sync::CancellationToken;

/// 一次录音的控制信号：两个 **level-triggered 终态闩**（terminal latch）。
///
/// 不变量（详见 docs/modules/voice.md control 红线）：
/// - `cancel`：用户取消。**广播**给所有阶段（engine / finalize / drain / post-dispatch）。
/// - `stop`：用户停止。**只**有 engine 的 `'active` / `'idle` 边界关心；finalize / drain /
///   post-dispatch 只拿 [`CancelSignal`]（[`Self::cancel_signal`]），既看不到 stop，也无法
///   主动触发 cancel —— 从类型上杜绝把 Stop 边沿吞掉、也杜绝下游误发 cancel。
///
/// 两个信号都单调（仅 未置位 → 置位，不可回退），且 **cancel 优先于 stop**：任何观察处
/// 一律先查 cancel。基于 [`CancellationToken`]：`cancelled()` / `stopped()` 一旦置位永远
/// resolve、可被任意多处反复 await/查询、不会被"消费"，因此不依赖 `changed()` 边沿的
/// 一次性语义，也不会因重复请求被合并而丢通知。
#[derive(Clone, Default)]
pub struct SessionControl {
    stop: CancellationToken,
    cancel: CancellationToken,
}

impl SessionControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request_stop(&self) {
        self.stop.cancel();
    }

    pub fn request_cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_stop_requested(&self) -> bool {
        self.stop.is_cancelled()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// 等待 stop 被请求。engine `'active` / `'idle` 边界专用（level-triggered，可重复 await）。
    pub async fn stopped(&self) {
        self.stop.cancelled().await;
    }

    /// 等待 cancel 被请求。
    pub async fn cancelled(&self) {
        self.cancel.cancelled().await;
    }

    /// 交给不应观察 stop、也不应主动 cancel 的阶段（finalize / drain / post-dispatch）的
    /// **只读** cancel 视图。返回 [`CancelSignal`] 而非裸 token：下游只能 await / 查询，
    /// 触发 cancel 的能力留在持有 `SessionControl` 的 daemon 一侧。
    pub fn cancel_signal(&self) -> CancelSignal<'_> {
        CancelSignal(&self.cancel)
    }
}

/// 只读 cancel 视图：从 [`SessionControl::cancel_signal`] 借出。只暴露观察能力
/// （await / 查询），**不**暴露 `request_cancel`，也完全看不到 stop —— 让「stop 引擎私有、
/// 下游对 cancel 只读」成为类型保证而非约定。
#[derive(Clone, Copy)]
pub struct CancelSignal<'a>(&'a CancellationToken);

impl<'a> CancelSignal<'a> {
    /// 测试用构造器：从一个裸 token 借出只读视图。
    #[cfg(test)]
    pub(crate) fn new(token: &'a CancellationToken) -> Self {
        Self(token)
    }

    /// 等待 cancel 被请求（level-triggered，可重复 await）。
    pub async fn cancelled(&self) {
        self.0.cancelled().await;
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

#[cfg(test)]
mod tests {
    use super::SessionControl;
    use std::time::Duration;

    /// stop-wedge 回归：finalize / drain / post 只拿 [`SessionControl::cancel_signal`]。
    /// 请求 stop 不得让 cancel 视图看到取消，也不得把 stop "消费"掉 —— 旧的 watch +
    /// `borrow_and_update` 设计正是在此把 stop 边沿吞掉导致卡死。
    #[tokio::test]
    async fn stop_is_invisible_to_the_cancel_view_and_never_lost() {
        let control = SessionControl::new();
        let cancel = control.cancel_signal();

        control.request_stop();

        assert!(!cancel.is_cancelled(), "stop must not surface as cancel");
        assert!(
            tokio::time::timeout(Duration::from_millis(20), cancel.cancelled())
                .await
                .is_err(),
            "a cancel-only wait must not wake on stop"
        );
        // stop 仍可被 engine 边界电平观察到（未被消费），且 stopped() 立即 resolve。
        assert!(control.is_stop_requested());
        control.stopped().await;
    }

    /// 两个信号都是幂等的 level-triggered 终态闩：重复请求不丢通知、不 panic；
    /// 已置位后 await 立即返回（不依赖一次性边沿）。
    #[tokio::test]
    async fn signals_are_idempotent_terminal_latches() {
        let control = SessionControl::new();
        assert!(!control.is_stop_requested());
        assert!(!control.is_cancelled());

        control.request_stop();
        control.request_stop();
        assert!(control.is_stop_requested());
        control.stopped().await;

        control.request_cancel();
        control.request_cancel();
        assert!(control.is_cancelled());
        control.cancelled().await;
    }
}
