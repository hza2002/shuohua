use std::cell::{OnceCell, RefCell};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{NSObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSFont, NSFontWeightBold,
    NSForegroundColorAttributeName, NSGlassEffectView, NSImage, NSImageAlignment,
    NSImageFrameStyle, NSImageScaling, NSImageSymbolConfiguration, NSImageSymbolScale, NSImageView,
    NSLineBreakMode, NSPanel, NSScreen, NSTextAlignment, NSTextField, NSView,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSMutableAttributedString, NSNotification, NSObjectProtocol,
    NSPoint, NSRange, NSRect, NSSize, NSString, NSTimer,
};
use objc2_quartz_core::{kCATransitionFade, CAMediaTiming, CATransition};
use tokio::sync::mpsc;

use crate::config::theme::EffectiveOverlayCfg;
use crate::overlay::layout as L;
use crate::overlay::{OverlayCmd, OverlayModel, OverlayState, TextKind};

use super::chrome::{
    apply_background_settings, apply_glass_settings, apply_panel_background_blur, build_chrome,
    color_from_rgb_alpha, make_panel,
};

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

#[derive(Default)]
struct DelegateIvars {
    overlay: OnceCell<RefCell<OverlayView>>,
    rx: OnceCell<RefCell<mpsc::UnboundedReceiver<OverlayCmd>>>,
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
    fn new(
        mtm: MainThreadMarker,
        rx: mpsc::UnboundedReceiver<OverlayCmd>,
        cfg: EffectiveOverlayCfg,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars {
            overlay: OnceCell::new(),
            rx: OnceCell::from(RefCell::new(rx)),
            cfg: OnceCell::from(cfg),
            timer: OnceCell::new(),
        });
        unsafe { msg_send![super(this), init] }
    }
}

