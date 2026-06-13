use std::cell::{OnceCell, RefCell};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSAutoresizingMaskOptions,
    NSBackingStoreType, NSColor, NSFont, NSImage, NSImageView, NSLineBreakMode, NSPanel, NSScreen,
    NSTextAlignment, NSTextField, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindowCollectionBehavior, NSWindowStyleMask,
    NSStatusWindowLevel,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize,
    NSString, NSTimer,
};
use objc2_quartz_core::{kCATransitionFade, CAMediaTiming, CATransition};
use tokio::sync::mpsc;

use crate::config::{OverlayCfg, OverlayPosition};
use crate::overlay::{OverlayCmd, OverlayModel, OverlayState, TextKind, ToastLevel};

const WIDTH: f64 = 540.0;
const HEIGHT: f64 = 86.0;
const TEXT_WIDTH: f64 = 500.0;
const CHARS_PER_LINE: usize = 34;
const TEXT_LINE_HEIGHT: f64 = 22.0;
const WINDOW_MARGIN: f64 = 16.0;

#[derive(Default)]
struct DelegateIvars {
    overlay: OnceCell<RefCell<OverlayView>>,
    rx: OnceCell<RefCell<mpsc::UnboundedReceiver<OverlayCmd>>>,
    cfg: OnceCell<OverlayCfg>,
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
        cfg: OverlayCfg,
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

pub fn run(rx: mpsc::UnboundedReceiver<OverlayCmd>, cfg: OverlayCfg) {
    let mtm = MainThreadMarker::new().expect("AppKit must run on main thread");
    let app = NSApplication::sharedApplication(mtm);
    let delegate = OverlayDelegate::new(mtm, rx, cfg);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
}

struct OverlayView {
    mtm: MainThreadMarker,
    cfg: OverlayCfg,
    model: OverlayModel,
    panel: Retained<NSPanel>,
    root: Retained<NSView>,
    glass: Retained<NSView>,
    scrim: Retained<NSView>,
    wave: Retained<NSImageView>,
    dot: Retained<NSTextField>,
    status: Retained<NSTextField>,
    meta: Retained<NSTextField>,
    text: Retained<NSTextField>,
    toast: Retained<NSTextField>,
    recording_started: Option<Instant>,
    last_text_update: Option<Instant>,
    toast_until: Option<Instant>,
    last_panel_frame: Option<NSRect>,
    last_visible_text: String,
    last_status_text: String,
    last_meta_text: String,
}

impl OverlayView {
    fn new(mtm: MainThreadMarker, cfg: OverlayCfg) -> Self {
        let frame = NSRect::new(NSPoint::new(80.0, 860.0), NSSize::new(WIDTH, HEIGHT));
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

        let root = NSView::new(mtm);
        root.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT)));
        root.setWantsLayer(true);
        if let Some(layer) = root.layer() {
            layer.setCornerRadius(18.0);
            layer.setMasksToBounds(true);
        }

        let glass = make_glass_view(mtm, cfg.glass_variant);
        set_glass_tint(&glass, cfg.background_rgb, cfg.background_alpha);
        glass.setFrame(root_frame());
        glass.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        root.addSubview(&glass);

        let scrim = NSView::new(mtm);
        scrim.setFrame(root_frame());
        scrim.setWantsLayer(true);
        if let Some(layer) = scrim.layer() {
            let color = color_from_rgb_alpha(cfg.background_rgb, cfg.background_alpha);
            layer.setBackgroundColor(Some(&color.CGColor()));
        }
        root.addSubview(&scrim);

        let wave = make_wave_icon(mtm, 0.35);
        wave.setHidden(true);
        let dot = label(mtm, 18.0, 48.0, 14.0, 22.0, 13.0, true);
        let status = label(mtm, 38.0, 48.0, 280.0, 22.0, 13.0, true);
        let meta = label(mtm, 320.0, 48.0, 190.0, 22.0, 12.0, false);
        let text = label(mtm, 18.0, 18.0, TEXT_WIDTH, 24.0, 15.0, false);
        text.setUsesSingleLineMode(false);
        text.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        let toast = label(mtm, 150.0, -30.0, 240.0, 24.0, 12.0, false);
        toast.setHidden(true);

        root.addSubview(&wave);
        root.addSubview(&dot);
        root.addSubview(&status);
        root.addSubview(&meta);
        root.addSubview(&text);
        root.addSubview(&toast);
        panel.setContentView(Some(&root));
        panel.orderOut(None);

        Self {
            mtm,
            cfg,
            model: OverlayModel::default(),
            panel,
            root,
            glass,
            scrim,
            wave,
            dot,
            status,
            meta,
            text,
            toast,
            recording_started: None,
            last_text_update: None,
            toast_until: None,
            last_panel_frame: None,
            last_visible_text: String::new(),
            last_status_text: String::new(),
            last_meta_text: String::new(),
        }
    }

    fn apply(&mut self, cmd: OverlayCmd) {
        match &cmd {
            OverlayCmd::SetState { state } => match state {
                OverlayState::Recording => {
                    self.recording_started = Some(Instant::now());
                    self.last_text_update = Some(Instant::now());
                }
                OverlayState::Idle | OverlayState::Error => {
                    self.recording_started = None;
                    self.last_text_update = None;
                }
                _ => {}
            },
            OverlayCmd::SetText { kind, .. } if *kind == TextKind::Partial => {
                self.last_text_update = Some(Instant::now());
            }
            OverlayCmd::AppendSegment { .. } => {
                self.last_text_update = Some(Instant::now());
            }
            OverlayCmd::Toast { ttl_ms, .. } => {
                self.toast_until = Some(Instant::now() + Duration::from_millis(*ttl_ms as u64));
            }
            OverlayCmd::Hide => {
                self.recording_started = None;
                self.last_text_update = None;
                self.toast_until = None;
            }
            _ => {}
        }
        self.model.apply(cmd);
        self.render();
    }

    fn tick(&mut self) {
        if let Some(started) = self.recording_started {
            self.model.dur_ms = started.elapsed().as_millis() as u64;
        }
        if self.toast_until.is_some_and(|until| Instant::now() >= until) {
            self.model.toast = None;
            self.toast_until = None;
        }
        self.render();
    }

    fn render(&mut self) {
        if self.model.visible {
            let (_, lines) = display_text_plan(
                &self.model.display_text(),
                self.cfg.max_text_lines,
                CHARS_PER_LINE,
            );
            let height = HEIGHT + (lines.saturating_sub(1) as f64 * TEXT_LINE_HEIGHT);
            self.layout(height, lines);
            self.place(height);
            self.panel.setAlphaValue(0.96);
            self.panel.makeKeyAndOrderFront(None);
        } else {
            self.panel.orderOut(None);
            return;
        }

        let full_text = self.model.display_text();
        let (display_text, _) =
            display_text_plan(&full_text, self.cfg.max_text_lines, CHARS_PER_LINE);
        let (state, label, color_rgb) = self.effective_state();
        let dur = format_duration(self.model.dur_ms);
        let status = format!("{label} · {dur} · {}字", full_text.chars().count());
        self.render_wave(state, color_rgb);
        self.dot.setStringValue(ns_string!("●"));
        self.dot.setTextColor(Some(&color_from_rgb(color_rgb)));
        if self.last_status_text != status {
            fade_view(&self.status, 0.16);
            self.status.setStringValue(&NSString::from_str(&status));
            self.last_status_text = status;
        }

        let app = self
            .model
            .app_name
            .as_deref()
            .or(self.model.bundle_id.as_deref())
            .unwrap_or("");
        let meta = if self.model.chain_summary.is_empty() {
            app.to_string()
        } else if app.is_empty() {
            self.model.chain_summary.clone()
        } else {
            format!("{app}  ·  {}", self.model.chain_summary)
        };
        if self.last_meta_text != meta {
            fade_view(&self.meta, 0.16);
            self.meta.setStringValue(&NSString::from_str(&meta));
            self.last_meta_text = meta;
        }
        if self.last_visible_text != display_text {
            fade_view(&self.text, 0.10);
            self.text.setStringValue(&NSString::from_str(&display_text));
            self.last_visible_text = display_text;
        }

        if let Some(toast) = &self.model.toast {
            self.toast.setStringValue(&NSString::from_str(&toast.text));
            let color = match toast.level {
                ToastLevel::Info => NSColor::labelColor(),
                ToastLevel::Warn => {
                    NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 0.76, 0.0, 1.0)
                }
                ToastLevel::Error => {
                    NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 0.23, 0.19, 1.0)
                }
            };
            self.toast.setTextColor(Some(&color));
            self.toast.setHidden(false);
        } else {
            self.toast.setHidden(true);
        }
    }

    fn effective_state(&self) -> (OverlayState, String, u32) {
        let thinking_delay = Duration::from_millis(self.cfg.thinking_delay_ms);
        if self.model.state == OverlayState::Recording
            && self
                .last_text_update
                .is_some_and(|last| last.elapsed() >= thinking_delay)
        {
            return (
                OverlayState::Thinking,
                crate::t!(OverlayState::Thinking.label_key()),
                OverlayState::Thinking.color_rgb(),
            );
        }
        (
            self.model.state,
            self.model.state_label.clone(),
            self.model.state_color,
        )
    }

    fn render_wave(&self, state: OverlayState, color_rgb: u32) {
        let is_recording = state == OverlayState::Recording;
        self.wave.setHidden(!is_recording);
        if !is_recording {
            return;
        }

        let phase = self
            .recording_started
            .map(|started| ((started.elapsed().as_millis() / 160) % 6) as usize)
            .unwrap_or(0);
        let values = [0.25, 0.45, 0.75, 1.0, 0.65, 0.35];
        if let Some(image) = wave_image(values[phase]) {
            self.wave.setImage(Some(&image));
        }
        let alpha = [0.55, 0.68, 0.86, 1.0, 0.78, 0.62][phase];
        self.wave
            .setContentTintColor(Some(&color_from_rgb_alpha(color_rgb, alpha)));
    }

    fn layout(&mut self, height: f64, lines: usize) {
        let top_offset = height - HEIGHT;
        self.root.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(WIDTH, height),
        ));
        self.glass.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(WIDTH, height),
        ));
        self.scrim.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(WIDTH, height),
        ));
        let wave_visible = self.effective_state().0 == OverlayState::Recording;
        self.wave.setFrame(NSRect::new(
            NSPoint::new(17.0, 48.0 + top_offset),
            NSSize::new(18.0, 22.0),
        ));
        let dot_x = if wave_visible { 40.0 } else { 18.0 };
        let status_x = if wave_visible { 60.0 } else { 38.0 };
        let status_w = if wave_visible { 258.0 } else { 280.0 };
        self.dot.setFrame(NSRect::new(
            NSPoint::new(dot_x, 48.0 + top_offset),
            NSSize::new(14.0, 22.0),
        ));
        self.status.setFrame(NSRect::new(
            NSPoint::new(status_x, 48.0 + top_offset),
            NSSize::new(status_w, 22.0),
        ));
        self.meta.setFrame(NSRect::new(
            NSPoint::new(320.0, 48.0 + top_offset),
            NSSize::new(190.0, 22.0),
        ));
        self.text.setFrame(NSRect::new(
            NSPoint::new(18.0, 18.0),
            NSSize::new(TEXT_WIDTH, 24.0 + (lines.saturating_sub(1) as f64 * TEXT_LINE_HEIGHT)),
        ));
        self.toast.setFrame(NSRect::new(
            NSPoint::new(150.0, -30.0),
            NSSize::new(240.0, 24.0),
        ));
    }

    fn place(&mut self, height: f64) {
        let screen = NSScreen::mainScreen(self.mtm)
            .map(|screen| screen.frame())
            .unwrap_or_else(|| NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0)));
        let anchor =
            crate::focused_window_darwin::focused_window_frame(screen.size.height).unwrap_or(screen);
        let frame = panel_frame(anchor, self.cfg.position, WIDTH, height, screen);
        let animate = self.last_panel_frame.is_some();
        if self
            .last_panel_frame
            .is_none_or(|last| !rect_nearly_eq(last, frame))
        {
            self.panel.setFrame_display_animate(frame, true, animate);
            self.last_panel_frame = Some(frame);
        }
    }
}

