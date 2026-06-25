use anyhow::{anyhow, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{GetLastError, COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, GetStockObject,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_TOP, DT_WORDBREAK, HBRUSH, HDC, HGDIOBJ, PAINTSTRUCT, TRANSPARENT,
    WHITE_BRUSH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, LoadCursorW, PeekMessageW, PostQuitMessage, RegisterClassW,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage,
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, HTTRANSPARENT, IDC_ARROW,
    LWA_ALPHA, MSG, PM_REMOVE, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    SW_HIDE, SW_SHOWNOACTIVATE, WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_NCHITTEST, WM_PAINT,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::overlay::layout as L;
use crate::overlay::{OverlayCmd, OverlayModel, OverlayReceiver};
use crate::platform::capability::{
    CapabilityId, CapabilityStatus, CapabilityStatusKind, PlatformKind,
};

const CLASS_NAME: &str = "ShuohuaOverlayWindow";
const POLL_INTERVAL: Duration = Duration::from_millis(16);

pub(super) fn renderer_capabilities() -> &'static [CapabilityStatus] {
    &WINDOWS_RENDERER_CAPABILITIES
}

pub(super) fn run(
    rx: OverlayReceiver,
    cfg: crate::config::theme::EffectiveOverlayCfg,
) -> Result<()> {
    WindowsOverlay::new(rx, cfg)?.run()
}

struct WindowsOverlay {
    hwnd: HWND,
    rx: OverlayReceiver,
    cfg: crate::config::theme::EffectiveOverlayCfg,
    model: OverlayModel,
    visible: bool,
    quit: bool,
}

impl WindowsOverlay {
    fn new(
        rx: OverlayReceiver,
        cfg: crate::config::theme::EffectiveOverlayCfg,
    ) -> Result<Box<Self>> {
        register_window_class()?;
        let mut overlay = Box::new(Self {
            hwnd: null_mut(),
            rx,
            model: OverlayModel::new(&cfg.core.state),
            cfg,
            visible: false,
            quit: false,
        });
        let raw = overlay.as_mut() as *mut Self;
        let hwnd = create_overlay_window(raw)?;
        overlay.hwnd = hwnd;
        Ok(overlay)
    }

    fn run(&mut self) -> Result<()> {
        let mut msg = MSG::default();
        while !self.quit {
            while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
                if msg.message == windows_sys::Win32::UI::WindowsAndMessaging::WM_QUIT {
                    self.quit = true;
                    break;
                }
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            self.drain_commands();
            if self.model.tick(Instant::now(), &self.cfg.core.state)
                == crate::overlay::model::TickOutcome::Hide
            {
                self.hide();
            }
            std::thread::sleep(POLL_INTERVAL);
        }

        if !self.hwnd.is_null() {
            unsafe {
                DestroyWindow(self.hwnd);
            }
            self.hwnd = null_mut();
        }
        Ok(())
    }

    fn drain_commands(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(OverlayCmd::Quit) => {
                    self.quit = true;
                    return;
                }
                Ok(OverlayCmd::Hide) | Ok(OverlayCmd::Dismiss) => {
                    self.model.apply(OverlayCmd::Hide, &self.cfg.core.state);
                    self.hide();
                }
                Ok(OverlayCmd::ReloadConfig { cfg }) => {
                    self.cfg = cfg;
                    self.invalidate();
                }
                Ok(cmd) => {
                    self.model.apply(cmd, &self.cfg.core.state);
                    self.sync_visibility();
                    self.invalidate();
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => return,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    self.quit = true;
                    return;
                }
            }
        }
    }

    fn sync_visibility(&mut self) {
        if self.model.visible {
            self.show();
        } else {
            self.hide();
        }
    }

    fn show(&mut self) {
        if self.visible {
            return;
        }
        let frame = screen_panel_frame(&self.cfg);
        unsafe {
            SetWindowPos(
                self.hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::HWND_TOPMOST,
                frame.x as i32,
                frame.y as i32,
                frame.w as i32,
                frame.h as i32,
                SWP_NOACTIVATE,
            );
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
        self.visible = true;
    }

    fn hide(&mut self) {
        if !self.visible {
            return;
        }
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
        }
        self.visible = false;
    }

    fn invalidate(&self) {
        unsafe {
            InvalidateRect(self.hwnd, std::ptr::null(), 1);
        }
    }

    unsafe fn paint(&self, hdc: HDC) {
        let rect = RECT {
            left: 0,
            top: 0,
            right: L::constants::WIDTH as i32,
            bottom: overlay_height(&self.model, &self.cfg) as i32,
        };
        let brush = CreateSolidBrush(to_colorref(self.cfg.core.background_rgb));
        FillRect(hdc, &rect, brush);
        DeleteObject(brush as HGDIOBJ);

        SetBkMode(hdc, TRANSPARENT as i32);
        let old_font = SelectObject(hdc, GetStockObject(17));
        draw_text(
            hdc,
            &mut RECT {
                left: 16,
                top: 11,
                right: 128,
                bottom: 34,
            },
            &self.model.state_label,
            self.model.state_color,
            DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );

        let app = self.model.app_name.as_deref().unwrap_or_default();
        let stats = L::stats_text(
            &L::format_duration(self.model.dur_ms),
            &crate::t!("overlay.word_count", n = self.model.words),
            app,
        );
        draw_text(
            hdc,
            &mut RECT {
                left: 132,
                top: 11,
                right: 430,
                bottom: 34,
            },
            &stats,
            self.cfg.core.text.secondary,
            DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );

        let (meta, meta_color) = if let Some(notice) = &self.model.notice {
            (notice.text.as_str(), self.cfg.core.text.notice)
        } else {
            (
                self.model.chain_summary.as_str(),
                self.cfg.core.text.tertiary,
            )
        };
        draw_text(
            hdc,
            &mut RECT {
                left: 430,
                top: 11,
                right: L::constants::WIDTH as i32 - 16,
                bottom: 34,
            },
            meta,
            meta_color,
            DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );

        let text_color = if self.model.error_text.is_empty() {
            self.cfg.core.text.primary
        } else {
            self.cfg.core.text.error
        };
        let (text, _) = L::display_text_plan(
            &self.model.display_text(),
            self.cfg.core.max_text_lines,
            L::constants::CHARS_PER_LINE,
        );
        draw_text(
            hdc,
            &mut RECT {
                left: 16,
                top: 36,
                right: L::constants::WIDTH as i32 - 16,
                bottom: rect.bottom - 8,
            },
            &text,
            text_color,
            DT_LEFT | DT_TOP | DT_WORDBREAK | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        SelectObject(hdc, old_font);
    }
}

