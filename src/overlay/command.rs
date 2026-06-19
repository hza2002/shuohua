use tokio::sync::mpsc;

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
    /// 配置热重载：替换 chrome（glass / tint / 文本布局相关参数）。
    /// model 不消费，view 单独处理。
    ReloadConfig {
        cfg: crate::config::theme::EffectiveOverlayCfg,
    },
    /// 语言切换后让 view 重新翻译当前 state label 并刷新。i18n 字典已经被
    /// `reload::spawn_i18n` 在到达 view 之前换好。
    Relabel,
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
    /// 终态错误文本，覆盖 partial/final，红字显示，3s 后随 overlay 自动 hide。
    Error,
}

#[derive(Debug, Clone)]
pub struct OverlayHandle {
    tx: mpsc::UnboundedSender<OverlayCmd>,
}

impl OverlayHandle {
    pub fn channel() -> (Self, mpsc::UnboundedReceiver<OverlayCmd>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx }, rx)
    }

    pub fn send(&self, cmd: OverlayCmd) {
        let _ = self.tx.send(cmd);
    }
}