fn root_frame() -> NSRect {
    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT))
}

fn label(
    mtm: MainThreadMarker,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    font_size: f64,
    bold: bool,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(ns_string!(""), mtm);
    field.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)));
    field.setDrawsBackground(false);
    field.setBezeled(false);
    field.setEditable(false);
    field.setSelectable(false);
    field.setWantsLayer(true);
    field.setTextColor(Some(&NSColor::labelColor()));
    let font = if bold {
        NSFont::boldSystemFontOfSize(font_size)
    } else {
        NSFont::systemFontOfSize(font_size)
    };
    field.setFont(Some(&font));
    field.setAlignment(NSTextAlignment::Left);
    field
}

fn make_glass_view(mtm: MainThreadMarker, variant: i64) -> Retained<NSView> {
    if let Some(cls) = AnyClass::get(c"NSGlassEffectView") {
        let glass: Retained<NSView> =
            unsafe { msg_send![msg_send![cls, alloc], initWithFrame: root_frame()] };
        set_glass_variant(&glass, variant);
        return glass;
    }

    let visual = NSVisualEffectView::new(mtm);
    visual.setFrame(root_frame());
    visual.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    visual.setMaterial(NSVisualEffectMaterial::HUDWindow);
    visual.setState(NSVisualEffectState::Active);
    visual.into_super()
}

