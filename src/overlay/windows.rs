use anyhow::{anyhow, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{GetLastError, COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateRoundRectRgn, CreateSolidBrush, DeleteObject,
    DrawTextW, Ellipse, EndPaint, FillRect, GetStockObject, InvalidateRect, LineTo, MoveToEx,
    Polygon, Rectangle, SelectObject, SetBkMode, SetTextColor, SetWindowRgn, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_TOP, DT_WORDBREAK, FF_DONTCARE, FW_BOLD, FW_NORMAL, HBRUSH, HDC, HGDIOBJ, HRGN,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, TRANSPARENT, WHITE_BRUSH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, LoadCursorW, PeekMessageW, PostQuitMessage, RegisterClassW,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow, SystemParametersInfoW,
    TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA,
    HTTRANSPARENT, IDC_ARROW, LWA_ALPHA, MSG, PM_REMOVE, SM_CXSCREEN, SM_CYSCREEN, SPI_GETWORKAREA,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WM_CREATE, WM_DESTROY,
    WM_ERASEBKGND, WM_NCHITTEST, WM_PAINT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::overlay::layout as L;
use crate::overlay::{OverlayCmd, OverlayModel, OverlayReceiver, OverlayState};
use crate::platform::capability::{
    CapabilityId, CapabilityStatus, CapabilityStatusKind, PlatformKind,
};

mod direct2d;

const CLASS_NAME: &str = "ShuohuaOverlayWindow";
const POLL_INTERVAL: Duration = Duration::from_millis(16);
const DEFAULT_DPI: f64 = 96.0;
pub(super) const DIRECT2D_SHADOW_OUTSET: f64 = 10.0;

#[derive(Clone, Copy)]
pub(super) struct WindowMetrics {
    pub(super) dpi: u32,
    scale: f64,
    work_area: RECT,
}

impl WindowMetrics {
    fn for_window(hwnd: HWND) -> Self {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let dpi = if dpi == 0 { DEFAULT_DPI as u32 } else { dpi };
        Self {
            dpi,
            scale: dpi as f64 / DEFAULT_DPI,
            work_area: work_area_rect(),
        }
    }

    fn px(self, logical: f64) -> i32 {
        (logical * self.scale).round() as i32
    }

    fn rect(self, left: f64, top: f64, right: f64, bottom: f64) -> RECT {
        RECT {
            left: self.px(left),
            top: self.px(top),
            right: self.px(right),
            bottom: self.px(bottom),
        }
    }

    fn rect_f(
        self,
        left: f64,
        top: f64,
        right: f64,
        bottom: f64,
    ) -> windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
        windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
            left: self.px(left) as f32,
            top: self.px(top) as f32,
            right: self.px(right) as f32,
            bottom: self.px(bottom) as f32,
        }
    }

    fn rect_from_frame(self, surface_height: f64, frame: L::LayoutFrame) -> RECT {
        let top = surface_height - frame.y - frame.h;
        self.rect(frame.x, top, frame.x + frame.w, top + frame.h)
    }

    fn rect_f_from_frame(
        self,
        surface_height: f64,
        frame: L::LayoutFrame,
    ) -> windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
        let top = surface_height - frame.y - frame.h;
        self.rect_f(frame.x, top, frame.x + frame.w, top + frame.h)
    }
}

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
    direct2d: Option<direct2d::Direct2dRenderer>,
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
            direct2d: None,
        });
        let raw = overlay.as_mut() as *mut Self;
        let hwnd = create_overlay_window(&overlay.cfg, raw)?;
        overlay.hwnd = hwnd;
        overlay.direct2d = match direct2d::Direct2dRenderer::new(hwnd) {
            Ok(renderer) => Some(renderer),
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "Direct2D overlay renderer unavailable; falling back to GDI"
                );
                None
            }
        };
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
                    self.apply_surface_shape();
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
        let metrics = WindowMetrics::for_window(self.hwnd);
        let frame = screen_panel_frame(&self.cfg, metrics);
        let outset = metrics.px(self.surface_outset());
        let height = metrics.px(overlay_height(&self.model, &self.cfg)) + outset * 2;
        let width = frame.w as i32 + outset * 2;
        unsafe {
            SetWindowPos(
                self.hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::HWND_TOPMOST,
                frame.x as i32 - outset,
                frame.y as i32 - outset,
                width,
                height,
                SWP_NOACTIVATE,
            );
        }
        self.apply_surface_shape();
        unsafe {
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

    fn apply_surface_shape(&self) {
        let metrics = WindowMetrics::for_window(self.hwnd);
        let width = metrics.px(self.cfg.core.width);
        let height = metrics.px(overlay_height(&self.model, &self.cfg));
        let radius = metrics.px(self.cfg.core.corner_radius).max(0);
        unsafe {
            if self.direct2d.is_none() {
                apply_window_alpha(self.hwnd, self.cfg.core.background_alpha);
                apply_rounded_window_region(self.hwnd, width, height, radius);
            } else {
                clear_window_region(self.hwnd);
            }
        }
    }

    fn surface_outset(&self) -> f64 {
        if self.direct2d.is_some() {
            DIRECT2D_SHADOW_OUTSET
        } else {
            0.0
        }
    }

    unsafe fn paint(&mut self, hdc: HDC) {
        let mut disable_direct2d = false;
        if let Some(renderer) = &mut self.direct2d {
            let metrics = WindowMetrics::for_window(self.hwnd);
            match renderer.paint(&self.model, &self.cfg, metrics) {
                Ok(()) => return,
                Err(error) => {
                    tracing::warn!(?error, "Direct2D overlay paint failed; falling back to GDI");
                    disable_direct2d = true;
                }
            }
        }
        if disable_direct2d {
            self.direct2d = None;
            self.apply_surface_shape();
        }

        let metrics = WindowMetrics::for_window(self.hwnd);
        let lines = overlay_line_count(&self.model, &self.cfg);
        let frames = L::overlay_frames(self.cfg.core.width, self.cfg.core.text_scale, lines);
        apply_window_alpha(self.hwnd, self.cfg.core.background_alpha);
        let rect = RECT {
            left: 0,
            top: 0,
            right: metrics.px(self.cfg.core.width),
            bottom: metrics.px(frames.height),
        };
        let brush = CreateSolidBrush(to_colorref(self.cfg.core.background_rgb));
        FillRect(hdc, &rect, brush);
        DeleteObject(brush as HGDIOBJ);

        SetBkMode(hdc, TRANSPARENT as i32);
        let text_scale = self.cfg.core.text_scale;
        let state_font = create_ui_font(metrics, L::scaled_font_size(13.0, text_scale), true);
        let meta_font = create_ui_font(metrics, L::scaled_font_size(12.0, text_scale), false);
        let body_font = create_ui_font(metrics, L::scaled_font_size(14.0, text_scale), false);
        draw_state_icon_gdi(
            hdc,
            metrics.rect_from_frame(frames.height, frames.row.icon),
            self.model.state,
            self.model.state_color,
        );
        let old_font = SelectObject(hdc, state_font);
        draw_text(
            hdc,
            &mut metrics.rect_from_frame(frames.height, frames.row.status),
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
        SelectObject(hdc, meta_font);
        draw_text(
            hdc,
            &mut metrics.rect_from_frame(frames.height, frames.row.stats),
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
            &mut metrics.rect_from_frame(frames.height, frames.row.meta),
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
            L::chars_per_line(self.cfg.core.width, self.cfg.core.text_scale),
        );
        SelectObject(hdc, body_font);
        draw_text(
            hdc,
            &mut metrics.rect_from_frame(frames.height, frames.body),
            &text,
            text_color,
            DT_LEFT | DT_TOP | DT_WORDBREAK | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        SelectObject(hdc, old_font);
        DeleteObject(state_font);
        DeleteObject(meta_font);
        DeleteObject(body_font);
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
                    as *mut WindowsOverlay;
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

fn create_overlay_window(
    cfg: &crate::config::theme::EffectiveOverlayCfg,
    overlay: *mut WindowsOverlay,
) -> Result<HWND> {
    let class_name = wide_null(CLASS_NAME);
    let title = wide_null("Shuohua");
    let frames = L::overlay_frames(cfg.core.width, cfg.core.text_scale, 1);
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            cfg.core.width.round() as i32,
            frames.height.round() as i32,
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
        ShowWindow(hwnd, SW_HIDE);
    }
    Ok(hwnd)
}

unsafe fn apply_window_alpha(hwnd: HWND, alpha: f64) {
    let alpha = (255.0 * alpha.clamp(0.0, 1.0)).round() as u8;
    SetLayeredWindowAttributes(hwnd, 0 as COLORREF, alpha, LWA_ALPHA);
}

unsafe fn apply_rounded_window_region(hwnd: HWND, width: i32, height: i32, radius: i32) {
    if width <= 0 || height <= 0 {
        return;
    }
    if radius <= 0 {
        SetWindowRgn(hwnd, null_mut(), 1);
        return;
    }
    let diameter = radius.saturating_mul(2);
    let region: HRGN = CreateRoundRectRgn(0, 0, width + 1, height + 1, diameter, diameter);
    if region.is_null() {
        return;
    }
    if SetWindowRgn(hwnd, region, 1) == 0 {
        DeleteObject(region as HGDIOBJ);
    }
}

unsafe fn clear_window_region(hwnd: HWND) {
    SetWindowRgn(hwnd, null_mut(), 1);
}

fn screen_panel_frame(
    cfg: &crate::config::theme::EffectiveOverlayCfg,
    metrics: WindowMetrics,
) -> L::LayoutFrame {
    let screen = work_area_layout(metrics);
    let height = L::overlay_frames(cfg.core.width, cfg.core.text_scale, 1).height;
    let anchor = screen;
    let mut frame = L::panel_frame(anchor, cfg.core.position, cfg.core.width, height, screen);
    frame.y = screen.h - frame.y - frame.h;
    frame.x = frame.x * metrics.scale + metrics.work_area.left as f64;
    frame.y = frame.y * metrics.scale + metrics.work_area.top as f64;
    frame.w *= metrics.scale;
    frame.h *= metrics.scale;
    frame
}

fn work_area_layout(metrics: WindowMetrics) -> L::LayoutFrame {
    let width = (metrics.work_area.right - metrics.work_area.left) as f64 / metrics.scale;
    let height = (metrics.work_area.bottom - metrics.work_area.top) as f64 / metrics.scale;
    L::LayoutFrame::new(0.0, 0.0, width, height)
}

fn work_area_rect() -> RECT {
    let mut rect = RECT::default();
    let ok =
        unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut rect as *mut RECT).cast(), 0) };
    if ok != 0 {
        return rect;
    }
    RECT {
        left: 0,
        top: 0,
        right: unsafe { GetSystemMetrics(SM_CXSCREEN) },
        bottom: unsafe { GetSystemMetrics(SM_CYSCREEN) },
    }
}

