use std::cell::{OnceCell, RefCell};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{NSObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSApplication,
    NSApplicationActivationPolicy, NSApplicationDelegate, NSEvent, NSFont, NSFontWeightBold,
    NSForegroundColorAttributeName, NSGlassEffectView, NSImage, NSImageAlignment,
    NSImageFrameStyle, NSImageScaling, NSImageSymbolConfiguration, NSImageSymbolScale, NSImageView,
    NSLineBreakMode, NSPanel, NSScreen, NSTextAlignment, NSTextField, NSView,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSMutableAttributedString, NSNotification, NSNumber,
    NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString, NSTimer,
};
use objc2_quartz_core::{kCATransitionFade, CABasicAnimation, CAMediaTiming, CATransition};

use crate::config::theme::EffectiveOverlayCfg;
use crate::overlay::layout as L;
use crate::overlay::{OverlayCmd, OverlayModel, OverlayReceiver, OverlayState, TextKind};

use super::chrome::{
    apply_background_settings, apply_glass_settings, apply_panel_background_blur, build_chrome,
    color_from_rgb_alpha, make_panel,
};
use super::icon_fx::IconFx;

fn to_nsrect(f: L::LayoutFrame) -> NSRect {
    NSRect::new(NSPoint::new(f.x, f.y), NSSize::new(f.w, f.h))
}

fn from_nsrect(r: NSRect) -> L::LayoutFrame {
    L::LayoutFrame::new(r.origin.x, r.origin.y, r.size.width, r.size.height)
}

mod typography {
    pub const ICON_SYMBOL: f64 = 18.0;
    pub const STATE: f64 = 15.0;
    pub const META: f64 = 13.0;
    pub const BODY: f64 = 15.0;
}

/// 状态图标的动画曲线。只剩 Idle 符号的缓慢 alpha 呼吸；其余状态的动效都是 icon_fx
/// 的自绘 CALayer，不走这里。
mod anim {
    use std::f64::consts::TAU;

    /// Idle 待命：缓慢轻微的 alpha 呼吸（周期 ~3s，约 0.55–1.0），配合背后的呼吸光晕。
    pub fn idle_breath(ms: f64) -> f64 {
        0.775 + 0.225 * (TAU * ms / 3000.0).sin()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn idle_breath_is_gentle_and_in_range() {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for i in 0..=2000 {
                let v = idle_breath(3000.0 * i as f64 / 2000.0);
                lo = lo.min(v);
                hi = hi.max(v);
            }
            assert!((0.54..0.57).contains(&lo), "lo={lo}");
            assert!((0.99..=1.0).contains(&hi), "hi={hi}");
        }
    }
}

#[derive(Default)]
struct DelegateIvars {
    overlay: OnceCell<RefCell<OverlayView>>,
    rx: OnceCell<RefCell<OverlayReceiver>>,
    cfg: OnceCell<EffectiveOverlayCfg>,
    timer: OnceCell<Retained<NSTimer>>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    struct OverlayDelegate;

    unsafe impl NSObjectProtocol for OverlayDelegate {}

    unsafe impl NSApplicationDelegate for OverlayDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            let mtm = self.mtm();
            let app = NSApplication::sharedApplication(mtm);
            let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

            let cfg = self.ivars().cfg.get().cloned().unwrap_or_default();
            let overlay = OverlayView::new(mtm, cfg);
            self.ivars().overlay.set(RefCell::new(overlay)).ok();

            let timer = unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    0.033,
                    self,
                    sel!(pollOverlay:),
                    None,
                    true,
                )
            };
            self.ivars().timer.set(timer).ok();
        }
    }

    impl OverlayDelegate {
        #[unsafe(method(pollOverlay:))]
        fn poll_overlay(&self, _timer: &NSTimer) {
            let Some(rx) = self.ivars().rx.get() else { return };
            let Some(overlay) = self.ivars().overlay.get() else { return };
            let mut rx = rx.borrow_mut();
            let mut overlay = overlay.borrow_mut();
            while let Ok(cmd) = rx.try_recv() {
                overlay.apply(cmd);
            }
            overlay.tick();
        }
    }
);