fn set_glass_tint(view: &NSView, rgb: u32, alpha: f64) {
    unsafe {
        let obj: &AnyObject = msg_send![view, self];
        let sel = sel!(setTintColor:);
        let responds: bool = msg_send![obj, respondsToSelector: sel];
        if responds {
            let color = color_from_rgb_alpha(rgb, alpha.clamp(0.0, 1.0));
            let _: () = msg_send![obj, setTintColor: &*color];
        }
    }
}

fn set_glass_variant(view: &NSView, variant: i64) {
    unsafe {
        let obj: &AnyObject = msg_send![view, self];
        let private_sel = sel!(set_variant:);
        let public_sel = sel!(setVariant:);
        let responds_private: bool = msg_send![obj, respondsToSelector: private_sel];
        let responds_public: bool = msg_send![obj, respondsToSelector: public_sel];
        if responds_private {
            let _: () = msg_send![obj, set_variant: variant];
        } else if responds_public {
            let _: () = msg_send![obj, setVariant: variant];
        }
    }
}

fn make_wave_icon(mtm: MainThreadMarker, value: f64) -> Retained<NSImageView> {
    let image = wave_image(value).unwrap_or_else(|| {
        NSImage::imageWithSystemSymbolName_accessibilityDescription(
            ns_string!("waveform"),
            Some(ns_string!("Recording")),
        )
        .expect("system waveform symbol should exist")
    });
    let view = NSImageView::imageViewWithImage(&image, mtm);
    view.setContentTintColor(Some(&NSColor::labelColor()));
    view
}

