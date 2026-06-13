//! Minimal Liquid Glass demo: a borderless NSPanel whose content is an
//! NSGlassEffectView. The window itself looks like a floating liquid glass
//! pill that samples the desktop / windows beneath it.
//!
//! Requires macOS 26 (Tahoe) or newer.

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{class, msg_send, MainThreadMarker, MainThreadOnly};

use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSFont, NSPanel,
    NSScreen, NSTextAlignment, NSTextField, NSView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

const WINDOW_W: f64 = 220.0;
const WINDOW_H: f64 = 90.0;
const CORNER: f64 = 22.0;
const TILE_COLS: i64 = 6;
const TILE_GAP: f64 = 16.0;

const VARIANT_NAMES: &[&str] = &[
    "regular", "clear", "dock", "appIcons",
    "widgets", "text", "avplayer", "facetime",
    "controlCenter", "notificationCenter", "monogram", "bubbles",
    "identity", "focusBorder", "focusPlatter", "keyboard",
    "sidebar", "abuttedSidebar", "inspector", "control",
    "loupe", "slider", "camera", "cartouchePopover",
];

fn main() {
    let mtm = MainThreadMarker::new().expect("must run on main thread");

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    // Variant: numeric env var (default = 1 = "clear"). -1 means leave default.
    let variant: i64 = std::env::var("JT_VARIANT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    // Tile index for grid layout: when JT_TILE_INDEX is set, the window is
    // placed in a TILE_COLS-wide grid filling the screen.
    let tile_index: Option<i64> = std::env::var("JT_TILE_INDEX")
        .ok()
        .and_then(|s| s.parse().ok());

    let vf = NSScreen::mainScreen(mtm)
        .map(|s| s.visibleFrame())
        .unwrap_or(NSRect {
            origin: NSPoint { x: 0.0, y: 0.0 },
            size: NSSize { width: 1440.0, height: 900.0 },
        });

    let origin = match tile_index {
        Some(idx) => {
            let col = idx.rem_euclid(TILE_COLS);
            let row = idx.div_euclid(TILE_COLS);
            NSPoint {
                x: vf.origin.x + TILE_GAP + col as f64 * (WINDOW_W + TILE_GAP),
                y: vf.origin.y + vf.size.height - WINDOW_H - TILE_GAP
                    - row as f64 * (WINDOW_H + TILE_GAP),
            }
        }
        None => NSPoint {
            x: vf.origin.x + (vf.size.width - WINDOW_W) / 2.0,
            y: vf.origin.y + vf.size.height - WINDOW_H - 80.0,
        },
    };
    let panel_rect = NSRect {
        origin,
        size: NSSize {
            width: WINDOW_W,
            height: WINDOW_H,
        },
    };

    let panel: Retained<NSPanel> = unsafe {
        let allocated = NSPanel::alloc(mtm);
        msg_send![
            allocated,
            initWithContentRect: panel_rect,
            styleMask: NSWindowStyleMask::Borderless,
            backing: NSBackingStoreType::Buffered,
            defer: false,
        ]
    };

    unsafe {
        panel.setOpaque(false);
        let clear = NSColor::clearColor();
        panel.setBackgroundColor(Some(&clear));
        // Match Go overlay: no system shadow on the panel itself.
        panel.setHasShadow(false);
        panel.setMovableByWindowBackground(true);
        // NSStatusWindowLevel = 25 — float above ordinary windows but below menubar.
        let _: () = msg_send![&*panel, setLevel: 25i64];
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary,
        );
    }

    let content_rect = NSRect {
        origin: NSPoint { x: 0.0, y: 0.0 },
        size: NSSize {
            width: WINDOW_W,
            height: WINDOW_H,
        },
    };

    // Root view: transparent, rounded, hosts the glass effect view.
    // (Architecture note: NSGlassEffectView must be a subview, not the panel's
    //  contentView directly, otherwise AppKit adds a second legibility blur layer.)
    let root: Retained<NSView> = unsafe {
        let alloc = NSView::alloc(mtm);
        msg_send![alloc, initWithFrame: content_rect]
    };
    unsafe {
        root.setWantsLayer(true);
        let layer: *mut AnyObject = msg_send![&*root, layer];
        let _: () = msg_send![layer, setCornerRadius: CORNER];
        let _: () = msg_send![layer, setMasksToBounds: true];
    }

    // NSGlassEffectView (macOS 26+). Public class but not yet in objc2-app-kit;
    // load via the Objective-C runtime.
    let glass_class = AnyClass::get(c"NSGlassEffectView")
        .expect("NSGlassEffectView unavailable — needs macOS 26 (Tahoe)");
    let glass: Retained<NSView> = unsafe {
        let allocated: *mut AnyObject = msg_send![glass_class, alloc];
        let inited: *mut NSView = msg_send![allocated, initWithFrame: content_rect];
        let glass = Retained::from_raw(inited).expect("glass init returned nil");
        // Corner geometry via CALayer (matches electron-liquid-glass + Go overlay).
        let _: () = msg_send![&*glass, setWantsLayer: true];
        let glass_layer: *mut AnyObject = msg_send![&*glass, layer];
        let _: () = msg_send![glass_layer, setCornerRadius: CORNER];
        let _: () = msg_send![glass_layer, setMasksToBounds: true];
        // NSViewWidthSizable(2) | NSViewHeightSizable(16) = 18 — fill the root.
        let _: () = msg_send![&*glass, setAutoresizingMask: 18u64];

        let sel_var = Sel::register(c"set_variant:");
        let var_ok: bool = msg_send![&*glass, respondsToSelector: sel_var];
        if var_ok && variant >= 0 {
            let _: () = msg_send![&*glass, set_variant: variant];
        }
        glass
    };

    // Label as a SIBLING of the glass view (above it in z-order), not as
    // glass.contentView. Setting glass.contentView triggers AppKit's
    // "legibility treatment" pass which dampens the material into vibrancy.
    // Matches electron-liquid-glass + the Go overlay's structure.
    let variant_name = VARIANT_NAMES
        .get(variant.max(0) as usize)
        .copied()
        .unwrap_or("?");
    let title_str = if variant < 0 {
        "default\n(no set_variant:)".to_string()
    } else {
        format!("{}\n{}", variant, variant_name)
    };
    let title = NSString::from_str(&title_str);
    let label: Retained<NSTextField> = unsafe {
        let cls = class!(NSTextField);
        let label_ptr: *mut NSTextField = msg_send![cls, labelWithString: &*title];
        Retained::retain(label_ptr).expect("labelWithString returned nil")
    };
    unsafe {
        let font = NSFont::systemFontOfSize(15.0);
        label.setFont(Some(&font));
        label.setAlignment(NSTextAlignment::Center);
        let _: () = msg_send![&*label, setUsesSingleLineMode: false];
        let _: () = msg_send![&*label, setMaximumNumberOfLines: 2i64];
        let _: () = msg_send![&*label, setFrame: content_rect];
    }

    // Hierarchy: panel.contentView = root; glass added below, label above.
    root.addSubview(&glass);
    root.addSubview(&label);
    panel.setContentView(Some(&root));
    panel.makeKeyAndOrderFront(None);
    app.activate();

    app.run();
}
