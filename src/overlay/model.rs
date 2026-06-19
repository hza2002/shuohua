use std::time::{Duration, Instant};

use crate::overlay::command::{OverlayCmd, OverlayState, TextKind};

/// Notice 默认 TTL。配置以 `OverlayCmd::Notice.ttl_ms` 字段覆盖。
pub const NOTICE_DEFAULT_TTL_MS: u32 = 3000;

/// `SetText{Error}` 后自动 hide overlay 的等待时长。比 notice 长，让用户读完
/// 错误并决定是否重试。
pub const ERROR_TTL_MS: u64 = 5000;

/// `model.tick(now)` 的返回值：模型是否要求 view 采取可见性动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    /// 模型无外显变化要求（dur_ms 可能已更新）。
    Idle,
    /// 模型决定 overlay 该开始隐藏。view 应触发 fade-out 动画。
    Hide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub ttl_ms: u32,
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

    /// Session 时钟：`SetState{Connecting}` 时起跳，`Hide`/`Dismiss` 时清。
    pub recording_started: Option<Instant>,
    /// Notice（meta 行 warn）到期点。`tick(now)` 到点恢复 chain_summary。
    pub notice_until: Option<Instant>,
    /// Error 文本到期点。`tick(now)` 到点自动 hide overlay。
    pub error_until: Option<Instant>,
    /// `Hide` 到达时若 notice 还活着就延期；`tick(now)` 在 notice 到期时释放。
    pub pending_hide: bool,
}

impl OverlayModel {
    pub fn new(theme: &crate::config::theme::OverlayStateTheme) -> Self {
        Self {
            state: OverlayState::Idle,
            state_label: crate::t!("overlay.state_idle"),
            state_color: OverlayState::Idle.color_rgb(theme),
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
            recording_started: None,
            notice_until: None,
            error_until: None,
            pending_hide: false,
        }
    }
}

impl Default for OverlayModel {
    fn default() -> Self {
        Self::new(&crate::config::theme::OverlayStateTheme::default())
    }
}

impl OverlayModel {
    pub fn apply(&mut self, cmd: OverlayCmd, theme: &crate::config::theme::OverlayStateTheme) {
        match cmd {
            OverlayCmd::SetState { state } => {
                if matches!(state, OverlayState::Connecting) {
                    self.clear_session();
                    self.visible = true;
                    self.recording_started = Some(Instant::now());
                    self.notice_until = None;
                    self.error_until = None;
                    self.pending_hide = false;
                }
                self.state = state;
                self.state_label = crate::t!(state.label_key());
                self.state_color = state.color_rgb(theme);
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
                    self.error_until = Some(Instant::now() + Duration::from_millis(ERROR_TTL_MS));
                }
            },
            OverlayCmd::AppendSegment { text } => {
                self.segments.push(text);
                self.partial.clear();
            }
            OverlayCmd::ReplaceRecentSegments { segments, text } => {
                let keep = self.segments.len().saturating_sub(segments);
                self.segments.truncate(keep);
                if !text.is_empty() {
                    self.segments.push(text);
                }
                self.partial.clear();
            }
            OverlayCmd::Notice { text, ttl_ms } => {
                self.notice = Some(Notice { text, ttl_ms });
                self.notice_until = Some(Instant::now() + Duration::from_millis(ttl_ms as u64));
            }
            OverlayCmd::Hide => {
                if self.notice_until.is_some() {
                    self.pending_hide = true;
                } else {
                    self.clear_session();
                    self.visible = false;
                    self.state = OverlayState::Idle;
                    self.state_label = crate::t!("overlay.state_idle");
                    self.state_color = OverlayState::Idle.color_rgb(theme);
                    self.recording_started = None;
                }
            }
            OverlayCmd::Dismiss => {
                self.clear_session();
                self.visible = false;
                self.state = OverlayState::Idle;
                self.state_label = crate::t!("overlay.state_idle");
                self.state_color = OverlayState::Idle.color_rgb(theme);
                self.recording_started = None;
                self.pending_hide = false;
            }
            OverlayCmd::ReloadConfig { .. } => {}
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
        self.notice_until = None;
        self.error_until = None;
    }