impl Drop for WindowsOverlay {
    fn drop(&mut self) {
        if !self.hwnd.is_null() {
            unsafe {
                DestroyWindow(self.hwnd);
            }
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let create = &*(lparam as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            0
        }
        WM_NCHITTEST => HTTRANSPARENT as isize,
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let ptr =
                windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, GWLP_USERDATA)
                    as *const WindowsOverlay;
            if !ptr.is_null() {
                (*ptr).paint(hdc);
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn register_window_class() -> Result<()> {
    let class_name = wide_null(CLASS_NAME);
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    if module.is_null() {
        return Err(last_error("GetModuleHandleW"));
    }
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: module,
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        hbrBackground: unsafe { GetStockObject(WHITE_BRUSH) as HBRUSH },
        lpszClassName: class_name.as_ptr(),
        ..Default::default()
    };
    let atom = unsafe { RegisterClassW(&class) };
    if atom == 0 {
        let code = unsafe { GetLastError() };
        const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
        if code != ERROR_CLASS_ALREADY_EXISTS {
            return Err(last_error("RegisterClassW"));
        }
    }
    Ok(())
}

fn create_overlay_window(overlay: *mut WindowsOverlay) -> Result<HWND> {
    let class_name = wide_null(CLASS_NAME);
    let title = wide_null("Shuohua");
    let alpha = (255.0_f64 * 0.88).round() as u8;
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            L::constants::WIDTH as i32,
            L::constants::BASE_HEIGHT as i32,
            null_mut(),
            null_mut(),
            GetModuleHandleW(std::ptr::null()),
            overlay.cast(),
        )
    };
    if hwnd.is_null() {
        return Err(last_error("CreateWindowExW"));
    }
    unsafe {
        SetLayeredWindowAttributes(hwnd, 0 as COLORREF, alpha, LWA_ALPHA);
        ShowWindow(hwnd, SW_HIDE);
    }
    Ok(hwnd)
}

fn screen_panel_frame(cfg: &crate::config::theme::EffectiveOverlayCfg) -> L::LayoutFrame {
    let screen = L::LayoutFrame::new(
        0.0,
        0.0,
        unsafe { GetSystemMetrics(SM_CXSCREEN) } as f64,
        unsafe { GetSystemMetrics(SM_CYSCREEN) } as f64,
    );
    let height = L::constants::BASE_HEIGHT;
    let anchor = screen;
    let mut frame = L::panel_frame(
        anchor,
        cfg.core.position,
        L::constants::WIDTH,
        height,
        screen,
    );
    frame.y = screen.h - frame.y - frame.h;
    frame
}

fn overlay_height(model: &OverlayModel, cfg: &crate::config::theme::EffectiveOverlayCfg) -> f64 {
    let (_, lines) = L::display_text_plan(
        &model.display_text(),
        cfg.core.max_text_lines,
        L::constants::CHARS_PER_LINE,
    );
    L::constants::BASE_HEIGHT + (lines.saturating_sub(1) as f64 * L::constants::BODY_LINE_H)
}