pub(super) fn overlay_height(
    model: &OverlayModel,
    cfg: &crate::config::theme::EffectiveOverlayCfg,
) -> f64 {
    let lines = overlay_line_count(model, cfg);
    L::overlay_frames(cfg.core.width, cfg.core.text_scale, lines).height
}

pub(super) fn overlay_line_count(
    model: &OverlayModel,
    cfg: &crate::config::theme::EffectiveOverlayCfg,
) -> usize {
    let (_, lines) = L::display_text_plan(
        &model.display_text(),
        cfg.core.max_text_lines,
        L::chars_per_line(cfg.core.width, cfg.core.text_scale),
    );
    lines
}

unsafe fn draw_text(hdc: HDC, rect: &mut RECT, text: &str, rgb: u32, format: u32) {
    SetTextColor(hdc, to_colorref(rgb));
    let wide = wide_null(text);
    DrawTextW(hdc, wide.as_ptr(), -1, rect, format);
}

unsafe fn draw_state_icon_gdi(hdc: HDC, rect: RECT, state: OverlayState, rgb: u32) {
    let color = to_colorref(rgb);
    let brush = CreateSolidBrush(color);
    let pen = CreatePen(PS_SOLID, 2, color);
    let old_brush = SelectObject(hdc, brush as HGDIOBJ);
    let old_pen = SelectObject(hdc, pen as HGDIOBJ);
    let cx = (rect.left + rect.right) / 2;
    let cy = (rect.top + rect.bottom) / 2;
    let size = (rect.right - rect.left).min(rect.bottom - rect.top).max(1);
    let r = (size / 2 - 3).max(3);

    match state {
        OverlayState::Idle | OverlayState::Connecting => {
            Ellipse(hdc, cx - r, cy - r, cx + r, cy + r);
        }
        OverlayState::Recording => {
            let bar_w = (size / 7).max(2);
            let gap = (bar_w / 2).max(1);
            for (idx, h) in [r, r * 2, r + r / 2].into_iter().enumerate() {
                let x = cx - bar_w - gap + idx as i32 * (bar_w + gap);
                Rectangle(hdc, x, cy - h / 2, x + bar_w, cy + h / 2);
            }
        }
        OverlayState::Thinking => {
            let points = [
                windows_sys::Win32::Foundation::POINT { x: cx, y: cy - r },
                windows_sys::Win32::Foundation::POINT {
                    x: cx + r / 3,
                    y: cy - r / 3,
                },
                windows_sys::Win32::Foundation::POINT { x: cx + r, y: cy },
                windows_sys::Win32::Foundation::POINT {
                    x: cx + r / 3,
                    y: cy + r / 3,
                },
                windows_sys::Win32::Foundation::POINT { x: cx, y: cy + r },
                windows_sys::Win32::Foundation::POINT {
                    x: cx - r / 3,
                    y: cy + r / 3,
                },
                windows_sys::Win32::Foundation::POINT { x: cx - r, y: cy },
                windows_sys::Win32::Foundation::POINT {
                    x: cx - r / 3,
                    y: cy - r / 3,
                },
            ];
            Polygon(hdc, points.as_ptr(), points.len() as i32);
        }
        OverlayState::Stopping => {
            Rectangle(hdc, cx - r, cy - r, cx + r, cy + r);
        }
        OverlayState::Error => {
            let points = [
                windows_sys::Win32::Foundation::POINT { x: cx, y: cy - r },
                windows_sys::Win32::Foundation::POINT {
                    x: cx + r,
                    y: cy + r,
                },
                windows_sys::Win32::Foundation::POINT {
                    x: cx - r,
                    y: cy + r,
                },
            ];
            Polygon(hdc, points.as_ptr(), points.len() as i32);
            let mark_pen = CreatePen(PS_SOLID, 2, to_colorref(0xffffff));
            let old_mark_pen = SelectObject(hdc, mark_pen as HGDIOBJ);
            MoveToEx(hdc, cx, cy - r / 2, std::ptr::null_mut());
            LineTo(hdc, cx, cy + r / 4);
            MoveToEx(hdc, cx, cy + r / 2, std::ptr::null_mut());
            LineTo(hdc, cx, cy + r / 2 + 1);
            SelectObject(hdc, old_mark_pen);
            DeleteObject(mark_pen as HGDIOBJ);
        }
    }

    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    DeleteObject(pen as HGDIOBJ);
    DeleteObject(brush as HGDIOBJ);
}

