use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSApplication,
    NSApplicationActivationPolicy, NSApplicationDelegate, NSBorderType, NSEvent, NSFont,
    NSFontAttributeName, NSFontWeightBold, NSForegroundColorAttributeName, NSGlassEffectView,
    NSImage, NSImageAlignment, NSImageFrameStyle, NSImageScaling, NSImageSymbolConfiguration,
    NSImageSymbolScale, NSImageView, NSLineBreakMode, NSMutableParagraphStyle, NSPanel,
    NSParagraphStyleAttributeName, NSScreen, NSScrollView, NSScrollerStyle, NSTextAlignment,
    NSTextField, NSTextView, NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSMutableAttributedString, NSNotification, NSNumber,
    NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString, NSTimer,
};
use objc2_quartz_core::{
    kCATransitionFade, CABasicAnimation, CALayer, CAMediaTiming, CATransaction, CATransition,
};

use crate::config::theme::EffectiveOverlayCfg;
use crate::overlay::layout as L;
use crate::overlay::{
    OverlayActionHandle, OverlayCmd, OverlayModel, OverlayReceiver, OverlayState, ProfileChoice,
    TextKind,
};

use super::chrome::{
    apply_background_settings, apply_glass_settings, apply_panel_background_blur, build_chrome,
    color_from_rgb_alpha, make_glass_surface, make_panel, InteractivePanel,
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

const BODY_TEXT_VERTICAL_PAD: f64 = 4.0;
const BODY_SCROLL_INDICATOR_W: f64 = 3.0;
const BODY_SCROLL_INDICATOR_MIN_H: f64 = 18.0;
const BODY_SCROLL_INDICATOR_HIDE_AFTER: Duration = Duration::from_millis(800);
const BODY_SCROLL_INDICATOR_FADE: f64 = 0.22;
const BODY_SCROLL_INDICATOR_ALPHA: f64 = 0.72;
const BODY_SCROLL_INDICATOR_OPACITY_EPSILON: f32 = 0.01;
const PROFILE_PICKER_CLOSE_AFTER: Duration = Duration::from_millis(800);
const PROFILE_PICKER_ROW_H: f64 = 28.0;
const PROFILE_PICKER_V_PAD: f64 = 6.0;
const PROFILE_PICKER_ROW_X: f64 = 6.0;
const PROFILE_PICKER_ROW_H_PAD: f64 = 12.0;
const PROFILE_PICKER_TITLE_X: f64 = 10.0;
const PROFILE_PICKER_TITLE_Y: f64 = 5.0;
const PROFILE_PICKER_TITLE_H_PAD: f64 = 20.0;
const PROFILE_PICKER_TITLE_V_PAD: f64 = 8.0;
const PROFILE_PICKER_MEASURE_W: f64 = 1000.0;
const PROFILE_PICKER_WIDTH_EXTRA: f64 = 28.0;
const PROFILE_PICKER_MIN_W: f64 = 180.0;
const PROFILE_PICKER_MAX_W: f64 = 520.0;
const PROFILE_PICKER_HIT_SLOP: f64 = 10.0;
const POLL_INTERVAL_S: f64 = 0.033;
const INITIAL_PANEL_X: f64 = 80.0;
const INITIAL_PANEL_Y: f64 = 860.0;
const ERROR_SHAKE_OFFSET: f64 = 4.0;
const ERROR_SHAKE_DURATION: f64 = 0.05;
const ERROR_SHAKE_REPEAT_COUNT: f32 = 3.0;
const BODY_ERROR_FADE: f64 = 0.10;
const BODY_FINAL_FADE: f64 = 0.18;
const HEADER_TEXT_FADE: f64 = 0.16;
const ICON_CHANGE_FADE: f64 = 0.18;
const LIVE_SEGMENT_ALPHA: f64 = 0.88;
const DEFAULT_SCREEN_W: f64 = 1440.0;
const DEFAULT_SCREEN_H: f64 = 900.0;

fn body_text_width() -> f64 {
    L::constants::BODY_TEXT_W
}

/// 状态图标的动画曲线。只剩 Idle 符号的缓慢 alpha 呼吸；其余状态的动效都是 icon_fx
/// 的自绘 CALayer，不走这里。
mod anim {
    use std::f64::consts::TAU;

    const IDLE_BREATH_PERIOD_MS: f64 = 3000.0;
    const IDLE_BREATH_MID_ALPHA: f64 = 0.775;
    const IDLE_BREATH_SWING: f64 = 0.225;

    /// Idle 待命：缓慢轻微的 alpha 呼吸（周期 ~3s，约 0.55–1.0），配合背后的呼吸光晕。
    pub fn idle_breath(ms: f64) -> f64 {
        IDLE_BREATH_MID_ALPHA + IDLE_BREATH_SWING * (TAU * ms / IDLE_BREATH_PERIOD_MS).sin()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn idle_breath_is_gentle_and_in_range() {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for i in 0..=2000 {
                let v = idle_breath(IDLE_BREATH_PERIOD_MS * i as f64 / 2000.0);
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
    actions: OnceCell<OverlayActionHandle>,
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
            let actions = self
                .ivars()
                .actions
                .get()
                .cloned()
                .expect("overlay actions initialized");
            let overlay = OverlayView::new(mtm, actions, cfg);
            self.ivars().overlay.set(RefCell::new(overlay)).ok();

            let timer = unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    POLL_INTERVAL_S,
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
        rx: OverlayReceiver,
        actions: OverlayActionHandle,
        cfg: EffectiveOverlayCfg,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars {
            overlay: OnceCell::new(),
            rx: OnceCell::from(RefCell::new(rx)),
            actions: OnceCell::from(actions),
            cfg: OnceCell::from(cfg),
            timer: OnceCell::new(),
        });
        unsafe { msg_send![super(this), init] }
    }
}

pub fn run(rx: OverlayReceiver, actions: OverlayActionHandle, cfg: EffectiveOverlayCfg) {
    let mtm = MainThreadMarker::new().expect("AppKit must run on main thread");
    let app = NSApplication::sharedApplication(mtm);
    let delegate = OverlayDelegate::new(mtm, rx, actions, cfg);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
}

struct OverlayView {
    mtm: MainThreadMarker,
    cfg: EffectiveOverlayCfg,
    actions: OverlayActionHandle,
    model: OverlayModel,
    panel: Retained<InteractivePanel>,
    /// root content view。glass/background/labels 都是它的直接子视图。
    container: Retained<NSView>,
    /// `Some` 表示拿到了真正的 `NSGlassEffectView`；`None` 表示走 NSVisualEffectView fallback。
    glass: Option<Retained<NSGlassEffectView>>,
    background: Retained<NSView>,
    state_icon: Retained<NSImageView>,
    status: Retained<NSTextField>,
    stats: Retained<NSTextField>,
    meta: Retained<NSTextField>,
    pipeline_click: Retained<PipelineClickView>,
    pipeline_clicked: Rc<Cell<bool>>,
    body_scroll: Retained<BodyScrollView>,
    body_text: Retained<NSTextView>,
    body_scroll_indicator: Retained<CALayer>,
    body_overflow: bool,
    body_follow: bool,
    body_programmatic_scroll: bool,
    body_scroll_indicator_until: Option<Instant>,
    last_body_visible_y: Option<f64>,
    profile_panel: Option<Retained<InteractivePanel>>,
    profile_selection_pending: Rc<Cell<bool>>,
    pending_profile_choice: Rc<RefCell<Option<ProfileChoice>>>,
    profile_close_after: Option<Instant>,
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
}

#[derive(Debug, Clone, Copy)]
struct BodyLineMetrics {
    /// 单行 body viewport 高度，包含 NSTextView 上下 inset。
    single: f64,
    /// 每新增一行带来的真实行高增量，不重复计算上下 inset。
    extra: f64,
    /// TextKit 实际 laid out line fragment 数。
    line_count: usize,
    /// 最后 max_text_lines 行的真实 viewport/scroll offset，包含 NSTextView 上下 inset。
    tail: Option<L::BodyTailMetrics>,
}

impl BodyLineMetrics {
    fn fallback() -> Self {
        Self {
            single: L::constants::BODY_LINE_H,
            extra: L::constants::BODY_LINE_H,
            line_count: 1,
            tail: None,
        }
    }
}

#[derive(Default)]
struct BodyScrollIvars {
    inside: Cell<bool>,
    tracking: OnceCell<Retained<NSTrackingArea>>,
}

define_class!(
    #[unsafe(super = NSScrollView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = BodyScrollIvars]
    struct BodyScrollView;

    unsafe impl NSObjectProtocol for BodyScrollView {}

    impl BodyScrollView {
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            unsafe {
                let _: () = msg_send![super(self), updateTrackingAreas];
            }
            if self.ivars().tracking.get().is_some() {
                return;
            }
            let owner: &AnyObject = unsafe { msg_send![self, self] };
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0)),
                    NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveAlways
                        | NSTrackingAreaOptions::InVisibleRect,
                    Some(owner),
                    None,
                )
            };
            self.addTrackingArea(&area);
            self.ivars().tracking.set(area).ok();
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            self.ivars().inside.set(true);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.ivars().inside.set(false);
        }
    }
);