impl OverlayDelegate {
    fn new(mtm: MainThreadMarker, rx: OverlayReceiver, cfg: EffectiveOverlayCfg) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars {
            overlay: OnceCell::new(),
            rx: OnceCell::from(RefCell::new(rx)),
            cfg: OnceCell::from(cfg),
            timer: OnceCell::new(),
        });
        unsafe { msg_send![super(this), init] }
    }
}

pub fn run(rx: OverlayReceiver, cfg: EffectiveOverlayCfg) {
    let mtm = MainThreadMarker::new().expect("AppKit must run on main thread");
    let app = NSApplication::sharedApplication(mtm);
    let delegate = OverlayDelegate::new(mtm, rx, cfg);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
}

struct OverlayView {
    mtm: MainThreadMarker,
    cfg: EffectiveOverlayCfg,
    model: OverlayModel,
    panel: Retained<NSPanel>,
    /// root content view。glass/background/labels 都是它的直接子视图。
    container: Retained<NSView>,
    /// `Some` 表示拿到了真正的 `NSGlassEffectView`；`None` 表示走 NSVisualEffectView fallback。
    glass: Option<Retained<NSGlassEffectView>>,
    background: Retained<NSView>,
    state_icon: Retained<NSImageView>,
    status: Retained<NSTextField>,
    stats: Retained<NSTextField>,
    meta: Retained<NSTextField>,
    text: Retained<NSTextField>,
    /// 挂在 state_icon 背后的自绘动效宿主视图；FX layer 都是它的 sublayer。
    icon_fx_view: Retained<NSView>,
    icon_fx: IconFx,
    /// 上一帧渲染的状态，用于检测"进入 Error"以触发一次性抖动。
    last_icon_state: Option<OverlayState>,
    /// panel 当前是否在屏（已 orderFront）。和 `model.visible` 分开：淡出期间 model 已
    /// 不可见但 panel 仍在屏。
    panel_shown: bool,
    /// 淡出动画结束、该真正 `orderOut` 的时刻。
    pending_order_out: Option<Instant>,
    /// 上次 panel 高度，用于检测尺寸变化以触发同步过渡动画。
    last_height: Option<f64>,
    /// 上一帧是否已有 final 文本，用于 final 定稿时做一次淡入。
    last_had_final: bool,
    animation_started: Instant,
    last_text_update: Option<Instant>,
    last_panel_frame: Option<NSRect>,
    last_visible_text: String,
    last_status_text: String,
    last_stats_text: String,
    last_meta_text: String,
    peak_text_lines: usize,
}

impl OverlayView {
    fn new(mtm: MainThreadMarker, cfg: EffectiveOverlayCfg) -> Self {
        let initial_frame = NSRect::new(
            NSPoint::new(80.0, 860.0),
            NSSize::new(L::constants::WIDTH, L::constants::BASE_HEIGHT),
        );
        let panel = make_panel(mtm, initial_frame);
        apply_panel_background_blur(&panel, cfg.macos.background_blur_radius);

        let (container, glass, background, pending_chrome_error) = build_chrome(mtm, &cfg);
        if let Some(error) = &pending_chrome_error {
            tracing::warn!(area = "overlay_chrome", message = %error);
        }

        let row = L::first_row_frames(0.0);
        let icon_fx_view = NSView::new(mtm);
        icon_fx_view.setFrame(to_nsrect(row.icon));
        icon_fx_view.setWantsLayer(true);
        let icon_fx = IconFx::new(icon_fx_view.layer().expect("layer-backed fx view"));
        let state_icon = make_state_icon(mtm, cfg.core.text.primary);
        state_icon.setFrame(to_nsrect(row.icon));
        let status = label(
            mtm,
            to_nsrect(row.status),
            typography::STATE,
            true,
            cfg.core.text.primary,
        );
        let stats = label(
            mtm,
            to_nsrect(row.stats),
            typography::META,
            false,
            cfg.core.text.secondary,
        );
        let meta = label(
            mtm,
            to_nsrect(row.meta),
            typography::META,
            false,
            cfg.core.text.tertiary,
        );
        meta.setAlignment(NSTextAlignment::Right);
        let text = label(
            mtm,
            NSRect::new(
                NSPoint::new(L::constants::H_PAD, L::constants::BOTTOM_PAD),
                NSSize::new(L::constants::BODY_W, L::constants::BODY_LINE_H),
            ),
            typography::BODY,
            false,
            cfg.core.text.primary,
        );
        text.setUsesSingleLineMode(false);
        text.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        // meta 行可能临时承载 notice 文案（warn），长文案安全截断为 "…"。
        meta.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);

