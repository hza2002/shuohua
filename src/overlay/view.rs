use std::cell::{OnceCell, RefCell};
use std::ffi::{c_char, c_int, c_void};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSAutoresizingMaskOptions,
    NSBackingStoreType, NSColor, NSFont, NSFontWeightBold, NSForegroundColorAttributeName,
    NSGlassEffectView, NSGlassEffectViewStyle, NSImage, NSImageAlignment, NSImageFrameStyle,
    NSImageScaling, NSImageSymbolConfiguration, NSImageSymbolScale, NSImageView, NSLineBreakMode,
    NSPanel, NSScreen, NSStatusWindowLevel, NSTextAlignment, NSTextField, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSMutableAttributedString, NSNotification, NSObjectProtocol,
    NSPoint, NSRange, NSRect, NSSize, NSString, NSTimer,
};
use objc2_quartz_core::{kCATransitionFade, CAMediaTiming, CATransition};
use tokio::sync::mpsc;

use crate::config::theme::{EffectiveOverlayCfg, GlassStyle};
use crate::config::OverlayPosition;
use crate::overlay::{OverlayCmd, OverlayModel, OverlayState, TextKind};

/// SetText{Error} 后多久自动 hide overlay。比 NOTICE_TTL_MS 长，因为 error 文案
/// 用户需要读完并决定是否重试；notice 只是顺手提示，过去就过去。仍然不挂太久，
/// shuo 是键盘工具，5s 后还没看就靠 ESC 自己关。
const ERROR_TTL_MS: u64 = 5000;