struct PipelineClickIvars {
    clicked: Rc<Cell<bool>>,
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = PipelineClickIvars]
    struct PipelineClickView;

    unsafe impl NSObjectProtocol for PipelineClickView {}

    impl PipelineClickView {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            self.ivars().clicked.set(true);
        }
    }
);

impl PipelineClickView {
    fn new(mtm: MainThreadMarker, frame: NSRect, clicked: Rc<Cell<bool>>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PipelineClickIvars { clicked });
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setWantsLayer(true);
        view
    }
}

struct ProfileRowIvars {
    actions: OverlayActionHandle,
    bundle_id: String,
    choice: ProfileChoice,
    selected: Rc<Cell<bool>>,
    pending_choice: Rc<RefCell<Option<ProfileChoice>>>,
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ProfileRowIvars]
    struct ProfileRowView;

    unsafe impl NSObjectProtocol for ProfileRowView {}

    impl ProfileRowView {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut AnyObject {
            if point_in_rect(point, self.bounds()) {
                unsafe { msg_send![self, self] }
            } else {
                std::ptr::null_mut()
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            let ivars = self.ivars();
            ivars.actions.send(crate::overlay::OverlayAction::BindProfile {
                bundle_id: ivars.bundle_id.clone(),
                profile: ivars.choice.id.clone(),
            });
            *ivars.pending_choice.borrow_mut() = Some(ivars.choice.clone());
            ivars.selected.set(true);
        }
    }
);

impl ProfileRowView {
    fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        actions: OverlayActionHandle,
        bundle_id: String,
        choice: ProfileChoice,
        selected: Rc<Cell<bool>>,
        pending_choice: Rc<RefCell<Option<ProfileChoice>>>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ProfileRowIvars {
            actions,
            bundle_id,
            choice,
            selected,
            pending_choice,
        });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