        // labels 后 addSubview = z-order 在前。glass 在 build_chrome 里已经先进 container 当底色，
        // 这里追加 labels 自然叠在 glass 上面。
        // icon_fx_view 先加 → z-order 在 state_icon 之后（背后），FX 画在符号后面。
        container.addSubview(&icon_fx_view);
        container.addSubview(&state_icon);
        container.addSubview(&status);
        container.addSubview(&stats);
        container.addSubview(&meta);
        container.addSubview(&text);

        panel.setContentView(Some(&container));
        panel.orderOut(None);

        let model = OverlayModel::new(&cfg.core.state);

        Self {
            mtm,
            cfg,
            model,
            panel,
            container,
            glass,
            background,
            state_icon,
            status,
            stats,
            meta,
            text,
            icon_fx_view,
            icon_fx,
            last_icon_state: None,
            panel_shown: false,
            pending_order_out: None,
            last_height: None,
            last_had_final: false,
            animation_started: Instant::now(),
            last_text_update: None,
            last_panel_frame: None,
            last_visible_text: String::new(),
            last_status_text: String::new(),
            last_stats_text: String::new(),
            last_meta_text: String::new(),
            peak_text_lines: 1,
        }
    }

    fn apply(&mut self, cmd: OverlayCmd) {
        if matches!(cmd, OverlayCmd::Quit) {
            NSApplication::sharedApplication(self.mtm).terminate(None);
            return;
        }
        if let OverlayCmd::ReloadConfig { cfg } = cmd {
            self.rebuild_chrome(cfg);
            return;
        }
        if matches!(cmd, OverlayCmd::Relabel) {
            // Force render() to push new translated status text.
            self.last_status_text.clear();
            self.model.apply(cmd, &self.cfg.core.state);
            self.render();
            return;
        }
        // View-only pre-processing（model.apply 不知道的事）.
        match &cmd {
            OverlayCmd::SetState {
                state: OverlayState::Connecting,
            } => {
                self.clear_rendered_session();
            }
            OverlayCmd::SetState {
                state: OverlayState::Recording,
            } => {
                self.last_text_update = Some(Instant::now());
            }
            OverlayCmd::SetText {
                kind: TextKind::Partial,
                ..
            }
            | OverlayCmd::AppendSegment { .. } => {
                self.last_text_update = Some(Instant::now());
            }
            _ => {}
        }
        let prev_visible = self.model.visible;
        self.model.apply(cmd, &self.cfg.core.state);
        if prev_visible && !self.model.visible {
            self.last_text_update = None;
            self.peak_text_lines = 1;
            self.clear_rendered_session();
        }
        self.render();
    }

    fn tick(&mut self) {
        let prev_visible = self.model.visible;
        let _ = self.model.tick(Instant::now(), &self.cfg.core.state);
        if prev_visible && !self.model.visible {
            self.last_text_update = None;
            self.peak_text_lines = 1;
            self.clear_rendered_session();
        }
        self.render();
    }

    fn render(&mut self) {
        // 淡出动画结束且仍不可见 → 真正下屏。若期间又变可见，留给下面的可见分支取走
        // pending 并淡回不透明（避免清了 pending 却没恢复 alpha 的竞态）。
        if let Some(at) = self.pending_order_out {
            if Instant::now() >= at && !self.model.visible {
                self.panel.orderOut(None);
                self.panel_shown = false;
                self.pending_order_out = None;
                self.last_panel_frame = None;
                self.last_height = None;
            }
        }

        if self.model.visible {
            let full_text = self.model.display_text();
            let (_, current_lines) = L::display_text_plan(
                &full_text,
                self.cfg.core.max_text_lines,
                L::constants::CHARS_PER_LINE,
            );
            let lines = if self.model.state == OverlayState::Recording {
                self.peak_text_lines = self.peak_text_lines.max(current_lines);
                self.peak_text_lines
            } else {
                current_lines
            };
            let height = L::constants::BASE_HEIGHT
                + (lines.saturating_sub(1) as f64 * L::constants::BODY_LINE_H);
            let height_changed = self.last_height != Some(height);
            self.last_height = Some(height);

            if !self.panel_shown {
                // 首次出现：先就位，再 alpha 0→1 淡入。
                self.pending_order_out = None;
                self.layout(height, lines, false);
                self.place(height, false);
                self.panel.setAlphaValue(0.0);
                self.panel.makeKeyAndOrderFront(None);
                fade_window_alpha(&self.panel, 1.0);
                self.panel_shown = true;
            } else {
                // 淡出途中又要显示 → 取消下屏、淡回不透明。
                if self.pending_order_out.take().is_some() {
                    fade_window_alpha(&self.panel, 1.0);
                }
                // 高度变化时，窗口与内容用同一动画组同步过渡，避免文字瞬移。
                if height_changed {
                    NSAnimationContext::beginGrouping();
                    NSAnimationContext::currentContext().setDuration(RESIZE_ANIM);
                    self.layout(height, lines, true);
                    self.place(height, true);
                    NSAnimationContext::endGrouping();
                } else {
                    self.layout(height, lines, false);
                    self.place(height, true);
                }
            }
        } else {
            // 隐藏：先淡出，淡完再 orderOut（上面的 pending 逻辑收尾）。
            if self.panel_shown && self.pending_order_out.is_none() {
                fade_window_alpha(&self.panel, 0.0);
                self.pending_order_out =
                    Some(Instant::now() + Duration::from_secs_f64(APPEAR_FADE));
            }
            return;
        }

        let full_text = self.model.display_text();
        let live_plan = if self.model.error_text.is_empty() && self.model.final_text.is_empty() {
            L::live_text_plan(
                &self.model.segments,
                &self.model.partial,
                self.cfg.core.max_text_lines,
                L::constants::CHARS_PER_LINE,
            )
        } else {
            let (display_text, lines) = L::display_text_plan(
                &full_text,
                self.cfg.core.max_text_lines,
                L::constants::CHARS_PER_LINE,
            );
            L::LiveTextPlan {
                segments: String::new(),
                partial: display_text,
                lines,
            }
        };
        let display_text = live_plan.full_text();
        let (state, label, color_rgb) = self.effective_state();
        let dur = L::format_duration(self.model.dur_ms);
        let app = self
            .model
            .app_name
            .as_deref()
            .or(self.model.bundle_id.as_deref())
            .unwrap_or("");
        let words_text = crate::t!("overlay.word_count", n = self.model.words);
        let header = L::header_parts(&label, &dur, &words_text, app, &self.model.chain_summary);
        self.render_state_icon(state, color_rgb);
        if self.last_status_text != header.state {
            fade_view(&self.status, 0.16);
            self.status
                .setStringValue(&NSString::from_str(&header.state));
            self.status
                .setTextColor(Some(&color_from_rgb_alpha(color_rgb, 1.0)));
            self.last_status_text = header.state;
        }

        let stats_text = L::stats_text(&header.duration, &header.words, &header.app);
        if self.last_stats_text != stats_text {
            self.stats.setStringValue(&NSString::from_str(&stats_text));
            self.last_stats_text = stats_text;
        }
        self.stats.setTextColor(Some(&color_from_rgb_alpha(
            self.cfg.core.text.secondary,
            1.0,
        )));

        // meta 行：notice 活跃时盖住 chain_summary，黄字。
        let (meta_text, meta_color) = if let Some(notice) = &self.model.notice {
            (notice.text.clone(), self.cfg.core.text.notice)
        } else {
            (header.meta, self.cfg.core.text.tertiary)
        };
        if self.last_meta_text != meta_text {
            fade_view(&self.meta, 0.16);
            self.meta.setStringValue(&NSString::from_str(&meta_text));
            self.last_meta_text = meta_text;
        }
        self.meta
            .setTextColor(Some(&color_from_rgb_alpha(meta_color, 1.0)));

        // text 区：error_text 非空时强制红字，覆盖 partial/final（display_text 已优先返回 error）。
        let text_color = if !self.model.error_text.is_empty() {
            self.cfg.core.text.error
        } else {
            self.cfg.core.text.primary
        };
        // final 定稿（empty→非空）时做一次轻淡入；live partial 不淡，保持 crisp。
        let final_appeared = !self.model.final_text.is_empty() && !self.last_had_final;
        self.last_had_final = !self.model.final_text.is_empty();
        if self.last_visible_text != display_text {
            if !self.model.error_text.is_empty() {
                fade_view(&self.text, 0.10);
            } else if final_appeared {
                fade_view(&self.text, 0.18);
            }
            self.render_body_text(&live_plan, text_color);
            self.last_visible_text = display_text;
        }
        self.text
            .setTextColor(Some(&color_from_rgb_alpha(text_color, 1.0)));
    }

    fn effective_state(&self) -> (OverlayState, String, u32) {
        (
            self.model.state,
            self.model.state_label.clone(),
            self.model.state_color,
        )
    }

    fn render_state_icon(&mut self, state: OverlayState, color_rgb: u32) {
        // 背后的自绘 FX（光晕/雷达/跳点/彗星尾/电平条）。返回 true 表示效果独占图标位、隐藏符号。
        let bounds = self.state_icon.bounds();
        let hide_symbol = self
            .icon_fx
            .render(bounds, state, color_rgb, self.model.level);

        let entering = self.last_icon_state != Some(state);
        self.last_icon_state = Some(state);
        // 进入 Error 时抖一下符号（一次性）。
        if state == OverlayState::Error && entering {
            self.trigger_shake();
        }

        if hide_symbol {
            self.state_icon.setImage(None);
            return;
        }

        // 符号（重新）出现时淡入，和背后 FX 的淡入一起形成交叉淡入。
        if entering {
            fade_view(&self.state_icon, 0.18);
        }

        let ms = self.animation_started.elapsed().as_millis() as f64;
        let symbol = state_symbol(state);
        // (variableValue, 是否变量渲染, alpha)。Idle 用 alpha 呼吸；Connecting/Error 静态符号
        // （动感交给背后的 FX）。Recording/Thinking/Stopping 已被 FX 盖住、不画符号。
        let (value, variable, alpha) = match state {
            OverlayState::Idle => (1.0, false, anim::idle_breath(ms)),
            OverlayState::Connecting | OverlayState::Error => (1.0, false, 1.0),
            OverlayState::Recording | OverlayState::Thinking | OverlayState::Stopping => {
                unreachable!("hidden by icon_fx above")
            }
        };
        if let Some(image) = symbol_image(symbol, value, variable) {
            self.state_icon.setImage(Some(&image));
        }
        self.state_icon.setAlphaValue(alpha);
        let bold = unsafe { NSFontWeightBold };
        let config = NSImageSymbolConfiguration::configurationWithPointSize_weight_scale(
            typography::ICON_SYMBOL,
            bold,
            NSImageSymbolScale::Large,
        );
        self.state_icon.setSymbolConfiguration(Some(&config));
        self.state_icon
            .setImageAlignment(NSImageAlignment::AlignCenter);
        self.state_icon
            .setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        self.state_icon.setImageFrameStyle(NSImageFrameStyle::None);
        self.state_icon
            .setContentTintColor(Some(&color_from_rgb_alpha(color_rgb, 1.0)));
    }

    /// 进入 Error 时让符号左右快速抖一下再定住（一次性，不循环；removedOnCompletion 默认 true）。
    fn trigger_shake(&self) {
        let Some(layer) = self.state_icon.layer() else {
            return;
        };
        let shake =
            CABasicAnimation::animationWithKeyPath(Some(ns_string!("transform.translation.x")));
        unsafe {
            shake.setFromValue(Some(&NSNumber::numberWithDouble(-4.0)));
            shake.setToValue(Some(&NSNumber::numberWithDouble(4.0)));
        }
        shake.setDuration(0.05);
        shake.setAutoreverses(true);
        shake.setRepeatCount(3.0);
        layer.addAnimation_forKey(&shake, Some(ns_string!("shake")));
    }

    fn render_body_text(&self, plan: &L::LiveTextPlan, fallback_color: u32) {
        if plan.segments.is_empty() {
            self.text.setStringValue(&NSString::from_str(&plan.partial));
            self.text
                .setTextColor(Some(&color_from_rgb_alpha(fallback_color, 1.0)));
            return;
        }

        let full = plan.full_text();
        let attributed = NSMutableAttributedString::from_nsstring(&NSString::from_str(&full));
        let segment_len = L::utf16_len(&plan.segments);
        let full_len = L::utf16_len(&full);
        let segment_color = color_from_rgb_alpha(self.cfg.core.text.segment, 0.88);
        let partial_color = color_from_rgb_alpha(self.cfg.core.text.primary, 1.0);
        unsafe {
            let _: () = msg_send![
                &attributed,
                addAttribute: NSForegroundColorAttributeName,
                value: &*segment_color,
                range: NSRange::new(0, segment_len)
            ];
            if full_len > segment_len {
                let _: () = msg_send![
                    &attributed,
                    addAttribute: NSForegroundColorAttributeName,
                    value: &*partial_color,
                    range: NSRange::new(segment_len, full_len - segment_len)
                ];
            }
            let _: () = msg_send![&self.text, setAttributedStringValue: &*attributed];
        }
    }

    fn clear_rendered_session(&mut self) {
        self.text.setStringValue(&NSString::from_str(""));
        self.stats.setStringValue(&NSString::from_str(""));
        self.meta.setStringValue(&NSString::from_str(""));
        self.last_visible_text.clear();
        self.last_stats_text.clear();
        self.last_meta_text.clear();
        // 隐藏后重置，下次重新进入 Error 能再触发抖动 / final 能再淡入。
        self.last_icon_state = None;
        self.last_had_final = false;
    }

    fn layout(&mut self, height: f64, lines: usize, animated: bool) {
        let top_offset = height - L::constants::BASE_HEIGHT;
        let full = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(L::constants::WIDTH, height),
        );
        // 材质层和色层必须跟 root 同步动画，避免 Liquid Glass 边缘与实际外框短暂错位。
        set_view_frame(&self.container, full, animated);
        if let Some(glass) = &self.glass {
            set_view_frame(glass, full, animated);
        }
        set_view_frame(&self.background, full, animated);
        let row = L::first_row_frames(top_offset);
        set_view_frame(&self.state_icon, to_nsrect(row.icon), animated);
        set_view_frame(&self.icon_fx_view, to_nsrect(row.icon), animated);
        set_view_frame(&self.status, to_nsrect(row.status), animated);
        set_view_frame(&self.stats, to_nsrect(row.stats), animated);
        set_view_frame(&self.meta, to_nsrect(row.meta), animated);
        set_view_frame(
            &self.text,
            NSRect::new(
                NSPoint::new(L::constants::H_PAD, L::constants::BOTTOM_PAD),
                NSSize::new(
                    L::constants::BODY_W,
                    L::constants::BODY_LINE_H
                        + (lines.saturating_sub(1) as f64 * L::constants::BODY_LINE_H),
                ),
            ),
            animated,
        );
    }

    fn place(&mut self, height: f64, animated: bool) {
        let screens = screen_frames(self.mtm);
        let fallback = fallback_screen(self.mtm, &screens);
        let (anchor, screen) =
            crate::platform::macos::window::focused_window_frame_for_screens(&screens)
                .unwrap_or((fallback, fallback));
        let frame = to_nsrect(L::panel_frame(
            from_nsrect(anchor),
            self.cfg.core.position,
            L::constants::WIDTH,
            height,
            from_nsrect(screen),
        ));
        if self
            .last_panel_frame
            .is_none_or(|last| !L::frame_nearly_eq(from_nsrect(last), from_nsrect(frame)))
        {
            if animated && self.last_panel_frame.is_some() {
                self.panel.animator().setFrame_display(frame, true);
            } else {
                self.panel.setFrame_display_animate(frame, true, false);
            }
            self.last_panel_frame = Some(frame);
        }
    }

    /// 热重载入口：原地调 setter，不换 view。fallback 路径下 SPI 不可用，整段 noop（已经在 init 时报过错）。
    fn rebuild_chrome(&mut self, cfg: EffectiveOverlayCfg) {
        if cfg == self.cfg {
            return;
        }
        if let Some(glass) = &self.glass {
            let missing = apply_glass_settings(glass, &cfg);
            if !missing.is_empty() {
                tracing::warn!(
                    area = "overlay_chrome",
                    missing = %missing.join(", "),
                    "glass SPI unavailable"
                );
            }
        }
        apply_panel_background_blur(&self.panel, cfg.macos.background_blur_radius);
        apply_background_settings(&self.background, &cfg);
        self.cfg = cfg;
        self.model.state_color = self.model.state.color_rgb(&self.cfg.core.state);
        self.last_status_text.clear();
        self.last_visible_text.clear();
        self.last_meta_text.clear();
        // peak_text_lines 用旧上限可能比新 max_text_lines 大，clamp 回去
        let cap = self.cfg.core.max_text_lines.clamp(1, 8);
        if self.peak_text_lines > cap {
            self.peak_text_lines = cap;
        }
        self.render();
    }
}