mod layout {
    pub const WIDTH: f64 = 572.0;
    pub const BASE_HEIGHT: f64 = 64.0;
    pub const WINDOW_MARGIN: f64 = 16.0;
    pub const H_PAD: f64 = 16.0;
    pub const BOTTOM_PAD: f64 = 7.0;
    pub const HEADER_BODY_GAP: f64 = 2.0;
    pub const BODY_LINE_H: f64 = 21.0;
    pub const BODY_W: f64 = WIDTH - H_PAD * 2.0;
    pub const CHARS_PER_LINE: usize = 38;
    pub const HEADER_CENTER_Y: f64 = BOTTOM_PAD + BODY_LINE_H + HEADER_BODY_GAP + 12.0;
    pub const ICON_BOX: f64 = 24.0;
    pub const STATE_BOX_H: f64 = 20.0;
    pub const META_BOX_H: f64 = 18.0;
    pub const ICON_OPTICAL_Y: f64 = -0.5;
    pub const STATE_OPTICAL_Y: f64 = 0.0;
    pub const META_OPTICAL_Y: f64 = 0.0;
    pub const ICON_STATE_GAP: f64 = 5.0;
    pub const STATE_W: f64 = 56.0;
    pub const STATE_STATS_GAP: f64 = 5.0;
    pub const STATS_W: f64 = 220.0;
    pub const META_GAP: f64 = 8.0;
    pub const META_MIN_W: f64 = 180.0;
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
    recording_started: Option<Instant>,
    last_text_update: Option<Instant>,
    /// notice（meta 行 warn）到期点：到点 tick 把 meta 恢复成 chain_summary。
    notice_until: Option<Instant>,
    /// error 文本（text 区红字）到期点：到点 tick 自动 hide overlay。
    error_until: Option<Instant>,
    /// Hide 命令到达时若 notice 还活着就延期；标记 true，待 notice 到期再真隐藏。
    pending_hide: bool,
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
            NSSize::new(layout::WIDTH, layout::BASE_HEIGHT),
        );
        let panel = make_panel(mtm, initial_frame);
        apply_panel_background_blur(&panel, cfg.background_blur_radius);

        let (container, glass, background, pending_chrome_error) = build_chrome(mtm, &cfg);

        let row = first_row_frames(0.0);
        let state_icon = make_state_icon(mtm, cfg.text.primary);
        state_icon.setFrame(row.icon);
        let status = label(mtm, row.status, typography::STATE, true, cfg.text.primary);
        let stats = label(mtm, row.stats, typography::META, false, cfg.text.secondary);
        let meta = label(mtm, row.meta, typography::META, false, cfg.text.tertiary);
        meta.setAlignment(NSTextAlignment::Right);
        let text = label(
            mtm,
            NSRect::new(
                NSPoint::new(layout::H_PAD, layout::BOTTOM_PAD),
                NSSize::new(layout::BODY_W, layout::BODY_LINE_H),
            ),
            typography::BODY,
            false,
            cfg.text.primary,
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

        let model = OverlayModel::new(&cfg.state);

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
            recording_started: None,
            last_text_update: None,
            notice_until: None,
            error_until: None,
            pending_hide: false,
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
            self.model.apply(cmd, &self.cfg.state);
            self.render();
            return;
        }
        match &cmd {
            OverlayCmd::SetState { state } => match state {
                OverlayState::Recording => {
                    // 多 session 路径上每次 resume 都会回到 Recording。
                    // 时钟只在录音首次起跳时归零，后续 resume 不能让它跳回 0。
                    if self.recording_started.is_none() {
                        self.recording_started = Some(Instant::now());
                    }
                    self.last_text_update = Some(Instant::now());
                }
                OverlayState::Idle => {
                    // 多 session 路径上 `Idle` 表示"当前没 ASR，麦克风还在听"。
                    // 不清时钟、不清 segments — Hide / Dismiss 会负责真正收尾。
                }
                OverlayState::Connecting => {
                    // 新 session 接管：抢断旧 session 留下的 lingering 状态。
                    self.notice_until = None;
                    self.error_until = None;
                    self.pending_hide = false;
                    self.clear_rendered_session();
                }
                _ => {}
            },
            OverlayCmd::SetText { kind, .. } => match kind {
                TextKind::Partial => {
                    self.last_text_update = Some(Instant::now());
                }
                TextKind::Error => {
                    self.error_until = Some(Instant::now() + Duration::from_millis(ERROR_TTL_MS));
                }
                TextKind::Final => {}
            },
            OverlayCmd::AppendSegment { .. } => {
                self.last_text_update = Some(Instant::now());
            }
            OverlayCmd::Notice { ttl_ms, .. } => {
                self.notice_until = Some(Instant::now() + Duration::from_millis(*ttl_ms as u64));
            }
            OverlayCmd::Hide => {
                // notice 还活着就延期，等 tick 到点真隐藏，避免 warn 一闪就没。
                if self
                    .notice_until
                    .is_some_and(|until| Instant::now() < until)
                {
                    self.pending_hide = true;
                    return;
                }
                self.recording_started = None;
                self.last_text_update = None;
                self.notice_until = None;
                self.error_until = None;
                self.pending_hide = false;
                self.peak_text_lines = 1;
                self.clear_rendered_session();
            }
            OverlayCmd::Dismiss => {
                // ESC 强制关，绕过 notice / error 延期。
                self.recording_started = None;
                self.last_text_update = None;
                self.notice_until = None;
                self.error_until = None;
                self.pending_hide = false;
                self.peak_text_lines = 1;
                self.clear_rendered_session();
            }
            _ => {}
        }
        self.model.apply(cmd, &self.cfg.state);
        self.render();
    }

    fn tick(&mut self) {
        if let Some(started) = self.recording_started {
            self.model.dur_ms = started.elapsed().as_millis() as u64;
        }
        let now = Instant::now();
        if self.notice_until.is_some_and(|until| now >= until) {
            self.model.notice = None;
            self.notice_until = None;
            if self.pending_hide {
                // 等到的就是这一刻——notice 到期，把延期的 Hide 真正执行。
                self.model.apply(OverlayCmd::Hide, &self.cfg.state);
                self.recording_started = None;
                self.last_text_update = None;
                self.pending_hide = false;
                self.peak_text_lines = 1;
                self.clear_rendered_session();
            }
        }
        if self.error_until.is_some_and(|until| now >= until) {
            // error 文本到期：自动关 overlay。
            self.error_until = None;
            self.model.apply(OverlayCmd::Hide, &self.cfg.state);
            self.recording_started = None;
            self.last_text_update = None;
            self.notice_until = None;
            self.pending_hide = false;
            self.peak_text_lines = 1;
            self.clear_rendered_session();
        }
        self.render();
    }

    fn render(&mut self) {
        if self.model.visible {
            // 首次可见时，把 chrome 初始化 / 热重载攒下的错误塞进 error 文本区。
            if self.model.error_text.is_empty() && self.error_until.is_none() {
                if let Some(err) = self.pending_chrome_error.take() {
                    self.model.error_text = err;
                    self.error_until = Some(Instant::now() + Duration::from_millis(ERROR_TTL_MS));
                }
            }

            let full_text = self.model.display_text();
            let (_, current_lines) =
                display_text_plan(&full_text, self.cfg.max_text_lines, layout::CHARS_PER_LINE);
            let lines = if self.model.state == OverlayState::Recording {
                self.peak_text_lines = self.peak_text_lines.max(current_lines);
                self.peak_text_lines
            } else {
                current_lines
            };
            let height =
                layout::BASE_HEIGHT + (lines.saturating_sub(1) as f64 * layout::BODY_LINE_H);
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
            live_text_plan(
                &self.model.segments,
                &self.model.partial,
                self.cfg.max_text_lines,
                layout::CHARS_PER_LINE,
            )
        } else {
            let (display_text, lines) =
                display_text_plan(&full_text, self.cfg.max_text_lines, layout::CHARS_PER_LINE);
            LiveTextPlan {
                segments: String::new(),
                partial: display_text,
                lines,
            }
        };
        let display_text = live_plan.full_text();
        let (state, label, color_rgb) = self.effective_state();
        let dur = format_duration(self.model.dur_ms);
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
        let header = header_parts(&label, &dur, &words_text, app, &self.model.chain_summary);
        self.render_state_icon(state, color_rgb);
        if self.last_status_text != header.state {
            fade_view(&self.status, 0.16);
            self.status
                .setStringValue(&NSString::from_str(&header.state));
            self.status
                .setTextColor(Some(&color_from_rgb_alpha(color_rgb, 1.0)));
            self.last_status_text = header.state;
        }

        let stats_text = stats_text(&header.duration, &header.words, &header.app);
        if self.last_stats_text != stats_text {
            self.stats.setStringValue(&NSString::from_str(&stats_text));
            self.last_stats_text = stats_text;
        }
        self.stats
            .setTextColor(Some(&color_from_rgb_alpha(self.cfg.text.secondary, 1.0)));

        // meta 行：notice 活跃时盖住 chain_summary，黄字。
        let (meta_text, meta_color) = if let Some(notice) = &self.model.notice {
            (notice.text.clone(), self.cfg.text.notice)
        } else {
            (header.meta, self.cfg.text.tertiary)
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
            self.cfg.text.error
        } else {
            self.cfg.text.primary
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
            self.recording_started
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

    fn render_body_text(&self, plan: &LiveTextPlan, fallback_color: u32) {
        if plan.segments.is_empty() {
            self.text.setStringValue(&NSString::from_str(&plan.partial));
            self.text
                .setTextColor(Some(&color_from_rgb_alpha(fallback_color, 1.0)));
            return;
        }

        let full = plan.full_text();
        let attributed = NSMutableAttributedString::from_nsstring(&NSString::from_str(&full));
        let segment_len = utf16_len(&plan.segments);
        let full_len = utf16_len(&full);
        let segment_color = color_from_rgb_alpha(self.cfg.text.segment, 0.88);
        let partial_color = color_from_rgb_alpha(self.cfg.text.primary, 1.0);
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
        let top_offset = height - layout::BASE_HEIGHT;
        let full = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(layout::WIDTH, height));
        // panel.contentView (glass 或 fallback) 跟随 panel 自动 resize；container 显式 set 确保 labels 坐标系对齐
        self.container.setFrame(full);
        let row = first_row_frames(top_offset);
        self.state_icon.setFrame(row.icon);
        self.status.setFrame(row.status);
        self.stats.setFrame(row.stats);
        self.meta.setFrame(row.meta);
        self.text.setFrame(NSRect::new(
            NSPoint::new(layout::H_PAD, layout::BOTTOM_PAD),
            NSSize::new(
                layout::BODY_W,
                layout::BODY_LINE_H + (lines.saturating_sub(1) as f64 * layout::BODY_LINE_H),
            ),
        ));
    }

    fn place(&mut self, height: f64) {
        let screen = NSScreen::mainScreen(self.mtm)
            .map(|screen| screen.frame())
            .unwrap_or_else(|| NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0)));
        let anchor = crate::focused_window_darwin::focused_window_frame(screen.size.height)
            .unwrap_or(screen);
        let frame = panel_frame(anchor, self.cfg.position, layout::WIDTH, height, screen);
        let animate = self.last_panel_frame.is_some();
        if self
            .last_panel_frame
            .is_none_or(|last| !rect_nearly_eq(last, frame))
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
        apply_panel_background_blur(&self.panel, cfg.background_blur_radius);
        apply_background_settings(&self.background, &cfg);
        self.cfg = cfg;
        self.model.state_color = self.model.state.color_rgb(&self.cfg.state);
        self.last_status_text.clear();
        self.last_visible_text.clear();
        self.last_meta_text.clear();
        // peak_text_lines 用旧上限可能比新 max_text_lines 大，clamp 回去
        let cap = self.cfg.max_text_lines.clamp(1, 8);
        if self.peak_text_lines > cap {
            self.peak_text_lines = cap;
        }
        self.render();
    }
}

