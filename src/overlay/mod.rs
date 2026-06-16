use tokio::sync::mpsc;

#[cfg(debug_assertions)]
pub mod debug;
pub mod view;

// Gruvbox dark palette (https://github.com/morhetz/gruvbox).
// 颜色不放进 config：状态色是语义、文字色是排版层级，都是设计决策不是用户偏好。
// 想换主题改这里。
#[allow(dead_code)]
pub mod palette {
    pub const FG0: u32 = 0xFBF1C7;
    pub const FG1: u32 = 0xEBDBB2;
    pub const FG2: u32 = 0xD5C4A1;
    pub const FG3: u32 = 0xBDAE93;
    pub const FG4: u32 = 0xA89984;
    pub const GRAY: u32 = 0x928374;
    pub const BRIGHT_RED: u32 = 0xFB4934;
    pub const RED: u32 = 0xCC241D;
    pub const BRIGHT_ORANGE: u32 = 0xFE8019;
    pub const BRIGHT_YELLOW: u32 = 0xFABD2F;
    pub const BRIGHT_BLUE: u32 = 0x83A598;
    pub const BRIGHT_AQUA: u32 = 0x8EC07C;
}

pub const COLOR_PRIMARY_TEXT: u32 = palette::FG0;
pub const COLOR_SECONDARY_TEXT: u32 = palette::FG1;
pub const COLOR_TERTIARY_TEXT: u32 = palette::FG3;
pub const COLOR_SEGMENT_TEXT: u32 = palette::FG1;
pub const COLOR_NOTICE: u32 = palette::BRIGHT_YELLOW;
pub const COLOR_ERROR_TEXT: u32 = palette::BRIGHT_RED;

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
            OverlayState::Idle => palette::BRIGHT_AQUA,
            OverlayState::Connecting => palette::BRIGHT_ORANGE,
            OverlayState::Recording => palette::BRIGHT_RED,
            OverlayState::Thinking => palette::BRIGHT_BLUE,
            OverlayState::Stopping => palette::BRIGHT_YELLOW,
            OverlayState::Error => palette::RED,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
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
    pub words: u32,
    pub bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub chain_summary: String,
    pub segments: Vec<String>,
    pub partial: String,
    pub final_text: String,
    /// 终态错误文案；非空时盖住 partial/final，红字显示。
    pub error_text: String,
    /// 当前 meta 行的临时 warn；非空时 meta 显示 notice.text 黄字，定时器到点自动恢复。
    pub notice: Option<Notice>,
    pub visible: bool,
}

impl Default for OverlayModel {
    fn default() -> Self {
        Self {
            state: OverlayState::Idle,
            state_label: crate::t!("overlay.state_idle"),
            state_color: OverlayState::Idle.color_rgb(),
            dur_ms: 0,
            words: 0,
            bundle_id: None,
            app_name: None,
            chain_summary: String::new(),
            segments: Vec::new(),
            partial: String::new(),
            final_text: String::new(),
            error_text: String::new(),
            notice: None,
            visible: false,
        }
    }
}

impl OverlayModel {
    pub fn apply(&mut self, cmd: OverlayCmd) {
        match cmd {
            OverlayCmd::SetState { state } => {
                // `Connecting` 是 session 起点；只有它把 overlay 拉起来。
                // M10 多 session 路径上 `Idle` 表示"当前没 ASR session，麦克风
                // 仍在听" — 这种状态下 overlay 必须保持可见，所以 SetState
                // 不再隐式地把 visible 跟 Idle 绑死。可见性只由 Connecting
                // 拉起，由 Hide / Dismiss 关闭。
                if matches!(state, OverlayState::Connecting) {
                    self.clear_session();
                    self.visible = true;
                }
                self.state = state;
                self.state_label = crate::t!(state.label_key());
                self.state_color = state.color_rgb();
            }
            OverlayCmd::SetStats { dur_ms, words } => {
                self.dur_ms = dur_ms;
                self.words = words;
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
                TextKind::Error => {
                    self.error_text = text;
                    self.partial.clear();
                }
            },
            OverlayCmd::AppendSegment { text } => {
                self.segments.push(text);
                self.partial.clear();
            }
            OverlayCmd::Notice { text, ttl_ms } => {
                self.notice = Some(Notice { text, ttl_ms });
            }
            OverlayCmd::Dismiss | OverlayCmd::Hide => {
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
        self.words = 0;
        self.segments.clear();
        self.partial.clear();
        self.final_text.clear();
        self.error_text.clear();
        self.notice = None;
    }

    pub fn display_text(&self) -> String {
        // 优先级：error > final > segments+partial。
        // error 在录音失败时盖住识别文本；history 已保留所有片段。
        if !self.error_text.is_empty() {
            return self.error_text.clone();
        }
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
    fn model_applies_state_text_stats_and_notice() {
        i18n::init("en-US");
        let mut model = OverlayModel::default();

        model.apply(OverlayCmd::SetState {
            state: OverlayState::Recording,
        });
        model.apply(OverlayCmd::SetStats {
            dur_ms: 3200,
            words: 14,
        });
        model.apply(OverlayCmd::AppendSegment {
            text: "今天".to_string(),
        });
        model.apply(OverlayCmd::SetText {
            text: "今天天气".to_string(),
            kind: TextKind::Partial,
        });
        model.apply(OverlayCmd::Notice {
            text: "filler skipped".to_string(),
            ttl_ms: 3000,
        });

        assert_eq!(model.state, OverlayState::Recording);
        assert_eq!(model.state_label, "Recording");
        assert_eq!(model.dur_ms, 3200);
        assert_eq!(model.words, 14);
        assert_eq!(model.segments, vec!["今天"]);
        assert_eq!(model.partial, "今天天气");
        assert_eq!(model.notice.as_ref().unwrap().text, "filler skipped");
    }

    #[test]
    fn error_text_overrides_partial_and_final_in_display() {
        i18n::init("en-US");
        let mut model = OverlayModel::default();
        model.apply(OverlayCmd::AppendSegment {
            text: "已识别一半".to_string(),
        });
        model.apply(OverlayCmd::SetText {
            text: "请检查输入设备".to_string(),
            kind: TextKind::Error,
        });
        assert_eq!(model.display_text(), "请检查输入设备");
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
        assert!(model.notice.is_none());
        assert!(model.error_text.is_empty());
    }

    #[test]
    fn set_state_idle_keeps_overlay_visible() {
        // M10 多 session 路径：VAD 切到 Idle 子状态时，overlay 仍要可见，
        // 不能跟着 visible=false。可见性只由 Connecting 拉起 / Hide 关闭。
        i18n::init("en-US");
        let mut model = OverlayModel::default();
        model.apply(OverlayCmd::SetState {
            state: OverlayState::Connecting,
        });
        assert!(model.visible);
        model.apply(OverlayCmd::SetState {
            state: OverlayState::Recording,
        });
        assert!(model.visible);
        model.apply(OverlayCmd::SetState {
            state: OverlayState::Idle,
        });
        assert!(model.visible, "Idle 子状态期间 overlay 应保持可见");
        assert_eq!(model.state, OverlayState::Idle);
        model.apply(OverlayCmd::Hide);
        assert!(!model.visible);
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