pub fn run(rx: mpsc::UnboundedReceiver<OverlayCmd>, cfg: EffectiveOverlayCfg) {
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
    /// 装文字 / icon 的容器：glass 路径下 = `glass.contentView`；fallback 路径下 = 直接挂在 panel 的 visualEffect。
    container: Retained<NSView>,
    /// `Some` 表示拿到了真正的 `NSGlassEffectView`；`None` 表示走 NSVisualEffectView fallback。
    glass: Option<Retained<NSGlassEffectView>>,
    background: Retained<NSView>,
    state_icon: Retained<NSImageView>,
    status: Retained<NSTextField>,
    stats: Retained<NSTextField>,
    meta: Retained<NSTextField>,
    text: Retained<NSTextField>,
    animation_started: Instant,
    last_text_update: Option<Instant>,
    last_panel_frame: Option<NSRect>,
    last_visible_text: String,
    last_status_text: String,
    last_stats_text: String,
    last_meta_text: String,
    peak_text_lines: usize,
    /// chrome 初始化 / 热重载时检测到不可用的 SPI / fallback 走人 → 等首次可见时塞 chrome error。
    pending_chrome_error: Option<String>,
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

        let row = L::first_row_frames(0.0);
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
            animation_started: Instant::now(),
            last_text_update: None,
            last_panel_frame: None,
            last_visible_text: String::new(),
            last_status_text: String::new(),
            last_stats_text: String::new(),
            last_meta_text: String::new(),
            peak_text_lines: 1,
            pending_chrome_error,
        }
    }

    fn apply(&mut self, cmd: OverlayCmd) {
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
        if self.model.visible {
            // 首次可见时，把 chrome 初始化 / 热重载攒下的错误塞进 error 文本区。
            if self.model.error_text.is_empty() && self.model.error_until.is_none() {
                if let Some(err) = self.pending_chrome_error.take() {
                    self.model.apply(
                        OverlayCmd::SetText {
                            text: err,
                            kind: TextKind::Error,
                        },
                        &self.cfg.core.state,
                    );
                }
            }

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
            self.layout(height, lines);
            self.place(height);
            if self.panel.alphaValue() < 1.0 {
                self.panel.setAlphaValue(1.0);
            }
            self.panel.makeKeyAndOrderFront(None);
        } else {
            self.panel.setAlphaValue(0.0);
            self.panel.orderOut(None);
            self.last_panel_frame = None;
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
        // 用 text_stats 算 words：CJK 按字、英文按词，跨语言一致。
        // 文案模板 {n} 字 / {n} words 由 i18n 选；底层数 = words。
        let words = crate::text_stats::compute(&full_text).words as u32;
        let words_text = crate::t!("overlay.word_count", n = words);
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
        self.stats
            .setTextColor(Some(&color_from_rgb_alpha(self.cfg.core.text.secondary, 1.0)));

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
        if self.last_visible_text != display_text {
            if !self.model.error_text.is_empty() {
                fade_view(&self.text, 0.10);
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

    fn render_state_icon(&self, state: OverlayState, color_rgb: u32) {
        let symbol = state_symbol(state);
        let phase = if state == OverlayState::Recording {
            self.model
                .recording_started
                .map(|started| ((started.elapsed().as_millis() / 160) % 6) as usize)
                .unwrap_or(0)
        } else {
            0
        };
        let value = if state == OverlayState::Recording {
            [0.25, 0.45, 0.75, 1.0, 0.65, 0.35][phase]
        } else if matches!(
            state,
            OverlayState::Connecting | OverlayState::Thinking | OverlayState::Stopping
        ) {
            let ms = self.animation_started.elapsed().as_millis() as f64;
            0.78 + ((ms / 360.0).sin() + 1.0) * 0.11
        } else {
            1.0
        };
        let variable = matches!(
            state,
            OverlayState::Recording
                | OverlayState::Connecting
                | OverlayState::Thinking
                | OverlayState::Stopping
        );
        if let Some(image) = symbol_image(symbol, value, variable) {
            self.state_icon.setImage(Some(&image));
        }
        self.state_icon.setAlphaValue(value.clamp(0.78, 1.0));
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
    }

    fn layout(&mut self, height: f64, lines: usize) {
        let top_offset = height - L::constants::BASE_HEIGHT;
        let full = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(L::constants::WIDTH, height),
        );
        // panel.contentView (glass 或 fallback) 跟随 panel 自动 resize；container 显式 set 确保 labels 坐标系对齐
        self.container.setFrame(full);
        let row = L::first_row_frames(top_offset);
        self.state_icon.setFrame(to_nsrect(row.icon));
        self.status.setFrame(to_nsrect(row.status));
        self.stats.setFrame(to_nsrect(row.stats));
        self.meta.setFrame(to_nsrect(row.meta));
        self.text.setFrame(NSRect::new(
            NSPoint::new(L::constants::H_PAD, L::constants::BOTTOM_PAD),
            NSSize::new(
                L::constants::BODY_W,
                L::constants::BODY_LINE_H
                    + (lines.saturating_sub(1) as f64 * L::constants::BODY_LINE_H),
            ),
        ));
    }

    fn place(&mut self, height: f64) {
        let screen = NSScreen::mainScreen(self.mtm)
            .map(|screen| screen.frame())
            .unwrap_or_else(|| NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0)));
        let anchor = crate::focused_window_darwin::focused_window_frame(screen.size.height)
            .unwrap_or(screen);
        let frame = to_nsrect(L::panel_frame(
            from_nsrect(anchor),
            self.cfg.core.position,
            L::constants::WIDTH,
            height,
            from_nsrect(screen),
        ));
        let animate = self.last_panel_frame.is_some();
        if self
            .last_panel_frame
            .is_none_or(|last| !L::frame_nearly_eq(from_nsrect(last), from_nsrect(frame)))
        {
            self.panel.setFrame_display_animate(frame, true, animate);
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
                self.pending_chrome_error =
                    Some(format!("glass SPI unavailable: {}", missing.join(", ")));
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