impl OverlayView {
    fn new(mtm: MainThreadMarker, actions: OverlayActionHandle, cfg: EffectiveOverlayCfg) -> Self {
        let initial_frame = NSRect::new(
            NSPoint::new(INITIAL_PANEL_X, INITIAL_PANEL_Y),
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
        let pipeline_clicked = Rc::new(Cell::new(false));
        let pipeline_click =
            PipelineClickView::new(mtm, to_nsrect(row.meta), pipeline_clicked.clone());
        pipeline_click.setHidden(true);
        let body_frame = NSRect::new(
            NSPoint::new(L::constants::H_PAD, L::constants::BOTTOM_PAD),
            NSSize::new(
                L::constants::WIDTH - L::constants::H_PAD,
                L::constants::BODY_LINE_H,
            ),
        );
        let body_scroll = make_body_scroll(mtm, body_frame);
        let body_scroll_indicator = make_body_scroll_indicator(cfg.core.text.primary);
        if let Some(layer) = body_scroll.layer() {
            layer.addSublayer(&body_scroll_indicator);
        }
        let body_text = body_scroll
            .documentView()
            .and_then(|view| view.downcast::<NSTextView>().ok())
            .expect("body scroll document view is NSTextView");
        // meta 行右对齐，长链路保留最右侧的实际 provider/processor 尾部。
        meta.setLineBreakMode(NSLineBreakMode::ByTruncatingHead);

        // labels 后 addSubview = z-order 在前。glass 在 build_chrome 里已经先进 container 当底色，
        // 这里追加 labels 自然叠在 glass 上面。
        // icon_fx_view 先加 → z-order 在 state_icon 之后（背后），FX 画在符号后面。
        container.addSubview(&icon_fx_view);
        container.addSubview(&body_scroll);
        container.addSubview(&state_icon);
        container.addSubview(&status);
        container.addSubview(&stats);
        container.addSubview(&meta);
        container.addSubview(&pipeline_click);

        panel.setContentView(Some(&container));
        panel.orderOut(None);

        let model = OverlayModel::new(&cfg.core.state);

        Self {
            mtm,
            cfg,
            actions,
            model,
            panel,
            container,
            glass,
            background,
            state_icon,
            status,
            stats,
            meta,
            pipeline_click,
            pipeline_clicked,
            body_scroll,
            body_text,
            body_scroll_indicator,
            body_overflow: false,
            body_follow: true,
            body_programmatic_scroll: false,
            body_scroll_indicator_until: None,
            last_body_visible_y: None,
            profile_panel: None,
            profile_selection_pending: Rc::new(Cell::new(false)),
            pending_profile_choice: Rc::new(RefCell::new(None)),
            profile_close_after: None,
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
        }
    }

    fn apply(&mut self, cmd: OverlayCmd) {
        if matches!(cmd, OverlayCmd::Quit) {
            self.close_profile_panel();
            NSApplication::sharedApplication(self.mtm).terminate(None);
            return;
        }
        if matches!(cmd, OverlayCmd::Dismiss) {
            self.close_profile_panel();
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
            self.clear_rendered_session();
        }
        self.render();
    }

    fn tick(&mut self) {
        let prev_visible = self.model.visible;
        let _ = self.model.tick(Instant::now(), &self.cfg.core.state);
        if prev_visible && !self.model.visible {
            self.last_text_update = None;
            self.clear_rendered_session();
        }
        self.render();
    }

    fn render(&mut self) {
        self.update_body_hover();
        self.update_profile_interaction();
        // 淡出动画结束且仍不可见 → 真正下屏。若期间又变可见，留给下面的可见分支取走
        // pending 并淡回不透明（避免清了 pending 却没恢复 alpha 的竞态）。
        if let Some(at) = self.pending_order_out {
            if Instant::now() >= at && !self.model.visible {
                self.panel.orderOut(None);
                self.panel_shown = false;
                self.pending_order_out = None;
                self.last_panel_frame = None;
                self.last_height = None;
                self.panel.setIgnoresMouseEvents(true);
            }
        }

        if !self.model.visible {
            if self.profile_panel.is_some() {
                return;
            }
            // 隐藏：先淡出，淡完再 orderOut（上面的 pending 逻辑收尾）。
            if self.panel_shown && self.pending_order_out.is_none() {
                fade_window_alpha(&self.panel, 0.0);
                self.pending_order_out =
                    Some(Instant::now() + Duration::from_secs_f64(APPEAR_FADE));
            }
            self.panel.setIgnoresMouseEvents(true);
            return;
        }

        // 先把 body 文本刷到 NSTextView，再用同一套 layout manager 实测高度——单一权威是真正
        // 画字的那套排版，面板长高/缩回完全跟随真实换行，不提前留空行也不裁字。
        self.update_body_text();
        let line_metrics = self.measure_body_line_metrics();
        let content_h = self.measure_body_height(&line_metrics);
        let geo = L::body_geometry_with_tail_metrics(
            content_h,
            self.cfg.core.max_text_lines,
            line_metrics.single,
            line_metrics.extra,
            line_metrics.line_count,
            line_metrics.tail,
        );
        let height = geo.panel_height;
        let height_changed = self.last_height != Some(height);
        self.last_height = Some(height);

        if !self.panel_shown {
            // 首次出现：先就位，再 alpha 0→1 淡入。
            self.pending_order_out = None;
            self.layout(&geo, false);
            self.place(height, false);
            self.panel.setAlphaValue(0.0);
            self.panel.orderFrontRegardless();
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
                self.layout(&geo, true);
                self.place(height, true);
                NSAnimationContext::endGrouping();
            } else {
                self.layout(&geo, false);
                self.place(height, true);
            }
        }

        self.update_body_follow_after_layout(&geo);
        self.render_header();
    }