fn root_frame() -> NSRect {
    NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(layout::WIDTH, layout::BASE_HEIGHT),
    )
}

fn make_panel(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSPanel> {
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        frame,
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setIgnoresMouseEvents(true);
    panel.setLevel(NSStatusWindowLevel);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    panel
}

fn apply_panel_background_blur(panel: &NSPanel, radius: i64) {
    unsafe {
        let Some(api) = SkyLightBlurApi::load() else {
            return;
        };
        let connection = (api.main_connection_id)();
        let Ok(window_id) = i32::try_from(panel.windowNumber()) else {
            return;
        };
        let _ = (api.set_window_background_blur_radius)(connection, window_id, radius.max(0));
    }
}

struct SkyLightBlurApi {
    main_connection_id: unsafe extern "C" fn() -> i32,
    set_window_background_blur_radius: unsafe extern "C" fn(i32, i32, i64) -> i32,
}

impl SkyLightBlurApi {
    fn load() -> Option<Self> {
        unsafe {
            const RTLD_LAZY: c_int = 0x1;
            let path = c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight";
            let handle = dlopen(path.as_ptr(), RTLD_LAZY);
            if handle.is_null() {
                return None;
            }
            let main = dlsym(handle, c"CGSMainConnectionID".as_ptr());
            let set_blur = dlsym(handle, c"CGSSetWindowBackgroundBlurRadius".as_ptr());
            if main.is_null() || set_blur.is_null() {
                return None;
            }
            Some(Self {
                main_connection_id: std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> i32>(
                    main,
                ),
                set_window_background_blur_radius: std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(i32, i32, i64) -> i32,
                >(set_blur),
            })
        }
    }
}

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

struct FirstRow {
    icon: NSRect,
    status: NSRect,
    stats: NSRect,
    meta: NSRect,
}

fn first_row_frames(top_offset: f64) -> FirstRow {
    let center_y = layout::HEADER_CENTER_Y + top_offset;
    let mut x = layout::H_PAD;
    let icon = NSRect::new(
        NSPoint::new(
            x,
            frame_y_for_visual_center(center_y, layout::ICON_BOX, layout::ICON_OPTICAL_Y),
        ),
        NSSize::new(layout::ICON_BOX, layout::ICON_BOX),
    );
    x += layout::ICON_BOX + layout::ICON_STATE_GAP;
    let status = NSRect::new(
        NSPoint::new(
            x,
            frame_y_for_visual_center(center_y, layout::STATE_BOX_H, layout::STATE_OPTICAL_Y),
        ),
        NSSize::new(layout::STATE_W, layout::STATE_BOX_H),
    );
    x += layout::STATE_W + layout::STATE_STATS_GAP;
    let stats = NSRect::new(
        NSPoint::new(
            x,
            frame_y_for_visual_center(center_y, layout::META_BOX_H, layout::META_OPTICAL_Y),
        ),
        NSSize::new(layout::STATS_W, layout::META_BOX_H),
    );
    x += layout::STATS_W + layout::META_GAP;
    let right = layout::WIDTH - layout::H_PAD;
    let meta_w = (right - x).max(layout::META_MIN_W);
    FirstRow {
        icon,
        status,
        stats,
        meta: NSRect::new(
            NSPoint::new(
                x,
                frame_y_for_visual_center(center_y, layout::META_BOX_H, layout::META_OPTICAL_Y),
            ),
            NSSize::new(meta_w, layout::META_BOX_H),
        ),
    }
}

fn frame_y_for_visual_center(center_y: f64, height: f64, optical_y: f64) -> f64 {
    center_y - height / 2.0 - optical_y
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

/// 构造 chrome。返回 (labels 容器, glass 句柄或 None, 可控色层, 初始化期错误).
///
/// 关键认知（v2 调整）：`NSGlassEffectView.contentView` 是 **"嵌在 glass 里"** 的语义 ——
/// 像琥珀封蝇，contentView 视觉上**在 glass 材质后面**，会被材质遮罩/模糊。所以 labels
/// **不能**塞进 glass.contentView，否则人眼看像在 glass 下面。
///
/// 正确做法：panel.contentView = 一个普通 root NSView；root 里 **glass 在底层**（先 addSubview），
/// labels 是 glass 的**兄弟**（后 addSubview，z-order 自然在上）。Apple 的 "arbitrary subviews
/// 行为未定" 警告只针对**直接 addSubview 到 glass**，不针对 glass 的兄弟。
fn build_chrome(
    mtm: MainThreadMarker,
    cfg: &EffectiveOverlayCfg,
) -> (
    Retained<NSView>,
    Option<Retained<NSGlassEffectView>>,
    Retained<NSView>,
    Option<String>,
) {
    let container = NSView::new(mtm);
    container.setFrame(root_frame());
    let background = make_background_layer(mtm, cfg);

    if AnyClass::get(c"NSGlassEffectView").is_some() {
        let glass = NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), root_frame());
        #[cfg(debug_assertions)]
        {
            crate::overlay::debug::dump_glass_selectors(&glass);
            crate::overlay::debug::probe_glass_state_ranges(&glass);
        }
        let missing = apply_glass_settings(&glass, cfg);
        glass.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        // glass 负责系统材质；background 负责稳定可控的色彩/深浅；labels 后续叠在最上面。
        container.addSubview(&glass);
        container.addSubview(&background);

        let err = if missing.is_empty() {
            None
        } else {
            Some(format!("glass SPI unavailable: {}", missing.join(", ")))
        };
        return (container, Some(glass), background, err);
    }

    let visual = NSVisualEffectView::new(mtm);
    visual.setFrame(root_frame());
    visual.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    visual.setMaterial(NSVisualEffectMaterial::HUDWindow);
    visual.setState(NSVisualEffectState::Active);
    visual.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    container.addSubview(&visual);
    container.addSubview(&background);
    (
        container,
        None,
        background,
        Some("NSGlassEffectView unavailable — falling back to HUD material".to_string()),
    )
}