    pub fn tick(
        &mut self,
        now: Instant,
        theme: &crate::config::theme::OverlayStateTheme,
    ) -> TickOutcome {
        if let Some(started) = self.recording_started {
            if now >= started {
                self.dur_ms = (now - started).as_millis() as u64;
            }
        }
        if let Some(until) = self.notice_until {
            if now >= until {
                self.notice = None;
                self.notice_until = None;
                if self.pending_hide {
                    self.clear_session();
                    self.visible = false;
                    self.state = OverlayState::Idle;
                    self.state_label = crate::t!("overlay.state_idle");
                    self.state_color = OverlayState::Idle.color_rgb(theme);
                    self.recording_started = None;
                    self.pending_hide = false;
                    return TickOutcome::Hide;
                }
            }
        }
        if let Some(until) = self.error_until {
            if now >= until {
                self.error_until = None;
                self.clear_session();
                self.visible = false;
                self.state = OverlayState::Idle;
                self.state_label = crate::t!("overlay.state_idle");
                self.state_color = OverlayState::Idle.color_rgb(theme);
                self.recording_started = None;
                return TickOutcome::Hide;
            }
        }
        TickOutcome::Idle
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
    use super::*;
    use crate::i18n;
    use crate::overlay::command::{OverlayCmd, OverlayHandle, OverlayState, TextKind};

    fn apply(model: &mut OverlayModel, cmd: OverlayCmd) {
        model.apply(cmd, &crate::config::theme::OverlayStateTheme::default());
    }

    #[test]
    fn model_applies_state_text_stats_and_notice() {
        i18n::init("en-US");
        let mut model = OverlayModel::default();

        apply(
            &mut model,
            OverlayCmd::SetState {
                state: OverlayState::Recording,
            },
        );
        apply(
            &mut model,
            OverlayCmd::SetStats {
                dur_ms: 3200,
                words: 14,
            },
        );
        apply(
            &mut model,
            OverlayCmd::AppendSegment {
                text: "今天".to_string(),
            },
        );
        apply(
            &mut model,
            OverlayCmd::SetText {
                text: "今天天气".to_string(),
                kind: TextKind::Partial,
            },
        );
        apply(
            &mut model,
            OverlayCmd::Notice {
                text: "filler skipped".to_string(),
                ttl_ms: 3000,
            },
        );

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
        apply(
            &mut model,
            OverlayCmd::AppendSegment {
                text: "已识别一半".to_string(),
            },
        );
        apply(
            &mut model,
            OverlayCmd::SetText {
                text: "请检查输入设备".to_string(),
                kind: TextKind::Error,
            },
        );
        assert_eq!(model.display_text(), "请检查输入设备");
    }

    #[test]
    fn replace_recent_segments_only_rewrites_tail_session() {
        i18n::init("en-US");
        let mut model = OverlayModel::default();
        apply(
            &mut model,
            OverlayCmd::AppendSegment {
                text: "第一段。".to_string(),
            },
        );
        apply(
            &mut model,
            OverlayCmd::AppendSegment {
                text: "第二".to_string(),
            },
        );
        apply(
            &mut model,
            OverlayCmd::AppendSegment {
                text: "段".to_string(),
            },
        );
        apply(
            &mut model,
            OverlayCmd::ReplaceRecentSegments {
                segments: 2,
                text: "第二段。".to_string(),
            },
        );

        assert_eq!(model.segments, vec!["第一段。", "第二段。"]);
        assert_eq!(model.display_text(), "第一段。第二段。");
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
        apply(
            &mut model,
            OverlayCmd::AppendSegment {
                text: "old".to_string(),
            },
        );
        apply(
            &mut model,
            OverlayCmd::SetText {
                text: "old final".to_string(),
                kind: TextKind::Final,
            },
        );
        apply(&mut model, OverlayCmd::Hide);

        assert_eq!(model.display_text(), "");
        assert_eq!(model.dur_ms, 0);
        assert!(model.notice.is_none());
        assert!(model.error_text.is_empty());
    }

    #[test]
    fn set_state_idle_keeps_overlay_visible() {
        // 多 session 路径：VAD 切到 Idle 子状态时，overlay 仍要可见，
        // 不能跟着 visible=false。可见性只由 Connecting 拉起 / Hide 关闭。
        i18n::init("en-US");
        let mut model = OverlayModel::default();
        apply(
            &mut model,
            OverlayCmd::SetState {
                state: OverlayState::Connecting,
            },
        );
        assert!(model.visible);
        apply(
            &mut model,
            OverlayCmd::SetState {
                state: OverlayState::Recording,
            },
        );
        assert!(model.visible);
        apply(
            &mut model,
            OverlayCmd::SetState {
                state: OverlayState::Idle,
            },
        );
        assert!(model.visible, "Idle 子状态期间 overlay 应保持可见");
        assert_eq!(model.state, OverlayState::Idle);
        apply(&mut model, OverlayCmd::Hide);
        assert!(!model.visible);
    }

    #[test]
    fn connecting_starts_with_empty_text() {
        i18n::init("en-US");
        let mut model = OverlayModel::default();
        apply(
            &mut model,
            OverlayCmd::AppendSegment {
                text: "old".to_string(),
            },
        );
        apply(
            &mut model,
            OverlayCmd::SetState {
                state: OverlayState::Connecting,
            },
        );

        assert_eq!(model.display_text(), "");
        assert!(model.visible);
    }

    // ── TTL / timing 测试 ──

    #[test]
    fn tick_updates_dur_ms_from_recording_started() {
        i18n::init("en-US");
        let mut model = OverlayModel::default();
        apply(
            &mut model,
            OverlayCmd::SetState {
                state: OverlayState::Connecting,
            },
        );
        let started = model.recording_started.expect("Connecting sets clock");
        let now = started + Duration::from_millis(1234);
        model.tick(now, &crate::config::theme::OverlayStateTheme::default());
        assert_eq!(model.dur_ms, 1234);
    }

    #[test]
    fn notice_expires_after_ttl() {
        i18n::init("en-US");
        let mut model = OverlayModel::default();
        apply(
            &mut model,
            OverlayCmd::Notice {
                text: "warn".into(),
                ttl_ms: 1000,
            },
        );
        assert!(model.notice.is_some());
        let until = model.notice_until.expect("notice_until set");
        let theme = crate::config::theme::OverlayStateTheme::default();
        let out = model.tick(until + Duration::from_millis(1), &theme);
        assert_eq!(out, TickOutcome::Idle);
        assert!(model.notice.is_none());
        assert!(model.notice_until.is_none());
    }

    #[test]
    fn hide_during_active_notice_defers_until_notice_expires() {
        i18n::init("en-US");
        let mut model = OverlayModel::default();
        let theme = crate::config::theme::OverlayStateTheme::default();
        apply(
            &mut model,
            OverlayCmd::SetState {
                state: OverlayState::Connecting,
            },
        );
        apply(
            &mut model,
            OverlayCmd::Notice {
                text: "skipped".into(),
                ttl_ms: 500,
            },
        );
        let until = model.notice_until.unwrap();
        apply(&mut model, OverlayCmd::Hide);
        assert!(model.pending_hide, "Hide 设 pending_hide");
        assert!(model.visible, "Hide 时 notice 活着，overlay 仍可见");

        let out = model.tick(until + Duration::from_millis(1), &theme);
        assert_eq!(out, TickOutcome::Hide);
        assert!(!model.visible);
        assert!(!model.pending_hide);
    }

    #[test]
    fn error_text_expires_and_returns_hide() {
        i18n::init("en-US");
        let mut model = OverlayModel::default();
        let theme = crate::config::theme::OverlayStateTheme::default();
        apply(
            &mut model,
            OverlayCmd::SetState {
                state: OverlayState::Connecting,
            },
        );
        apply(
            &mut model,
            OverlayCmd::SetText {
                text: "请检查输入设备".into(),
                kind: TextKind::Error,
            },
        );
        let until = model.error_until.expect("error_until set");
        let out = model.tick(until + Duration::from_millis(1), &theme);
        assert_eq!(out, TickOutcome::Hide);
        assert!(!model.visible);
        assert!(model.error_text.is_empty());
    }

    #[test]
    fn dismiss_skips_notice_deferral() {
        i18n::init("en-US");
        let mut model = OverlayModel::default();
        apply(
            &mut model,
            OverlayCmd::SetState {
                state: OverlayState::Connecting,
            },
        );
        apply(
            &mut model,
            OverlayCmd::Notice {
                text: "warn".into(),
                ttl_ms: 5000,
            },
        );
        apply(&mut model, OverlayCmd::Dismiss);
        assert!(!model.visible);
        assert!(!model.pending_hide);
        assert!(model.notice.is_none());
    }

    #[test]
    fn connecting_resets_all_until_fields() {
        i18n::init("en-US");
        let mut model = OverlayModel::default();
        apply(
            &mut model,
            OverlayCmd::Notice {
                text: "old".into(),
                ttl_ms: 5000,
            },
        );
        apply(
            &mut model,
            OverlayCmd::SetText {
                text: "old err".into(),
                kind: TextKind::Error,
            },
        );
        apply(&mut model, OverlayCmd::Hide);
        assert!(model.pending_hide);

        apply(
            &mut model,
            OverlayCmd::SetState {
                state: OverlayState::Connecting,
            },
        );
        assert!(!model.pending_hide);
        assert!(model.notice_until.is_none());
        assert!(model.error_until.is_none());
        assert!(model.visible);
        assert!(model.recording_started.is_some());
    }
}