    fn update_profile_interaction(&mut self) {
        if self.profile_selection_pending.replace(false) {
            if let Some(choice) = self.pending_profile_choice.borrow_mut().take() {
                self.model.profile = choice.id;
                self.model.profile_display_name = choice.display_name;
                self.model.asr_provider = choice.asr_provider;
                self.model.chain_summary = choice.chain_summary;
                self.last_meta_text.clear();
            }
            self.close_profile_panel();
        }
        if self.pipeline_clicked.replace(false) {
            if self.profile_panel.is_some() {
                self.close_profile_panel();
            } else {
                self.open_profile_panel();
            }
        }
        if self.profile_panel.is_some() {
            if self.mouse_inside_profile_picker() {
                self.profile_close_after = None;
            } else {
                let close_after = self
                    .profile_close_after
                    .get_or_insert_with(|| Instant::now() + PROFILE_PICKER_CLOSE_AFTER);
                if Instant::now() >= *close_after {
                    self.close_profile_panel();
                }
            }
        }
    }

    fn update_body_hover(&mut self) {
        let inside = self.body_scroll.ivars().inside.get();
        if inside && self.body_overflow && self.body_follow {
            self.body_follow = false;
            self.show_body_scroll_indicator();
        } else if (!inside || !self.body_overflow) && !self.body_follow {
            self.body_follow = true;
            self.body_programmatic_scroll = true;
            self.body_scroll_indicator_until = None;
        }
    }

    /// 刷新 body text field 的内容（dedup），让随后的实测拿到真实排版。
    /// 显示优先级 error > final > segments+partial，与 `model.display_text` 一致：
    /// error/final 单色，live 的 segments/partial 双色。
    fn update_body_text(&mut self) {
        let text_color = if !self.model.error_text.is_empty() {
            self.cfg.core.text.error
        } else {
            self.cfg.core.text.primary
        };
        // final 定稿（empty→非空）时做一次轻淡入；live partial 不淡，保持 crisp。
        let final_appeared = !self.model.final_text.is_empty() && !self.last_had_final;
        self.last_had_final = !self.model.final_text.is_empty();

        let plain = if !self.model.error_text.is_empty() {
            Some(self.model.error_text.clone())
        } else if !self.model.final_text.is_empty() {
            Some(self.model.final_text.clone())
        } else {
            None
        };
        let segments = self.model.segments.join("");
        let partial = self.model.partial.clone();
        // dedup key 用 model 的权威显示串（与 plain/segments+partial 分支一致），
        // 避免在 view 里重复一份显示优先级逻辑。
        let display_text = self.model.display_text();

        if self.last_visible_text != display_text {
            if !self.model.error_text.is_empty() {
                fade_view(&self.body_text, BODY_ERROR_FADE);
            } else if final_appeared {
                fade_view(&self.body_text, BODY_FINAL_FADE);
            }
            match &plain {
                Some(text) => self.set_body_plain(text, text_color),
                None => self.set_body_live(&segments, &partial),
            }
            self.last_visible_text = display_text;
        }
        self.body_text
            .setTextColor(Some(&color_from_rgb_alpha(text_color, 1.0)));
        self.update_body_scroll_indicator_visibility();
    }

    /// 用 body NSTextView 自己的 layout manager 在固定内容宽度下实测高度。
    /// 这是真正画字的尺寸，跟显示零漂移。
    fn measure_body_height(&self, metrics: &BodyLineMetrics) -> f64 {
        unsafe {
            let Some(container) = self.body_text.textContainer() else {
                return metrics.single;
            };
            let Some(layout) = self.body_text.layoutManager() else {
                return metrics.single;
            };
            layout.ensureLayoutForTextContainer(&container);
            let used = layout.usedRectForTextContainer(&container);
            let inset = self.body_text.textContainerInset();
            (used.origin.y + used.size.height + inset.height * 2.0).ceil()
        }
        .max(metrics.single)
    }

    fn measure_body_line_metrics(&self) -> BodyLineMetrics {
        unsafe {
            let Some(container) = self.body_text.textContainer() else {
                return BodyLineMetrics::fallback();
            };
            let Some(layout) = self.body_text.layoutManager() else {
                return BodyLineMetrics::fallback();
            };
            layout.ensureLayoutForTextContainer(&container);
            let inset = self.body_text.textContainerInset();
            let fragments = measured_body_line_fragments(&layout, &container);
            let extra = fragments.iter().map(|line| line.h).fold(0.0_f64, f64::max);
            let extra = if extra > 0.0 {
                extra
            } else {
                let font = self
                    .body_text
                    .font()
                    .unwrap_or_else(|| NSFont::systemFontOfSize(typography::BODY));
                layout.defaultLineHeightForFont(&font).ceil().max(1.0)
            };
            let max_lines = self.cfg.core.max_text_lines.max(1);
            let tail = body_tail_metrics(&fragments, max_lines, inset.height);
            BodyLineMetrics {
                single: (extra + inset.height * 2.0).ceil().max(1.0),
                extra,
                line_count: fragments.len().max(1),
                tail,
            }
        }
    }

