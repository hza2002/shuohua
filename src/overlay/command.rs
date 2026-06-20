use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

const QUEUE_CAPACITY: usize = 256;

#[derive(Debug, Clone)]
pub enum OverlayCmd {
    SetState {
        state: OverlayState,
    },
    SetStats {
        dur_ms: u64,
        words: u32,
    },
    SetApp {
        bundle_id: Option<String>,
        app_name: Option<String>,
        chain_summary: String,
    },
    SetText {
        text: String,
        kind: TextKind,
    },
    AppendSegment {
        text: String,
    },
    /// 用 provider session 的最终全文替换本 session 已追加到 overlay 的
    /// utterance segments。`segments` 是要从尾部替换的 segment 数。
    ReplaceRecentSegments {
        segments: usize,
        text: String,
    },
    /// 非阻断提示，进 meta 行黄字，ttl 到点自动恢复 chain_summary。
    /// 替代以前的 toast warn 用法。
    Notice {
        text: String,
        ttl_ms: u32,
    },
    /// 立即关闭 overlay，跳过所有延期逻辑。ESC 专用。
    Dismiss,
    /// 正常隐藏。如果当前有活跃 notice（warn 还没自动消失），延期到 notice
    /// 到期再真正隐藏，让用户有机会看到 warn。
    Hide,
    /// Overlay runtime config 热重载：替换 chrome、tint、文字布局等渲染参数。
    /// 这是 config/theme 合并后的 snapshot；model 不消费，view 单独处理。
    ReloadConfig {
        cfg: crate::config::theme::EffectiveOverlayCfg,
    },
    /// 语言切换后让 view 重新翻译当前 state label 并刷新。i18n 字典已经被
    /// `reload::spawn_i18n` 在到达 view 之前换好。
    Relabel,
    /// Daemon graceful shutdown: ask the AppKit main loop to terminate.
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayState {
    Idle,
    Connecting,
    Recording,
    Thinking,
    Stopping,
    Error,
}

impl OverlayState {
    pub fn label_key(self) -> &'static str {
        match self {
            OverlayState::Idle => "overlay.state_idle",
            OverlayState::Connecting => "overlay.state_connecting",
            OverlayState::Recording => "overlay.state_recording",
            OverlayState::Thinking => "overlay.state_thinking",
            OverlayState::Stopping => "overlay.state_stopping",
            OverlayState::Error => "overlay.state_error",
        }
    }