fn screen_frames(mtm: MainThreadMarker) -> Vec<NSRect> {
    NSScreen::screens(mtm)
        .iter()
        .map(|screen| screen.frame())
        .collect()
}

fn fallback_screen(mtm: MainThreadMarker, screens: &[NSRect]) -> NSRect {
    let mouse = NSEvent::mouseLocation();
    screens
        .iter()
        .copied()
        .find(|screen| point_in_rect(mouse, *screen))
        .or_else(|| NSScreen::mainScreen(mtm).map(|screen| screen.frame()))
        .unwrap_or_else(|| NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0)))
}

fn point_in_rect(point: NSPoint, rect: NSRect) -> bool {
    point.x >= rect.origin.x
        && point.x <= rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y <= rect.origin.y + rect.size.height
}

fn label(
    mtm: MainThreadMarker,
    frame: NSRect,
    font_size: f64,
    bold: bool,
    color_rgb: u32,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(ns_string!(""), mtm);
    field.setFrame(frame);
    field.setDrawsBackground(false);
    field.setBezeled(false);
    field.setEditable(false);
    field.setSelectable(false);
    field.setWantsLayer(true);
    field.setTextColor(Some(&color_from_rgb_alpha(color_rgb, 1.0)));
    let font = if bold {
        NSFont::boldSystemFontOfSize(font_size)
    } else {
        NSFont::systemFontOfSize(font_size)
    };
    field.setFont(Some(&font));
    field.setAlignment(NSTextAlignment::Left);
    field
}