    fn render_header(&mut self) {
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
            fade_view(&self.status, HEADER_TEXT_FADE);
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

        // meta 行：notice 活跃时盖住 profile/pipeline，纯黄字；否则 profile 加粗高亮 +
        // pipeline（剥掉 kind 前缀）暗色。
        if let Some(notice) = &self.model.notice {
            let notice_text = notice.text.clone();
            if self.last_meta_text != notice_text {
                fade_view(&self.meta, HEADER_TEXT_FADE);
                self.meta.setStringValue(&NSString::from_str(&notice_text));
                self.last_meta_text = notice_text;
            }
            self.meta
                .setTextColor(Some(&color_from_rgb_alpha(self.cfg.core.text.notice, 1.0)));
        } else {
            let meta_text = L::profile_chain_display(
                &self.model.profile_display_name,
                &self.model.asr_provider,
                &header.meta,
            );
            // dedup key：profile 与 pipeline 用控制字符分隔，避免与 notice 文案撞键。
            let key = format!("{}\u{1}{}", self.model.profile, meta_text);
            if self.last_meta_text != key {
                fade_view(&self.meta, HEADER_TEXT_FADE);
                self.render_meta_text(&self.model.profile_display_name, &meta_text);
                self.last_meta_text = key;
            }
        }
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
            fade_view(&self.state_icon, ICON_CHANGE_FADE);
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
            shake.setFromValue(Some(&NSNumber::numberWithDouble(-ERROR_SHAKE_OFFSET)));
            shake.setToValue(Some(&NSNumber::numberWithDouble(ERROR_SHAKE_OFFSET)));
        }
        shake.setDuration(ERROR_SHAKE_DURATION);
        shake.setAutoreverses(true);
        shake.setRepeatCount(ERROR_SHAKE_REPEAT_COUNT);
        layer.addAnimation_forKey(&shake, Some(ns_string!("shake")));
    }

    /// error/final 单色全文。
    fn set_body_plain(&self, text: &str, color_rgb: u32) {
        let attributed = self.body_attributed(text, color_rgb);
        unsafe {
            if let Some(storage) = self.body_text.textStorage() {
                storage.setAttributedString(&attributed);
            }
        }
    }

    /// 录音中 live 文本：已定型 segments（暗色）+ 当前 partial（高亮）双色。
    fn set_body_live(&self, segments: &str, partial: &str) {
        if segments.is_empty() {
            self.set_body_plain(partial, self.cfg.core.text.primary);
            return;
        }

        let full = format!("{segments}{partial}");
        let attributed = self.body_attributed(&full, self.cfg.core.text.primary);
        let segment_len = L::utf16_len(segments);
        let full_len = L::utf16_len(&full);
        let segment_color = color_from_rgb_alpha(self.cfg.core.text.segment, LIVE_SEGMENT_ALPHA);
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
            if let Some(storage) = self.body_text.textStorage() {
                storage.setAttributedString(&attributed);
            }
        }
    }

    fn body_attributed(&self, text: &str, color_rgb: u32) -> Retained<NSMutableAttributedString> {
        let attributed = NSMutableAttributedString::from_nsstring(&NSString::from_str(text));
        let len = L::utf16_len(text);
        if len == 0 {
            return attributed;
        }
        let font = NSFont::systemFontOfSize(typography::BODY);
        let color = color_from_rgb_alpha(color_rgb, 1.0);
        unsafe {
            let _: () = msg_send![
                &attributed,
                addAttribute: NSFontAttributeName,
                value: &*font,
                range: NSRange::new(0, len)
            ];
            let _: () = msg_send![
                &attributed,
                addAttribute: NSForegroundColorAttributeName,
                value: &*color,
                range: NSRange::new(0, len)
            ];
        }
        attributed
    }

    /// meta 行：`Profile:` 加粗高亮，后接完整链路（tertiary 暗色）。
    fn render_meta_text(&self, display_name: &str, text: &str) {
        self.render_profile_chain_text(&self.meta, display_name, text, NSTextAlignment::Right);
    }

    fn render_profile_chain_text(
        &self,
        field: &NSTextField,
        display_name: &str,
        text: &str,
        alignment: NSTextAlignment,
    ) {
        if display_name.is_empty() {
            field.setStringValue(&NSString::from_str(text));
            field.setTextColor(Some(&color_from_rgb_alpha(
                self.cfg.core.text.tertiary,
                1.0,
            )));
            return;
        }
        let prefix = format!("{display_name}:");
        let attributed = NSMutableAttributedString::from_nsstring(&NSString::from_str(text));
        let prefix_len = L::utf16_len(&prefix);
        let full_len = L::utf16_len(text);
        let profile_color = color_from_rgb_alpha(self.cfg.core.text.primary, 1.0);
        let pipeline_color = color_from_rgb_alpha(self.cfg.core.text.tertiary, 1.0);
        let bold_font = NSFont::boldSystemFontOfSize(typography::META);
        // attributed string 不带 paragraph style 时按 natural（左）对齐，会无视 field 的
        // setAlignment(.Right)，导致 meta 左靠、右侧空一大块。显式加右对齐段落样式。
        let paragraph = NSMutableParagraphStyle::new();
        paragraph.setAlignment(alignment);
        unsafe {
            let _: () = msg_send![
                &attributed,
                addAttribute: NSParagraphStyleAttributeName,
                value: &*paragraph,
                range: NSRange::new(0, full_len)
            ];
            let _: () = msg_send![
                &attributed,
                addAttribute: NSForegroundColorAttributeName,
                value: &*profile_color,
                range: NSRange::new(0, prefix_len)
            ];
            let _: () = msg_send![
                &attributed,
                addAttribute: NSFontAttributeName,
                value: &*bold_font,
                range: NSRange::new(0, prefix_len)
            ];
            if full_len > prefix_len {
                let _: () = msg_send![
                    &attributed,
                    addAttribute: NSForegroundColorAttributeName,
                    value: &*pipeline_color,
                    range: NSRange::new(prefix_len, full_len - prefix_len)
                ];
            }
            let _: () = msg_send![field, setAttributedStringValue: &*attributed];
        }
    }

    fn clear_rendered_session(&mut self) {
        self.body_text.setString(ns_string!(""));
        self.stats.setStringValue(&NSString::from_str(""));
        self.meta.setStringValue(&NSString::from_str(""));
        self.last_visible_text.clear();
        self.last_stats_text.clear();
        self.last_meta_text.clear();
        // 隐藏后重置，下次重新进入 Error 能再触发抖动 / final 能再淡入。
        self.last_icon_state = None;
        self.last_had_final = false;
        self.body_overflow = false;
        self.body_follow = true;
        self.body_programmatic_scroll = false;
        self.body_scroll_indicator_until = None;
        self.last_body_visible_y = None;
    }

    fn layout(&mut self, geo: &L::BodyGeometry, animated: bool) {
        let full = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(L::constants::WIDTH, geo.panel_height),
        );
        // 材质层和色层必须跟 root 同步动画，避免 Liquid Glass 边缘与实际外框短暂错位。
        set_view_frame(&self.container, full, animated);
        if let Some(glass) = &self.glass {
            set_view_frame(glass, full, animated);
        }
        set_view_frame(&self.background, full, animated);
        let row = L::first_row_frames(geo.top_offset);
        set_view_frame(&self.state_icon, to_nsrect(row.icon), animated);
        set_view_frame(&self.icon_fx_view, to_nsrect(row.icon), animated);
        set_view_frame(&self.status, to_nsrect(row.status), animated);
        set_view_frame(&self.stats, to_nsrect(row.stats), animated);
        set_view_frame(&self.meta, to_nsrect(row.meta), animated);
        set_view_frame(&self.pipeline_click, to_nsrect(row.meta), animated);
        self.pipeline_click.setHidden(
            !self.model.visible || self.model.bundle_id.is_none() || self.model.notice.is_some(),
        );
        set_view_frame(&self.body_scroll, to_nsrect(geo.body_viewport), animated);
        self.body_text.setFrame(to_nsrect(geo.body_document));
        let became_overflow = geo.body_overflow && !self.body_overflow;
        self.body_overflow = geo.body_overflow;
        if became_overflow {
            self.show_body_scroll_indicator();
        }
        self.panel.setIgnoresMouseEvents(!L::wants_mouse(
            self.model.visible,
            self.body_overflow,
            self.model.bundle_id.is_some(),
        ));
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
        self.body_scroll_indicator.setBackgroundColor(Some(
            &color_from_rgb_alpha(self.cfg.core.text.primary, BODY_SCROLL_INDICATOR_ALPHA)
                .CGColor(),
        ));
        self.model.state_color = self.model.state.color_rgb(&self.cfg.core.state);
        self.last_status_text.clear();
        self.last_visible_text.clear();
        self.last_meta_text.clear();
        self.render();
    }

    fn update_body_follow_after_layout(&mut self, geo: &L::BodyGeometry) {
        if self.body_follow {
            self.scroll_body_to_end(geo.scroll_bottom_offset);
            self.body_programmatic_scroll = true;
        }
        self.update_body_scroll_indicator(geo);
    }

    fn scroll_body_to_end(&self, bottom_offset: f64) {
        let clip = self.body_scroll.contentView();
        let target = clip.constrainBoundsRect(NSRect::new(
            NSPoint::new(0.0, bottom_offset),
            clip.bounds().size,
        ));
        clip.scrollToPoint(target.origin);
        self.body_scroll.reflectScrolledClipView(&clip);
    }

    fn open_profile_panel(&mut self) {
        let Some(bundle_id) = self.model.bundle_id.clone() else {
            return;
        };
        let profiles = self
            .model
            .profiles
            .iter()
            .filter(|profile| profile.id != self.model.profile)
            .cloned()
            .collect::<Vec<_>>();
        if profiles.is_empty() {
            return;
        }
        let row_h = PROFILE_PICKER_ROW_H;
        let width = self.profile_picker_width(&profiles, row_h);
        let height = row_h * profiles.len() as f64 + PROFILE_PICKER_V_PAD * 2.0;
        let Some(parent_frame) = self.last_panel_frame else {
            return;
        };
        let screens = screen_frames(self.mtm);
        let fallback = fallback_screen(self.mtm, &screens);
        let screen = screens
            .iter()
            .copied()
            .find(|screen| point_in_rect(parent_frame.origin, *screen))
            .unwrap_or(fallback);
        let frame = to_nsrect(L::picker_frame(
            from_nsrect(parent_frame),
            width,
            height,
            from_nsrect(screen),
        ));
        let panel = make_panel(self.mtm, frame);
        apply_panel_background_blur(&panel, self.cfg.macos.background_blur_radius);
        panel.setIgnoresMouseEvents(false);
        let (container, _glass, _background, error) = make_glass_surface(
            self.mtm,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)),
            &self.cfg,
        );
        if let Some(error) = error {
            tracing::warn!(area = "overlay_profile_picker", message = %error);
        }
        for (idx, profile) in profiles.iter().enumerate() {
            let y = height - PROFILE_PICKER_V_PAD - row_h * (idx + 1) as f64;
            let row = ProfileRowView::new(
                self.mtm,
                NSRect::new(
                    NSPoint::new(PROFILE_PICKER_ROW_X, y),
                    NSSize::new(width - PROFILE_PICKER_ROW_H_PAD, row_h),
                ),
                self.actions.clone(),
                bundle_id.clone(),
                profile.clone(),
                self.profile_selection_pending.clone(),
                self.pending_profile_choice.clone(),
            );
            let title = label(
                self.mtm,
                NSRect::new(
                    NSPoint::new(PROFILE_PICKER_TITLE_X, PROFILE_PICKER_TITLE_Y),
                    NSSize::new(
                        width - PROFILE_PICKER_TITLE_H_PAD,
                        row_h - PROFILE_PICKER_TITLE_V_PAD,
                    ),
                ),
                typography::META,
                false,
                self.cfg.core.text.secondary,
            );
            title.setAlignment(NSTextAlignment::Left);
            title.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            let title_text = L::profile_chain_display(
                &profile.display_name,
                &profile.asr_provider,
                &profile.chain_summary,
            );
            self.render_profile_chain_text(
                &title,
                &profile.display_name,
                &title_text,
                NSTextAlignment::Left,
            );
            row.addSubview(&title);
            container.addSubview(&row);
        }
        panel.setContentView(Some(&container));
        panel.orderFrontRegardless();
        self.profile_panel = Some(panel);
    }

    fn profile_picker_width(&self, profiles: &[ProfileChoice], row_h: f64) -> f64 {
        (profiles
            .iter()
            .map(|profile| {
                let title_text = L::profile_chain_display(
                    &profile.display_name,
                    &profile.asr_provider,
                    &profile.chain_summary,
                );
                let title = label(
                    self.mtm,
                    NSRect::new(
                        NSPoint::new(0.0, 0.0),
                        NSSize::new(PROFILE_PICKER_MEASURE_W, row_h),
                    ),
                    typography::META,
                    false,
                    self.cfg.core.text.secondary,
                );
                self.render_profile_chain_text(
                    &title,
                    &profile.display_name,
                    &title_text,
                    NSTextAlignment::Left,
                );
                title.sizeThatFits(NSSize::new(f64::MAX, row_h)).width
            })
            .fold(0.0, f64::max)
            .ceil()
            + PROFILE_PICKER_WIDTH_EXTRA)
            .clamp(PROFILE_PICKER_MIN_W, PROFILE_PICKER_MAX_W)
    }

    fn close_profile_panel(&mut self) {
        if let Some(panel) = self.profile_panel.take() {
            panel.orderOut(None);
        }
        self.profile_close_after = None;
    }

    fn show_body_scroll_indicator(&mut self) {
        if self.body_overflow {
            self.body_scroll_indicator_until =
                Some(Instant::now() + BODY_SCROLL_INDICATOR_HIDE_AFTER);
        }
    }

    fn update_body_scroll_indicator_visibility(&mut self) {
        if !self.body_overflow {
            fade_layer_opacity(&self.body_scroll_indicator, 0.0, BODY_SCROLL_INDICATOR_FADE);
            self.body_scroll_indicator_until = None;
            return;
        }
        let visible = self
            .body_scroll_indicator_until
            .is_some_and(|until| Instant::now() < until);
        fade_layer_opacity(
            &self.body_scroll_indicator,
            if visible {
                BODY_SCROLL_INDICATOR_ALPHA as f32
            } else {
                0.0
            },
            BODY_SCROLL_INDICATOR_FADE,
        );
    }

    fn update_body_scroll_indicator(&mut self, geo: &L::BodyGeometry) {
        if !geo.body_overflow {
            fade_layer_opacity(&self.body_scroll_indicator, 0.0, BODY_SCROLL_INDICATOR_FADE);
            return;
        }

        let visible = self.body_scroll.documentVisibleRect();
        let Some(frame) = geo.scroll_indicator_frame(
            BODY_SCROLL_INDICATOR_MIN_H,
            BODY_SCROLL_INDICATOR_W,
            visible.origin.y,
        ) else {
            return;
        };
        set_layer_frame_now(&self.body_scroll_indicator, to_nsrect(frame));
        if L::scroll_discovery_should_extend(
            self.last_body_visible_y,
            visible.origin.y,
            self.body_programmatic_scroll,
        ) {
            self.show_body_scroll_indicator();
        }
        self.body_programmatic_scroll = false;
        self.last_body_visible_y = Some(visible.origin.y);
        self.update_body_scroll_indicator_visibility();
    }

    fn mouse_inside_profile_picker(&self) -> bool {
        let mouse = NSEvent::mouseLocation();
        self.profile_panel.as_ref().is_some_and(|panel| {
            point_in_rect(mouse, expanded_rect(panel.frame(), PROFILE_PICKER_HIT_SLOP))
        })
    }
}