unsafe fn create_ui_font(metrics: WindowMetrics, point_size: f64, bold: bool) -> HGDIOBJ {
    let face = wide_null("Segoe UI");
    CreateFontW(
        -(point_size * metrics.dpi as f64 / 72.0).round() as i32,
        0,
        0,
        0,
        if bold {
            FW_BOLD as i32
        } else {
            FW_NORMAL as i32
        },
        0,
        0,
        0,
        DEFAULT_CHARSET.into(),
        OUT_DEFAULT_PRECIS.into(),
        CLIP_DEFAULT_PRECIS.into(),
        CLEARTYPE_QUALITY.into(),
        FF_DONTCARE.into(),
        face.as_ptr(),
    ) as HGDIOBJ
}

fn to_colorref(rgb: u32) -> COLORREF {
    let r = (rgb >> 16) & 0xff;
    let g = rgb & 0x00ff00;
    let b = rgb & 0x0000ff;
    (b << 16) | g | r
}

pub(super) fn wide_null(text: &str) -> Vec<u16> {
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
    fn metrics_scale_logical_units_to_physical_pixels() {
        let metrics = WindowMetrics {
            dpi: 144,
            scale: 1.5,
            work_area: RECT {
                left: 100,
                top: 50,
                right: 2500,
                bottom: 1850,
            },
        };

        assert_eq!(metrics.px(10.0), 15);
        let rect = metrics.rect(10.0, 20.0, 30.0, 40.0);
        assert_eq!(rect.left, 15);
        assert_eq!(rect.top, 30);
        assert_eq!(rect.right, 45);
        assert_eq!(rect.bottom, 60);
        assert_eq!(work_area_layout(metrics).w, 1600.0);
        assert_eq!(work_area_layout(metrics).h, 1200.0);
    }

    #[test]
    fn windows_rect_conversion_flips_shared_bottom_left_y_axis() {
        let metrics = WindowMetrics {
            dpi: 96,
            scale: 1.0,
            work_area: RECT::default(),
        };

        let rect = metrics.rect_from_frame(64.0, L::LayoutFrame::new(16.0, 8.0, 100.0, 20.0));

        assert_eq!(rect.left, 16);
        assert_eq!(rect.top, 36);
        assert_eq!(rect.right, 116);
        assert_eq!(rect.bottom, 56);
    }

    #[test]
    fn screen_panel_frame_uses_work_area_origin_and_scale() {
        let mut cfg = crate::config::theme::EffectiveOverlayCfg::default();
        cfg.core.width = 720.0;
        let metrics = WindowMetrics {
            dpi: 144,
            scale: 1.5,
            work_area: RECT {
                left: 100,
                top: 50,
                right: 2500,
                bottom: 1850,
            },
        };

        let frame = screen_panel_frame(&cfg, metrics);

        assert_eq!(frame.w, 720.0 * 1.5);
        assert!(frame.x >= 100.0);
        assert!(frame.y >= 50.0);
        assert!(frame.x + frame.w <= 2500.0);
    }

    #[test]
    fn screen_panel_frame_height_follows_text_scale() {
        let mut cfg = crate::config::theme::EffectiveOverlayCfg::default();
        let metrics = WindowMetrics {
            dpi: 144,
            scale: 1.5,
            work_area: RECT {
                left: 0,
                top: 0,
                right: 2400,
                bottom: 1800,
            },
        };

        let normal = screen_panel_frame(&cfg, metrics);
        cfg.core.text_scale = 1.5;
        let scaled = screen_panel_frame(&cfg, metrics);

        assert!(scaled.h > normal.h);
    }

    #[test]
    fn direct2d_shadow_outset_expands_window_without_changing_panel_size() {
        let cfg = crate::config::theme::EffectiveOverlayCfg::default();
        let metrics = WindowMetrics {
            dpi: 144,
            scale: 1.5,
            work_area: RECT {
                left: 0,
                top: 0,
                right: 2400,
                bottom: 1800,
            },
        };

        let panel = screen_panel_frame(&cfg, metrics);
        let outset = metrics.px(DIRECT2D_SHADOW_OUTSET);

        assert_eq!(
            panel.w as i32 + outset * 2,
            metrics.px(cfg.core.width) + outset * 2
        );
        assert_eq!(outset, 15);
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