fn make_state_icon(mtm: MainThreadMarker, color_rgb: u32) -> Retained<NSImageView> {
    let image = symbol_image("circle.fill", 1.0, false).expect("system symbol should exist");
    let view = NSImageView::imageViewWithImage(&image, mtm);
    // 旋转弧（Stopping）需要 state_icon 有 backing layer 才能挂 sublayer。
    view.setWantsLayer(true);
    let bold = unsafe { NSFontWeightBold };
    let config = NSImageSymbolConfiguration::configurationWithPointSize_weight_scale(
        typography::ICON_SYMBOL,
        bold,
        NSImageSymbolScale::Large,
    );
    view.setSymbolConfiguration(Some(&config));
    view.setImageAlignment(NSImageAlignment::AlignCenter);
    view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    view.setImageFrameStyle(NSImageFrameStyle::None);
    view.setContentTintColor(Some(&color_from_rgb_alpha(color_rgb, 1.0)));
    view
}

fn symbol_image(name: &str, value: f64, variable: bool) -> Option<Retained<NSImage>> {
    let name = NSString::from_str(name);
    let desc = NSString::from_str("state");
    if variable {
        return NSImage::imageWithSystemSymbolName_variableValue_accessibilityDescription(
            &name,
            value.clamp(0.0, 1.0),
            Some(&desc),
        );
    }
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&name, Some(&desc))
}