unsafe fn draw_text(hdc: HDC, rect: &mut RECT, text: &str, rgb: u32, format: u32) {
    SetTextColor(hdc, to_colorref(rgb));
    let wide = wide_null(text);
    DrawTextW(hdc, wide.as_ptr(), -1, rect, format);
}

fn to_colorref(rgb: u32) -> COLORREF {
    let r = (rgb >> 16) & 0xff;
    let g = rgb & 0x00ff00;
    let b = rgb & 0x0000ff;
    (b << 16) | g | r
}

fn wide_null(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain([0]).collect()
}

fn last_error(context: &'static str) -> anyhow::Error {
    let code = unsafe { GetLastError() };
    anyhow!(
        "{context}: {}",
        std::io::Error::from_raw_os_error(code as i32)
    )
}

static WINDOWS_RENDERER_CAPABILITIES: [CapabilityStatus; 5] = {
    use CapabilityId as Id;
    use CapabilityStatusKind as Kind;

    [
        CapabilityStatus {
            id: Id::OverlayRenderer,
            platform: PlatformKind::Windows,
            backend: "win32_overlay_minimal",
            status: Kind::Partial,
            summary: "Windows Win32 overlay window backend is implemented but visual/runtime parity needs validation",
            reason: "runtime_smoke_only",
            next_step: Some("Validate visible overlay behavior on Windows 11/10 foreground apps"),
        },
        CapabilityStatus {
            id: Id::OverlayMaterial,
            platform: PlatformKind::Windows,
            backend: "win32_overlay_minimal",
            status: Kind::Degraded,
            summary: "Windows overlay currently uses translucent layered-window fallback only",
            reason: "translucent_fallback_only",
            next_step: Some("Evaluate Acrylic/Mica/blur only after the solid/translucent baseline is stable"),
        },
        CapabilityStatus {
            id: Id::OverlayAlwaysOnTop,
            platform: PlatformKind::Windows,
            backend: "win32_overlay_minimal",
            status: Kind::Partial,
            summary: "Windows overlay uses WS_EX_TOPMOST but broader foreground-app validation is pending",
            reason: "runtime_smoke_only",
            next_step: Some("Validate topmost behavior across foreground apps, UAC prompts, and fullscreen modes"),
        },
        CapabilityStatus {
            id: Id::OverlayInputPassthrough,
            platform: PlatformKind::Windows,
            backend: "win32_overlay_minimal",
            status: Kind::Partial,
            summary: "Windows overlay returns HTTRANSPARENT for hit testing but click-through needs real app validation",
            reason: "runtime_smoke_only",
            next_step: Some("Validate mouse/touch/pen passthrough with real foreground apps"),
        },
        CapabilityStatus {
            id: Id::OverlayWindowAnchor,
            platform: PlatformKind::Windows,
            backend: "win32_overlay_minimal",
            status: Kind::Degraded,
            summary: "Windows overlay uses screen anchoring; focused-window anchoring is not implemented",
            reason: "screen_anchor_only",
            next_step: Some("Add focused-window anchoring after foreground-window geometry is designed"),
        },
    ]
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::{OverlayHandle, OverlayState, TextKind};

    #[test]
    fn colorref_converts_rgb_to_windows_bgr_order() {
        assert_eq!(to_colorref(0x112233), 0x332211);
    }

    #[test]
    fn overlay_height_grows_with_wrapped_text() {
        let cfg = crate::config::theme::EffectiveOverlayCfg::default();
        let mut model = OverlayModel::new(&cfg.core.state);
        model.apply(
            OverlayCmd::SetText {
                text: "字".repeat(120),
                kind: TextKind::Partial,
            },
            &cfg.core.state,
        );
        assert!(overlay_height(&model, &cfg) > L::constants::BASE_HEIGHT);
    }

    #[test]
    fn capabilities_report_minimal_win32_backend() {
        let capabilities = renderer_capabilities();
        assert!(capabilities
            .iter()
            .any(|status| status.backend == "win32_overlay_minimal"
                && status.status == CapabilityStatusKind::Partial));
    }

    #[test]
    #[ignore = "creates a visible Win32 overlay window for a short runtime smoke"]
    fn runtime_smoke_creates_shows_hides_and_quits_window() {
        crate::i18n::init("en-US");
        let (handle, rx) = OverlayHandle::channel();
        let join = std::thread::spawn(move || {
            run(rx, crate::config::theme::EffectiveOverlayCfg::default()).expect("overlay run")
        });
        std::thread::sleep(Duration::from_millis(150));
        handle.send(OverlayCmd::SetState {
            state: OverlayState::Connecting,
        });
        handle.send(OverlayCmd::SetText {
            text: "Windows overlay smoke".to_string(),
            kind: TextKind::Partial,
        });
        std::thread::sleep(Duration::from_millis(350));
        handle.send(OverlayCmd::Hide);
        std::thread::sleep(Duration::from_millis(150));
        handle.send(OverlayCmd::Quit);
        join.join().expect("overlay thread");
    }
}