/// 把 cfg 里所有 chrome 旋钮拍到 glass 上。返回检测到不可用的 SPI 列表（私有 selector 没 respond）。
/// 公开 API（cornerRadius/style）由 typed binding 保证存在，不进 missing 列表。
fn apply_glass_settings(glass: &NSGlassEffectView, cfg: &EffectiveOverlayCfg) -> Vec<&'static str> {
    glass.setCornerRadius(cfg.corner_radius);
    let style = match cfg.glass_style {
        GlassStyle::Clear => NSGlassEffectViewStyle::Regular,
        GlassStyle::Blur => NSGlassEffectViewStyle::Clear,
    };
    glass.setStyle(style);

    let mut missing = Vec::new();
    if !try_set_long(glass, c"set_variant:", c"setVariant:", cfg.glass_variant) {
        missing.push("variant");
    }
    if !try_set_long(
        glass,
        c"set_subduedState:",
        c"setSubduedState:",
        cfg.subdued,
    ) && cfg.subdued != 0
    {
        missing.push("subdued");
    }
    missing
}

fn make_background_layer(mtm: MainThreadMarker, cfg: &EffectiveOverlayCfg) -> Retained<NSView> {
    let background = NSView::new(mtm);
    background.setFrame(root_frame());
    background.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    background.setWantsLayer(true);
    apply_background_settings(&background, cfg);
    background
}