    pub fn color_rgb(self, theme: &crate::config::theme::OverlayStateTheme) -> u32 {
        match self {
            OverlayState::Idle => theme.idle,
            OverlayState::Connecting => theme.connecting,
            OverlayState::Recording => theme.recording,
            OverlayState::Thinking => theme.thinking,
            OverlayState::Stopping => theme.stopping,
            OverlayState::Error => theme.error,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKind {
    Partial,
    Final,
    /// 终态错误文本，覆盖 partial/final，红字显示，5s 后随 overlay 自动 hide。
    Error,
}

#[derive(Debug, Clone)]
pub struct OverlayHandle {
    inner: Arc<Mutex<OverlayMailbox>>,
    wake: mpsc::UnboundedSender<()>,
}

#[derive(Debug)]
pub struct OverlayReceiver {
    inner: Arc<Mutex<OverlayMailbox>>,
    wake: mpsc::UnboundedReceiver<()>,
}

#[derive(Debug, Default)]
struct OverlayMailbox {
    queue: VecDeque<OverlayCmd>,
    latest_stats: Option<OverlayCmd>,
    latest_partial: Option<OverlayCmd>,
    wake_pending: bool,
}

impl OverlayHandle {
    pub fn channel() -> (Self, OverlayReceiver) {
        let (wake, wake_rx) = mpsc::unbounded_channel();
        let inner = Arc::new(Mutex::new(OverlayMailbox::default()));
        (
            Self {
                inner: inner.clone(),
                wake,
            },
            OverlayReceiver {
                inner,
                wake: wake_rx,
            },
        )
    }

    pub fn send(&self, cmd: OverlayCmd) {
        let should_wake = {
            let Ok(mut mailbox) = self.inner.lock() else {
                return;
            };
            mailbox.push(cmd);
            if mailbox.wake_pending {
                false
            } else {
                mailbox.wake_pending = true;
                true
            }
        };
        if should_wake && self.wake.send(()).is_err() {
            if let Ok(mut mailbox) = self.inner.lock() {
                mailbox.wake_pending = false;
            }
        }
    }
}

impl OverlayReceiver {
    pub fn try_recv(&mut self) -> Result<OverlayCmd, mpsc::error::TryRecvError> {
        if let Some(cmd) = self.pop_ready() {
            return Ok(cmd);
        }
        match self.wake.try_recv() {
            Ok(()) => {
                if let Ok(mut mailbox) = self.inner.lock() {
                    mailbox.wake_pending = false;
                }
                self.pop_ready().ok_or(mpsc::error::TryRecvError::Empty)
            }
            Err(error) => Err(error),
        }
    }

    fn pop_ready(&mut self) -> Option<OverlayCmd> {
        self.inner.lock().ok()?.pop()
    }
}

impl OverlayMailbox {
    fn push(&mut self, cmd: OverlayCmd) {
        match cmd {
            OverlayCmd::SetStats { .. } => self.latest_stats = Some(cmd),
            OverlayCmd::SetText {
                kind: TextKind::Partial,
                ..
            } => self.latest_partial = Some(cmd),
            _ => {
                if self.queue.len() >= QUEUE_CAPACITY {
                    let _ = self.queue.pop_front();
                    tracing::warn!(
                        area = "overlay",
                        capacity = QUEUE_CAPACITY,
                        "overlay command queue full; dropping oldest command"
                    );
                }
                self.queue.push_back(cmd);
            }
        }
    }

    fn pop(&mut self) -> Option<OverlayCmd> {
        self.queue
            .pop_front()
            .or_else(|| self.latest_stats.take())
            .or_else(|| self.latest_partial.take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_stats_and_partial_are_coalesced() {
        let (handle, mut rx) = OverlayHandle::channel();
        handle.send(OverlayCmd::SetStats {
            dur_ms: 1,
            words: 1,
        });
        handle.send(OverlayCmd::SetStats {
            dur_ms: 2,
            words: 2,
        });
        handle.send(OverlayCmd::SetText {
            text: "old".into(),
            kind: TextKind::Partial,
        });
        handle.send(OverlayCmd::SetText {
            text: "new".into(),
            kind: TextKind::Partial,
        });

        assert!(matches!(
            rx.try_recv().unwrap(),
            OverlayCmd::SetStats {
                dur_ms: 2,
                words: 2
            }
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            OverlayCmd::SetText {
                text,
                kind: TextKind::Partial
            } if text == "new"
        ));
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn critical_commands_are_kept_in_order() {
        let (handle, mut rx) = OverlayHandle::channel();
        handle.send(OverlayCmd::SetState {
            state: OverlayState::Connecting,
        });
        handle.send(OverlayCmd::Hide);

        assert!(matches!(
            rx.try_recv().unwrap(),
            OverlayCmd::SetState {
                state: OverlayState::Connecting
            }
        ));
        assert!(matches!(rx.try_recv().unwrap(), OverlayCmd::Hide));
    }

    #[test]
    fn structural_commands_are_not_reordered_behind_transient_updates() {
        let (handle, mut rx) = OverlayHandle::channel();
        handle.send(OverlayCmd::SetState {
            state: OverlayState::Connecting,
        });
        handle.send(OverlayCmd::SetText {
            text: "first words".into(),
            kind: TextKind::Partial,
        });

        assert!(matches!(
            rx.try_recv().unwrap(),
            OverlayCmd::SetState {
                state: OverlayState::Connecting
            }
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            OverlayCmd::SetText {
                text,
                kind: TextKind::Partial
            } if text == "first words"
        ));
    }

    #[test]
    fn full_queue_keeps_new_critical_command() {
        let (handle, mut rx) = OverlayHandle::channel();
        for i in 0..QUEUE_CAPACITY {
            handle.send(OverlayCmd::Notice {
                text: format!("notice {i}"),
                ttl_ms: 1,
            });
        }
        handle.send(OverlayCmd::Dismiss);

        let mut saw_dismiss = false;
        while let Ok(cmd) = rx.try_recv() {
            if matches!(cmd, OverlayCmd::Dismiss) {
                saw_dismiss = true;
            }
        }

        assert!(saw_dismiss);
    }
}