fn wave_image(value: f64) -> Option<Retained<NSImage>> {
    NSImage::imageWithSystemSymbolName_variableValue_accessibilityDescription(
        ns_string!("waveform"),
        value.clamp(0.0, 1.0),
        Some(ns_string!("Recording")),
    )
}

fn color_from_rgb(rgb: u32) -> Retained<NSColor> {
    color_from_rgb_alpha(rgb, 1.0)
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

fn panel_frame(
    anchor: NSRect,
    position: OverlayPosition,
    width: f64,
    height: f64,
    screen: NSRect,
) -> NSRect {
    let x = anchor.origin.x + (anchor.size.width - width) / 2.0;
    let y = match position {
        OverlayPosition::Top => anchor.origin.y + anchor.size.height - height - WINDOW_MARGIN,
        OverlayPosition::Middle => anchor.origin.y + (anchor.size.height - height) / 2.0,
        OverlayPosition::Bottom => anchor.origin.y + WINDOW_MARGIN,
    };
    let x = clamp(
        x,
        screen.origin.x + WINDOW_MARGIN,
        screen.origin.x + screen.size.width - width - WINDOW_MARGIN,
    );
    let y = clamp(
        y,
        screen.origin.y + WINDOW_MARGIN,
        screen.origin.y + screen.size.height - height - WINDOW_MARGIN,
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
    if ms < 10_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{:02}:{:02}", ms / 60_000, (ms / 1000) % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