fn apply_background_settings(background: &NSView, cfg: &EffectiveOverlayCfg) {
    unsafe {
        let layer: *mut AnyObject = msg_send![background, layer];
        let color = color_from_rgb_alpha(cfg.background_rgb, cfg.background_alpha).CGColor();
        let _: () = msg_send![layer, setBackgroundColor: &*color];
        let _: () = msg_send![layer, setCornerRadius: cfg.corner_radius];
        let _: () = msg_send![layer, setMasksToBounds: true];
    }
}

/// 私有 SPI setter 通道。先试 `set_<key>:` 私有名，再退到 `setKey:` 公有名 ——
/// 顺序和参数 ABI（i64）都参考 electron-liquid-glass `glass_effect.mm::ResolveSetter`。
/// 私有 selector 在每个 macOS 版本可能改名/消失，所以两步 `respondsToSelector:` 验过再发。
///
/// 注意：subdued 看起来像 BOOL，但 runtime 实际期望 `NSInteger` (i64)。
/// 错传 BOOL 就调用静默无效果。
fn try_set_long(
    glass: &NSGlassEffectView,
    private_name: &core::ffi::CStr,
    public_name: &core::ffi::CStr,
    value: i64,
) -> bool {
    use objc2::runtime::{MessageReceiver, Sel};
    unsafe {
        let obj: &AnyObject = msg_send![glass, self];
        let private = Sel::register(private_name);
        if msg_send![obj, respondsToSelector: private] {
            let _: () = obj.send_message(private, (value,));
            #[cfg(debug_assertions)]
            crate::overlay::debug::trace(format!(
                "dispatched {} <- {value}",
                private_name.to_string_lossy()
            ));
            return true;
        }
        let public = Sel::register(public_name);
        if msg_send![obj, respondsToSelector: public] {
            let _: () = obj.send_message(public, (value,));
            #[cfg(debug_assertions)]
            crate::overlay::debug::trace(format!(
                "dispatched {} <- {value}",
                public_name.to_string_lossy()
            ));
            return true;
        }
        #[cfg(debug_assertions)]
        crate::overlay::debug::trace(format!(
            "missing {} / {}",
            private_name.to_string_lossy(),
            public_name.to_string_lossy()
        ));
    }
    false
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

fn color_from_rgb_alpha(rgb: u32, alpha: f64) -> Retained<NSColor> {
    let r = ((rgb >> 16) & 0xff) as f64 / 255.0;
    let g = ((rgb >> 8) & 0xff) as f64 / 255.0;
    let b = (rgb & 0xff) as f64 / 255.0;
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, alpha.clamp(0.0, 1.0))
}