fn state_symbol(state: OverlayState) -> &'static str {
    match state {
        OverlayState::Idle => "waveform.circle",
        OverlayState::Connecting => "antenna.radiowaves.left.and.right",
        OverlayState::Recording => "waveform",
        OverlayState::Thinking => "sparkles",
        OverlayState::Stopping => "stop.circle",
        OverlayState::Error => "exclamationmark.triangle.fill",
    }
}

/// overlay 淡入/淡出时长。
const APPEAR_FADE: f64 = 0.14;
/// 尺寸变化时窗口+内容同步过渡时长。
const RESIZE_ANIM: f64 = 0.2;

/// 在 Core Animation 过渡里把 panel alpha 动到目标值（show/hide 淡变）。
fn fade_window_alpha(panel: &NSPanel, target: f64) {
    NSAnimationContext::beginGrouping();
    NSAnimationContext::currentContext().setDuration(APPEAR_FADE);
    panel.animator().setAlphaValue(target);
    NSAnimationContext::endGrouping();
}

/// 设置视图 frame；`animated` 时走 `animator()` 代理，由外层 NSAnimationContext 决定时长。
fn set_view_frame(view: &NSView, frame: NSRect, animated: bool) {
    if animated {
        view.animator().setFrame(frame);
    } else {
        view.setFrame(frame);
    }
}

