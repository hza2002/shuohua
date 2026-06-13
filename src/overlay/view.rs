use std::cell::{OnceCell, RefCell};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSAutoresizingMaskOptions, NSBackingStoreType, NSColor, NSFont, NSPanel, NSTextAlignment,
    NSTextField, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
    NSVisualEffectView, NSWindowCollectionBehavior, NSWindowStyleMask,
    NSStatusWindowLevel,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize,
    NSString, NSTimer,
};
use tokio::sync::mpsc;

use crate::overlay::{OverlayCmd, OverlayModel, ToastLevel};

const WIDTH: f64 = 540.0;
const HEIGHT: f64 = 86.0;

#[derive(Default)]
struct DelegateIvars {
    overlay: OnceCell<RefCell<OverlayView>>,
    rx: OnceCell<RefCell<mpsc::UnboundedReceiver<OverlayCmd>>>,
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

            let overlay = OverlayView::new(mtm);
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
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars {
            overlay: OnceCell::new(),
            rx: OnceCell::from(RefCell::new(rx)),
            timer: OnceCell::new(),
        });
        unsafe { msg_send![super(this), init] }
    }
}

pub fn run(rx: mpsc::UnboundedReceiver<OverlayCmd>) {
    let mtm = MainThreadMarker::new().expect("AppKit must run on main thread");
    let app = NSApplication::sharedApplication(mtm);
    let delegate = OverlayDelegate::new(mtm, rx);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
}

struct OverlayView {
    model: OverlayModel,
    panel: Retained<NSPanel>,
    status: Retained<NSTextField>,
    meta: Retained<NSTextField>,
    text: Retained<NSTextField>,
    toast: Retained<NSTextField>,
    recording_started: Option<Instant>,
    toast_until: Option<Instant>,
}

impl OverlayView {
    fn new(mtm: MainThreadMarker) -> Self {
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

        let glass = make_glass_view(mtm);
        glass.setFrame(root_frame());
        glass.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        root.addSubview(&glass);

        let status = label(mtm, 18.0, 48.0, 300.0, 22.0, 13.0, true);
        let meta = label(mtm, 320.0, 48.0, 190.0, 22.0, 12.0, false);
        let text = label(mtm, 18.0, 18.0, 500.0, 24.0, 15.0, false);
        let toast = label(mtm, 150.0, -30.0, 240.0, 24.0, 12.0, false);
        toast.setHidden(true);

        root.addSubview(&status);
        root.addSubview(&meta);
        root.addSubview(&text);
        root.addSubview(&toast);
        panel.setContentView(Some(&root));
        panel.orderOut(None);

        Self {
            model: OverlayModel::default(),
            panel,
            status,
            meta,
            text,
            toast,
            recording_started: None,
            toast_until: None,
        }
    }

    fn apply(&mut self, cmd: OverlayCmd) {
        match &cmd {
            OverlayCmd::SetState { state } => match state {
                crate::overlay::OverlayState::Recording => {
                    self.recording_started = Some(Instant::now());
                }
                crate::overlay::OverlayState::Idle | crate::overlay::OverlayState::Error => {
                    self.recording_started = None;
                }
                _ => {}
            },
            OverlayCmd::Toast { ttl_ms, .. } => {
                self.toast_until = Some(Instant::now() + Duration::from_millis(*ttl_ms as u64));
            }
            OverlayCmd::Hide => {
                self.recording_started = None;
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

    fn render(&self) {
        if self.model.visible {
            self.panel.setAlphaValue(0.96);
            self.panel.makeKeyAndOrderFront(None);
        } else {
            self.panel.orderOut(None);
            return;
        }

        let display_text = self.model.display_text();
        let dot = colored_dot(self.model.state_color);
        let dur = format_duration(self.model.dur_ms);
        let status = format!(
            "{dot} {} · {dur} · {}字",
            self.model.state_label,
            display_text.chars().count()
        );
        self.status.setStringValue(&NSString::from_str(&status));

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
        self.meta.setStringValue(&NSString::from_str(&meta));
        self.text.setStringValue(&NSString::from_str(&display_text));

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

fn make_glass_view(mtm: MainThreadMarker) -> Retained<NSView> {
    if let Some(cls) = AnyClass::get(c"NSGlassEffectView") {
        let glass: Retained<NSView> = unsafe { msg_send![msg_send![cls, alloc], initWithFrame: root_frame()] };
        set_glass_variant(&glass, 19);
        return glass;
    }

    let visual = NSVisualEffectView::new(mtm);
    visual.setFrame(root_frame());
    visual.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    visual.setMaterial(NSVisualEffectMaterial::HUDWindow);
    visual.setState(NSVisualEffectState::Active);
    visual.into_super()
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

fn colored_dot(rgb: u32) -> &'static str {
    match rgb {
        0xFF3B30 | 0xFF453A => "●",
        0xFF9F0A | 0xFFD60A => "●",
        0x0A84FF => "●",
        _ => "●",
    }
}

fn format_duration(ms: u64) -> String {
    if ms < 10_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{:02}:{:02}", ms / 60_000, (ms / 1000) % 60)
    }
}