fn display_text_plan(text: &str, max_lines: usize, chars_per_line: usize) -> (String, usize) {
    let max_lines = max_lines.clamp(1, 8);
    let chars_per_line = chars_per_line.max(8);
    let chars = text.chars().count().max(1);
    let lines = chars.div_ceil(chars_per_line).clamp(1, max_lines);
    let capacity = chars_per_line * max_lines;
    if chars <= capacity {
        return (text.to_string(), lines);
    }

    let keep = capacity.saturating_sub(1);
    let tail: String = text
        .chars()
        .rev()
        .take(keep)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    (format!("…{tail}"), max_lines)
}

#[derive(Debug, PartialEq, Eq)]
struct LiveTextPlan {
    segments: String,
    partial: String,
    lines: usize,
}

impl LiveTextPlan {
    fn full_text(&self) -> String {
        let mut text = self.segments.clone();
        text.push_str(&self.partial);
        text
    }
}

fn live_text_plan(
    segments: &[String],
    partial: &str,
    max_lines: usize,
    chars_per_line: usize,
) -> LiveTextPlan {
    let full_segments = segments.join("");
    let mut full = full_segments.clone();
    full.push_str(partial);
    let (display, lines) = display_text_plan(&full, max_lines, chars_per_line);
    if display == full {
        return LiveTextPlan {
            segments: full_segments,
            partial: partial.to_string(),
            lines,
        };
    }

    let visible_chars = display
        .strip_prefix('…')
        .unwrap_or(&display)
        .chars()
        .count();
    let partial_chars = partial.chars().count();
    let visible_partial_chars = partial_chars.min(visible_chars);
    let visible_segment_chars = visible_chars.saturating_sub(visible_partial_chars);
    let segment_tail = tail_chars(&full_segments, visible_segment_chars);
    let partial_tail = tail_chars(partial, visible_partial_chars);
    LiveTextPlan {
        segments: if segment_tail.is_empty() {
            "…".to_string()
        } else {
            format!("…{segment_tail}")
        },
        partial: partial_tail,
        lines,
    }
}