fn fade_view(view: &NSView, duration_s: f64) {
    if let Some(layer) = view.layer() {
        let transition = CATransition::animation();
        transition.setDuration(duration_s);
        transition.setType(unsafe { kCATransitionFade });
        layer.addAnimation_forKey(&transition, Some(ns_string!("fade")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_symbols_match_overlay_semantics() {
        assert_eq!(state_symbol(OverlayState::Idle), "waveform.circle");
        assert_eq!(
            state_symbol(OverlayState::Connecting),
            "antenna.radiowaves.left.and.right"
        );
        assert_eq!(state_symbol(OverlayState::Recording), "waveform");
        assert_eq!(state_symbol(OverlayState::Thinking), "sparkles");
        assert_eq!(state_symbol(OverlayState::Stopping), "stop.circle");
        assert_eq!(
            state_symbol(OverlayState::Error),
            "exclamationmark.triangle.fill"
        );
    }

    #[test]
    fn meta_typography_is_readable_but_secondary() {
        const {
            assert!(typography::META >= 13.0);
            assert!(typography::META < typography::STATE);
        }
        assert_eq!(
            crate::config::theme::OverlayTextTheme::default().tertiary,
            crate::config::theme::palette::FG3
        );
    }

    #[test]
    fn segment_text_uses_readable_secondary_color() {
        assert_eq!(
            crate::config::theme::OverlayTextTheme::default().segment,
            crate::config::theme::palette::FG1
        );
    }
}
