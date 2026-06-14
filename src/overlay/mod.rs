use tokio::sync::mpsc;

pub mod animations;
#[cfg(debug_assertions)]
pub mod debug;
pub mod view;

// Gruvbox dark palette (https://github.com/morhetz/gruvbox).
// 颜色不放进 config：状态色是语义、文字色是排版层级，都是设计决策不是用户偏好。
// 想换主题改这里。
pub const COLOR_PRIMARY_TEXT: u32 = 0xFBF1C7; // fg0 / light0
pub const COLOR_SECONDARY_TEXT: u32 = 0xBDAE93; // fg3
pub const COLOR_TOAST_WARN: u32 = 0xFABD2F; // bright_yellow
pub const COLOR_TOAST_ERROR: u32 = 0xFB4934; // bright_red

#[derive(Debug, Clone)]
pub enum OverlayCmd {
    SetState {
        state: OverlayState,
    },
    SetStats {
        dur_ms: u64,
        chars: u32,
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
    Toast {
        text: String,
        level: ToastLevel,
        ttl_ms: u32,
    },
    Hide,
    /// 配置热重载：替换 chrome（glass / tint / 文本布局相关参数）。
    /// model 不消费，view 单独处理。
    ReloadConfig {
        cfg: crate::config::OverlayCfg,
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

    pub fn color_rgb(self) -> u32 {
        // Gruvbox semantic colors.
        match self {
            OverlayState::Idle => 0x8EC07C,       // aqua
            OverlayState::Connecting => 0xFE8019, // bright_orange
            OverlayState::Recording => 0xFB4934,  // bright_red
            OverlayState::Thinking => 0x458588,   // bright_blue
            OverlayState::Stopping => 0xFABD2F,   // bright_yellow
            OverlayState::Error => 0xCC241D,      // red（比 Recording 略沉，区分语义）
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKind {
    Partial,
    Final,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub text: String,
    pub level: ToastLevel,
    pub ttl_ms: u32,
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

#[derive(Debug, Clone)]
pub struct OverlayModel {
    pub state: OverlayState,
    pub state_label: String,
    pub state_color: u32,
    pub dur_ms: u64,
    pub chars: u32,
    pub bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub chain_summary: String,
    pub segments: Vec<String>,
    pub partial: String,
    pub final_text: String,
    pub toast: Option<Toast>,
    pub visible: bool,
}

impl Default for OverlayModel {
    fn default() -> Self {
        Self {
            state: OverlayState::Idle,
            state_label: crate::t!("overlay.state_idle"),
            state_color: OverlayState::Idle.color_rgb(),
            dur_ms: 0,
            chars: 0,
            bundle_id: None,
            app_name: None,
            chain_summary: String::new(),
            segments: Vec::new(),
            partial: String::new(),
            final_text: String::new(),
            toast: None,
            visible: false,
        }
    }
}

impl OverlayModel {
    pub fn apply(&mut self, cmd: OverlayCmd) {
        match cmd {
            OverlayCmd::SetState { state } => {
                if matches!(state, OverlayState::Connecting) {
                    self.clear_session();
                }
                self.state = state;
                self.state_label = crate::t!(state.label_key());
                self.state_color = state.color_rgb();
                self.visible = !matches!(state, OverlayState::Idle);
            }
            OverlayCmd::SetStats { dur_ms, chars } => {
                self.dur_ms = dur_ms;
                self.chars = chars;
            }
            OverlayCmd::SetApp {
                bundle_id,
                app_name,
                chain_summary,
            } => {
                self.bundle_id = bundle_id;
                self.app_name = app_name;
                self.chain_summary = chain_summary;
            }
            OverlayCmd::SetText { text, kind } => match kind {
                TextKind::Partial => self.partial = text,
                TextKind::Final => {
                    self.final_text = text;
                    self.partial.clear();
                }
            },
            OverlayCmd::AppendSegment { text } => {
                self.segments.push(text);
                self.partial.clear();
            }
            OverlayCmd::Toast {
                text,
                level,
                ttl_ms,
            } => {
                self.toast = Some(Toast {
                    text,
                    level,
                    ttl_ms,
                });
            }
            OverlayCmd::Hide => {
                self.clear_session();
                self.visible = false;
                self.state = OverlayState::Idle;
                self.state_label = crate::t!("overlay.state_idle");
                self.state_color = OverlayState::Idle.color_rgb();
            }
            OverlayCmd::ReloadConfig { .. } => {
                // 仅 view 关心；model 无状态变更。
            }
            OverlayCmd::Relabel => {
                self.state_label = crate::t!(self.state.label_key());
            }
        }
    }

    fn clear_session(&mut self) {
        self.dur_ms = 0;
        self.chars = 0;
        self.segments.clear();
        self.partial.clear();
        self.final_text.clear();
        self.toast = None;
    }

    pub fn display_text(&self) -> String {
        if !self.final_text.is_empty() {
            return self.final_text.clone();
        }
        let mut text = self.segments.join("");
        text.push_str(&self.partial);
        text
    }
}

#[cfg(test)]
mod tests {
    use crate::i18n;

    use super::*;

    #[test]
    fn model_applies_state_text_stats_and_toast() {
        i18n::init("en-US");
        let mut model = OverlayModel::default();

        model.apply(OverlayCmd::SetState {
            state: OverlayState::Recording,
        });
        model.apply(OverlayCmd::SetStats {
            dur_ms: 3200,
            chars: 84,
        });
        model.apply(OverlayCmd::AppendSegment {
            text: "今天".to_string(),
        });
        model.apply(OverlayCmd::SetText {
            text: "今天天气".to_string(),
            kind: TextKind::Partial,
        });
        model.apply(OverlayCmd::Toast {
            text: "network reconnecting".to_string(),
            level: ToastLevel::Warn,
            ttl_ms: 1500,
        });

        assert_eq!(model.state, OverlayState::Recording);
        assert_eq!(model.state_label, "Recording");
        assert_eq!(model.dur_ms, 3200);
        assert_eq!(model.chars, 84);
        assert_eq!(model.segments, vec!["今天"]);
        assert_eq!(model.partial, "今天天气");
        assert_eq!(model.toast.as_ref().unwrap().level, ToastLevel::Warn);
    }

    #[test]
    fn handle_send_is_non_fatal_when_receiver_is_gone() {
        let (handle, rx) = OverlayHandle::channel();
        drop(rx);
        handle.send(OverlayCmd::Hide);
    }

    #[test]
    fn hide_clears_transient_recording_text() {
        i18n::init("en-US");
        let mut model = OverlayModel::default();
        model.apply(OverlayCmd::AppendSegment {
            text: "old".to_string(),
        });
        model.apply(OverlayCmd::SetText {
            text: "old final".to_string(),
            kind: TextKind::Final,
        });
        model.apply(OverlayCmd::Hide);

        assert_eq!(model.display_text(), "");
        assert_eq!(model.dur_ms, 0);
        assert!(model.toast.is_none());
    }

    #[test]
    fn connecting_starts_with_empty_text() {
        i18n::init("en-US");
        let mut model = OverlayModel::default();
        model.apply(OverlayCmd::AppendSegment {
            text: "old".to_string(),
        });
        model.apply(OverlayCmd::SetState {
            state: OverlayState::Connecting,
        });

        assert_eq!(model.display_text(), "");
        assert!(model.visible);
    }
}