fn tail_chars(text: &str, count: usize) -> String {
    if count == 0 {
        return String::new();
    }
    text.chars()
        .rev()
        .take(count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

#[derive(Debug, PartialEq, Eq)]
struct HeaderParts {
    state: String,
    duration: String,
    words: String,
    app: String,
    meta: String,
}

fn header_parts(state: &str, duration: &str, words: &str, app: &str, chain: &str) -> HeaderParts {
    HeaderParts {
        state: state.to_string(),
        duration: duration.to_string(),
        words: words.to_string(),
        app: app.to_string(),
        meta: chain.to_string(),
    }
}

fn stats_text(duration: &str, words: &str, app: &str) -> String {
    if app.is_empty() {
        format!("{duration} · {words}")
    } else {
        format!("{duration} · {words} · {app}")
    }
}

fn panel_frame(
    anchor: NSRect,
    position: OverlayPosition,
    width: f64,
    height: f64,
    screen: NSRect,
) -> NSRect {
    let x = anchor.origin.x + (anchor.size.width - width) / 2.0;
    let y = match position {
        OverlayPosition::Top => {
            anchor.origin.y + anchor.size.height - height - layout::WINDOW_MARGIN
        }
        OverlayPosition::Middle => anchor.origin.y + (anchor.size.height - height) / 2.0,
        OverlayPosition::Bottom => anchor.origin.y + layout::WINDOW_MARGIN,
    };
    let x = clamp(
        x,
        screen.origin.x + layout::WINDOW_MARGIN,
        screen.origin.x + screen.size.width - width - layout::WINDOW_MARGIN,
    );
    let y = clamp(
        y,
        screen.origin.y + layout::WINDOW_MARGIN,
        screen.origin.y + screen.size.height - height - layout::WINDOW_MARGIN,
    );
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if min > max {
        return min;
    }
    value.max(min).min(max)
}

fn rect_nearly_eq(a: NSRect, b: NSRect) -> bool {
    (a.origin.x - b.origin.x).abs() < 0.5
        && (a.origin.y - b.origin.y).abs() < 0.5
        && (a.size.width - b.size.width).abs() < 0.5
        && (a.size.height - b.size.height).abs() < 0.5
}

fn fade_view(view: &NSView, duration_s: f64) {
    if let Some(layer) = view.layer() {
        let transition = CATransition::animation();
        transition.setDuration(duration_s);
        transition.setType(unsafe { kCATransitionFade });
        layer.addAnimation_forKey(&transition, Some(ns_string!("fade")));
    }
}

fn format_duration(ms: u64) -> String {
    let total_secs = ms / 1000;
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    if h > 0 {
        format!("{h}h{m}m{s}s")
    } else if m > 0 {
        format!("{m}m{s}s")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visual_center(frame: NSRect, optical_y: f64) -> f64 {
        frame.origin.y + frame.size.height / 2.0 + optical_y
    }

    #[test]
    fn text_line_count_is_bounded() {
        assert_eq!(display_text_plan("", 5, 34).1, 1);
        assert_eq!(display_text_plan("短句", 5, 34).1, 1);
        assert_eq!(display_text_plan(&"字".repeat(70), 5, 34).1, 3);
        assert_eq!(display_text_plan(&"字".repeat(300), 5, 34).1, 5);
    }

    #[test]
    fn long_text_keeps_tail() {
        let text = format!("{}{}", "前".repeat(200), "后".repeat(20));
        let (visible, lines) = display_text_plan(&text, 5, 20);
        assert_eq!(lines, 5);
        assert!(visible.starts_with('…'));
        assert!(visible.ends_with(&"后".repeat(20)));
        assert!(!visible.contains(&"前".repeat(120)));
    }

    #[test]
    fn header_parts_keep_state_duration_and_meta_separate() {
        let parts = header_parts("Recording", "3s", "84 words", "Xcode", "filler");
        assert_eq!(parts.state, "Recording");
        assert_eq!(parts.duration, "3s");
        assert_eq!(parts.words, "84 words");
        assert_eq!(parts.app, "Xcode");
        assert_eq!(parts.meta, "filler");
    }

    #[test]
    fn first_row_clusters_stats_and_app_on_left_with_wide_meta() {
        let row = first_row_frames(0.0);
        assert!(row.stats.origin.x - (row.status.origin.x + row.status.size.width) <= 6.0);
        assert!(row.stats.size.width >= 210.0);
        assert!(row.stats.origin.x < row.meta.origin.x);
        assert!(row.meta.size.width >= 180.0);
    }

    #[test]
    fn base_overlay_spacing_is_compact() {
        const {
            assert!(layout::BASE_HEIGHT <= 68.0);
            assert!(layout::H_PAD <= 16.0);
            assert!(layout::BOTTOM_PAD <= 8.0);
        }
    }

    #[test]
    fn first_row_uses_shared_visual_center() {
        let row = first_row_frames(0.0);
        let center = layout::HEADER_CENTER_Y;
        assert!((visual_center(row.icon, layout::ICON_OPTICAL_Y) - center).abs() < 0.1);
        assert!((visual_center(row.status, layout::STATE_OPTICAL_Y) - center).abs() < 0.1);
        assert!((visual_center(row.stats, layout::META_OPTICAL_Y) - center).abs() < 0.1);
        assert!((visual_center(row.meta, layout::META_OPTICAL_Y) - center).abs() < 0.1);
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
    fn header_body_gap_keeps_rows_breathing() {
        assert_eq!(layout::HEADER_BODY_GAP, 2.0);
    }

    #[test]
    fn segment_text_uses_readable_secondary_color() {
        assert_eq!(
            crate::config::theme::OverlayTextTheme::default().segment,
            crate::config::theme::palette::FG1
        );
    }

    #[test]
    fn stats_text_is_inline_metadata() {
        assert_eq!(stats_text("12s", "128字", "Xcode"), "12s · 128字 · Xcode");
        assert_eq!(stats_text("12s", "128字", ""), "12s · 128字");
    }

    #[test]
    fn live_text_plan_keeps_segments_and_partial_distinct() {
        let plan = live_text_plan(&["已经定型。".to_string()], "正在识别", 5, 34);
        assert_eq!(plan.segments, "已经定型。");
        assert_eq!(plan.partial, "正在识别");
        assert_eq!(plan.lines, 1);
    }

    #[test]
    fn positions_overlay_inside_anchor_centered() {
        let anchor = NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(800.0, 600.0));
        let screen = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1200.0, 900.0));
        let bottom = panel_frame(anchor, OverlayPosition::Bottom, 540.0, 86.0, screen);
        assert_eq!(bottom.origin.x, 230.0);
        assert_eq!(bottom.origin.y, 116.0);
        let middle = panel_frame(anchor, OverlayPosition::Middle, 540.0, 86.0, screen);
        assert_eq!(middle.origin.x, 230.0);
        assert_eq!(middle.origin.y, 357.0);
        let top = panel_frame(anchor, OverlayPosition::Top, 540.0, 86.0, screen);
        assert_eq!(top.origin.x, 230.0);
        assert_eq!(top.origin.y, 598.0);
    }
}