fn make_body_scroll(mtm: MainThreadMarker, frame: NSRect) -> Retained<BodyScrollView> {
    let this = BodyScrollView::alloc(mtm).set_ivars(BodyScrollIvars::default());
    let scroll: Retained<BodyScrollView> = unsafe { msg_send![super(this), initWithFrame: frame] };
    scroll.setDrawsBackground(false);
    scroll.setBorderType(NSBorderType::NoBorder);
    scroll.setHasVerticalScroller(false);
    scroll.setAutohidesScrollers(false);
    scroll.setScrollerStyle(NSScrollerStyle::Overlay);
    scroll.setWantsLayer(true);

    let text = NSTextView::initWithFrame(
        NSTextView::alloc(mtm),
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(body_text_width(), L::constants::BODY_LINE_H),
        ),
    );
    text.setEditable(false);
    text.setSelectable(false);
    text.setDrawsBackground(false);
    text.setFont(Some(&NSFont::systemFontOfSize(typography::BODY)));
    text.setTextContainerInset(NSSize::new(0.0, BODY_TEXT_VERTICAL_PAD / 2.0));
    unsafe {
        if let Some(container) = text.textContainer() {
            container.setLineFragmentPadding(0.0);
            container.setWidthTracksTextView(true);
            container.setHeightTracksTextView(false);
            container.setContainerSize(NSSize::new(body_text_width(), f64::MAX));
        }
    }
    scroll.setDocumentView(Some(&text));
    scroll
}

