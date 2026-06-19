use std::ffi::{c_char, c_int, c_void};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBackingStoreType, NSColor, NSGlassEffectView,
    NSGlassEffectViewStyle, NSPanel, NSStatusWindowLevel, NSView, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};

use crate::config::theme::{EffectiveOverlayCfg, GlassStyle};
use crate::overlay::layout as L;

pub(super) fn root_frame() -> NSRect {
    NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(L::constants::WIDTH, L::constants::BASE_HEIGHT),
    )
}

pub(super) fn make_panel(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSPanel> {
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

pub(super) fn apply_panel_background_blur(panel: &NSPanel, radius: i64) {
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

/// 构造 chrome。返回 (labels 容器, glass 句柄或 None, 可控色层, 初始化期错误).
///
/// 关键认知（v2 调整）：`NSGlassEffectView.contentView` 是 **"嵌在 glass 里"** 的语义 ——
/// 像琥珀封蝇，contentView 视觉上**在 glass 材质后面**，会被材质遮罩/模糊。所以 labels
/// **不能**塞进 glass.contentView，否则人眼看像在 glass 下面。
///
/// 正确做法：panel.contentView = 一个普通 root NSView；root 里 **glass 在底层**（先 addSubview），
/// labels 是 glass 的**兄弟**（后 addSubview，z-order 自然在上）。Apple 的 "arbitrary subviews
/// 行为未定" 警告只针对**直接 addSubview 到 glass**，不针对 glass 的兄弟。
pub(super) fn build_chrome(
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
            super::debug::dump_glass_selectors(&glass);
            super::debug::probe_glass_state_ranges(&glass);
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
pub(super) fn apply_glass_settings(
    glass: &NSGlassEffectView,
    cfg: &EffectiveOverlayCfg,
) -> Vec<&'static str> {
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

pub(super) fn make_background_layer(
    mtm: MainThreadMarker,
    cfg: &EffectiveOverlayCfg,
) -> Retained<NSView> {
    let background = NSView::new(mtm);
    background.setFrame(root_frame());
    background.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    background.setWantsLayer(true);
    apply_background_settings(&background, cfg);
    background
}

pub(super) fn apply_background_settings(background: &NSView, cfg: &EffectiveOverlayCfg) {
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
            super::debug::trace(format!(
                "dispatched {} <- {value}",
                private_name.to_string_lossy()
            ));
            return true;
        }
        let public = Sel::register(public_name);
        if msg_send![obj, respondsToSelector: public] {
            let _: () = obj.send_message(public, (value,));
            #[cfg(debug_assertions)]
            super::debug::trace(format!(
                "dispatched {} <- {value}",
                public_name.to_string_lossy()
            ));
            return true;
        }
        #[cfg(debug_assertions)]
        super::debug::trace(format!(
            "missing {} / {}",
            private_name.to_string_lossy(),
            public_name.to_string_lossy()
        ));
    }
    false
}

pub(super) fn color_from_rgb_alpha(rgb: u32, alpha: f64) -> Retained<NSColor> {
    let r = ((rgb >> 16) & 0xff) as f64 / 255.0;
    let g = ((rgb >> 8) & 0xff) as f64 / 255.0;
    let b = (rgb & 0xff) as f64 / 255.0;
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, alpha.clamp(0.0, 1.0))
}