#[derive(Debug, Clone, Copy)]
struct LineFragment {
    y: f64,
    h: f64,
    content_bottom: f64,
}

fn measured_body_line_fragments(
    layout: &objc2_app_kit::NSLayoutManager,
    container: &objc2_app_kit::NSTextContainer,
) -> Vec<LineFragment> {
    let glyphs = layout.glyphRangeForTextContainer(container);
    let mut idx = glyphs.location;
    let end = glyphs.location.saturating_add(glyphs.length);
    let mut fragments = Vec::new();
    while idx < end {
        let mut effective = NSRange::new(0, 0);
        let line =
            unsafe { layout.lineFragmentRectForGlyphAtIndex_effectiveRange(idx, &mut effective) };
        if line.size.height > 0.0 {
            let used = unsafe {
                layout.lineFragmentUsedRectForGlyphAtIndex_effectiveRange(idx, std::ptr::null_mut())
            };
            let bounds = layout.boundingRectForGlyphRange_inTextContainer(effective, container);
            let content_bottom = (line.origin.y + line.size.height)
                .max(used.origin.y + used.size.height)
                .max(bounds.origin.y + bounds.size.height);
            fragments.push(LineFragment {
                y: line.origin.y,
                h: line.size.height,
                content_bottom,
            });
        }
        let next = effective.location.saturating_add(effective.length);
        idx = if next > idx { next } else { idx + 1 };
    }
    let extra = layout.extraLineFragmentRect();
    if extra.size.height > 0.0 {
        let used = layout.extraLineFragmentUsedRect();
        let used_has_content = used.size.height > 0.0;
        fragments.push(LineFragment {
            y: extra.origin.y,
            h: extra.size.height,
            content_bottom: if used_has_content {
                (extra.origin.y + extra.size.height).max(used.origin.y + used.size.height)
            } else {
                extra.origin.y + extra.size.height
            },
        });
    }

    fragments
}

fn body_tail_metrics(
    fragments: &[LineFragment],
    max_lines: usize,
    text_inset_y: f64,
) -> Option<L::BodyTailMetrics> {
    if fragments.is_empty() {
        return None;
    }
    let start = fragments.len().saturating_sub(max_lines.max(1));
    let tail = &fragments[start..];
    let top = tail.iter().map(|line| line.y).fold(f64::INFINITY, f64::min);
    let bottom = tail
        .iter()
        .map(|line| line.y + line.h)
        .fold(f64::NEG_INFINITY, f64::max);
    let preceding_content_bottom = fragments[..start]
        .iter()
        .map(|line| line.content_bottom)
        .fold(0.0_f64, f64::max);
    let scroll_offset = top.max(preceding_content_bottom) + text_inset_y;
    (bottom > top).then_some(L::BodyTailMetrics {
        viewport_height: (bottom - top + text_inset_y * 2.0).ceil().max(1.0),
        scroll_offset: scroll_offset.max(0.0),
    })
}

fn make_body_scroll_indicator(color_rgb: u32) -> Retained<CALayer> {
    let layer = CALayer::layer();
    layer.setBackgroundColor(Some(
        &color_from_rgb_alpha(color_rgb, BODY_SCROLL_INDICATOR_ALPHA).CGColor(),
    ));
    layer.setCornerRadius(BODY_SCROLL_INDICATOR_W / 2.0);
    layer.setOpacity(0.0);
    layer
}

fn set_layer_frame_now(layer: &CALayer, frame: NSRect) {
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    layer.setFrame(frame);
    CATransaction::commit();
}

fn set_layer_opacity_now(layer: &CALayer, opacity: f32) {
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    layer.setOpacity(opacity);
    CATransaction::commit();
}

fn fade_layer_opacity(layer: &CALayer, target: f32, duration_s: f64) {
    if (layer.opacity() - target).abs() < BODY_SCROLL_INDICATOR_OPACITY_EPSILON {
        return;
    }
    let animation = CABasicAnimation::animationWithKeyPath(Some(ns_string!("opacity")));
    unsafe {
        animation.setFromValue(Some(&NSNumber::numberWithDouble(layer.opacity() as f64)));
        animation.setToValue(Some(&NSNumber::numberWithDouble(target as f64)));
    }
    animation.setDuration(duration_s);
    layer.addAnimation_forKey(&animation, Some(ns_string!("scroll-indicator-opacity")));
    set_layer_opacity_now(layer, target);
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
        .unwrap_or_else(|| {
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(DEFAULT_SCREEN_W, DEFAULT_SCREEN_H),
            )
        })
}

fn point_in_rect(point: NSPoint, rect: NSRect) -> bool {
    point.x >= rect.origin.x
        && point.x <= rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y <= rect.origin.y + rect.size.height
}

fn expanded_rect(rect: NSRect, amount: f64) -> NSRect {
    NSRect::new(
        NSPoint::new(rect.origin.x - amount, rect.origin.y - amount),
        NSSize::new(
            rect.size.width + amount * 2.0,
            rect.size.height + amount * 2.0,
        ),
    )
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
    fade_window_alpha_for(panel, target, APPEAR_FADE);
}

fn fade_window_alpha_for(panel: &NSPanel, target: f64, duration_s: f64) {
    NSAnimationContext::beginGrouping();
    NSAnimationContext::currentContext().setDuration(duration_s);
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

    #[test]
    fn body_tail_scroll_offset_excludes_previous_content() {
        let metrics = body_tail_metrics(
            &[
                LineFragment {
                    y: 0.0,
                    h: 20.0,
                    content_bottom: 23.0,
                },
                LineFragment {
                    y: 20.0,
                    h: 20.0,
                    content_bottom: 39.0,
                },
                LineFragment {
                    y: 40.0,
                    h: 20.0,
                    content_bottom: 58.0,
                },
            ],
            2,
            2.0,
        )
        .expect("tail metrics");

        assert_eq!(metrics.viewport_height, 44.0);
        assert_eq!(metrics.scroll_offset, 25.0);
    }

    #[test]
    fn body_tail_scroll_offset_keeps_fragment_boundary_when_previous_content_fits() {
        let metrics = body_tail_metrics(
            &[
                LineFragment {
                    y: 0.0,
                    h: 20.0,
                    content_bottom: 19.0,
                },
                LineFragment {
                    y: 20.0,
                    h: 20.0,
                    content_bottom: 39.0,
                },
                LineFragment {
                    y: 40.0,
                    h: 20.0,
                    content_bottom: 59.0,
                },
            ],
            2,
            2.0,
        )
        .expect("tail metrics");

        assert_eq!(metrics.scroll_offset, 22.0);
    }
}
