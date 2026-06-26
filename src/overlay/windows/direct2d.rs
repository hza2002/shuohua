use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, ID2D1RenderTarget,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

use super::{to_colorref, wide_null, WindowMetrics};
use crate::overlay::layout as L;
use crate::overlay::{OverlayModel, OverlayState};

const FONT_FAMILY: &str = "Segoe UI";
const LOCALE: &str = "en-us";

pub(super) struct Direct2dRenderer {
    hwnd: HWND,
    factory: ID2D1Factory,
    dwrite: IDWriteFactory,
    surface: Option<LayeredSurface>,
}

impl Direct2dRenderer {
    pub(super) fn new(hwnd: windows_sys::Win32::Foundation::HWND) -> Result<Self> {
        let hwnd = HWND(hwnd.cast());
        let factory = unsafe {
            D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
                .context("D2D1CreateFactory")?
        };
        let dwrite = unsafe {
            DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED)
                .context("DWriteCreateFactory")?
        };
        Ok(Self {
            hwnd,
            factory,
            dwrite,
            surface: None,
        })
    }

    pub(super) fn paint(
        &mut self,
        model: &OverlayModel,
        cfg: &crate::config::theme::EffectiveOverlayCfg,
        metrics: WindowMetrics,
    ) -> Result<()> {
        let width = metrics.px(cfg.core.width).max(1);
        let height = metrics.px(super::overlay_height(model, cfg)).max(1);
        let lines = super::overlay_line_count(model, cfg);
        let frames = L::overlay_frames(cfg.core.width, cfg.core.text_scale, lines);
        self.ensure_surface(width, height, metrics)?;
        let surface = self.surface.as_ref().context("Direct2D layered surface")?;
        surface.clear_pixels();

        unsafe {
            surface.target.BeginDraw();
            surface.target.Clear(Some(&transparent_color()));

            let background_alpha = cfg.core.background_alpha.clamp(0.0, 1.0) as f32;
            let background = surface
                .target
                .CreateSolidColorBrush(&color(cfg.core.background_rgb, background_alpha), None)
                .context("CreateSolidColorBrush background")?;
            surface.target.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: 0.0,
                        top: 0.0,
                        right: width as f32,
                        bottom: height as f32,
                    },
                    radiusX: metrics.px(cfg.core.corner_radius) as f32,
                    radiusY: metrics.px(cfg.core.corner_radius) as f32,
                },
                &background,
            );

            let meta_format = self.text_format(
                L::scaled_font_size(12.0, cfg.core.text_scale) as f32,
                false,
                false,
            )?;
            let state_format = self.text_format(
                L::scaled_font_size(13.0, cfg.core.text_scale) as f32,
                true,
                false,
            )?;
            let body_format = self.text_format(
                L::scaled_font_size(14.0, cfg.core.text_scale) as f32,
                false,
                true,
            )?;

            self.draw_state_icon(
                &surface.target,
                metrics.rect_f_from_frame(frames.height, frames.row.icon),
                model.state,
                model.state_color,
            )?;

            self.draw_text(
                &surface.target,
                &state_format,
                &model.state_label,
                metrics.rect_f_from_frame(frames.height, frames.row.status),
                model.state_color,
            )?;

            let app = model.app_name.as_deref().unwrap_or_default();
            let stats = L::stats_text(
                &L::format_duration(model.dur_ms),
                &crate::t!("overlay.word_count", n = model.words),
                app,
            );
            self.draw_text(
                &surface.target,
                &meta_format,
                &stats,
                metrics.rect_f_from_frame(frames.height, frames.row.stats),
                cfg.core.text.secondary,
            )?;

            let (meta, meta_color) = if let Some(notice) = &model.notice {
                (notice.text.as_str(), cfg.core.text.notice)
            } else {
                (model.chain_summary.as_str(), cfg.core.text.tertiary)
            };
            self.draw_text(
                &surface.target,
                &meta_format,
                meta,
                metrics.rect_f_from_frame(frames.height, frames.row.meta),
                meta_color,
            )?;

            let text_color = if model.error_text.is_empty() {
                cfg.core.text.primary
            } else {
                cfg.core.text.error
            };
            let (text, _) = L::display_text_plan(
                &model.display_text(),
                cfg.core.max_text_lines,
                L::chars_per_line(cfg.core.width, cfg.core.text_scale),
            );
            self.draw_text(
                &surface.target,
                &body_format,
                &text,
                metrics.rect_f_from_frame(frames.height, frames.body),
                text_color,
            )?;

            surface
                .target
                .EndDraw(None, None)
                .context("Direct2D EndDraw")?;
        }

        surface.update_window(self.hwnd)?;
        Ok(())
    }

    fn draw_state_icon(
        &self,
        target: &ID2D1DCRenderTarget,
        rect: D2D_RECT_F,
        state: OverlayState,
        rgb: u32,
    ) -> Result<()> {
        let brush = unsafe {
            target
                .CreateSolidColorBrush(&color(rgb, 1.0), None)
                .context("CreateSolidColorBrush state icon")?
        };
        let white = unsafe {
            target
                .CreateSolidColorBrush(&color(0xffffff, 1.0), None)
                .context("CreateSolidColorBrush state icon mark")?
        };
        let size = (rect.right - rect.left)
            .min(rect.bottom - rect.top)
            .max(1.0);
        let cx = (rect.left + rect.right) / 2.0;
        let cy = (rect.top + rect.bottom) / 2.0;
        let r = (size / 2.0 - 3.0).max(3.0);

        unsafe {
            match state {
                OverlayState::Idle | OverlayState::Connecting => {
                    fill_round_rect(target, square(cx, cy, r), r, &brush);
                }
                OverlayState::Recording => {
                    let bar_w = (size / 7.0).max(2.0);
                    let gap = (bar_w / 2.0).max(1.0);
                    for (idx, h) in [r, r * 2.0, r * 1.5].into_iter().enumerate() {
                        let x = cx - bar_w - gap + idx as f32 * (bar_w + gap);
                        fill_round_rect(
                            target,
                            D2D_RECT_F {
                                left: x,
                                top: cy - h / 2.0,
                                right: x + bar_w,
                                bottom: cy + h / 2.0,
                            },
                            bar_w / 2.0,
                            &brush,
                        );
                    }
                }
                OverlayState::Thinking => {
                    let dot_r = (r / 3.0).max(2.0);
                    for (x, y) in [(cx, cy - r), (cx + r, cy), (cx, cy + r), (cx - r, cy)] {
                        fill_round_rect(target, square(x, y, dot_r), dot_r, &brush);
                    }
                }
                OverlayState::Stopping => {
                    fill_round_rect(target, square(cx, cy, r), 2.0, &brush);
                }
                OverlayState::Error => {
                    fill_round_rect(target, square(cx, cy, r), 3.0, &brush);
                    target.FillRectangle(
                        &D2D_RECT_F {
                            left: cx - 1.0,
                            top: cy - r / 2.0,
                            right: cx + 1.0,
                            bottom: cy + r / 4.0,
                        },
                        &white,
                    );
                    target.FillRectangle(
                        &D2D_RECT_F {
                            left: cx - 1.0,
                            top: cy + r / 2.0,
                            right: cx + 1.0,
                            bottom: cy + r / 2.0 + 2.0,
                        },
                        &white,
                    );
                }
            }
        }
        Ok(())
    }

    fn ensure_surface(&mut self, width: i32, height: i32, metrics: WindowMetrics) -> Result<()> {
        let needs_new = self
            .surface
            .as_ref()
            .is_none_or(|surface| surface.width != width || surface.height != height);
        if !needs_new {
            return Ok(());
        }
        self.surface = Some(LayeredSurface::new(&self.factory, width, height, metrics)?);
        Ok(())
    }

    fn text_format(&self, point_size: f32, bold: bool, wrap: bool) -> Result<IDWriteTextFormat> {
        let family = wide_null(FONT_FAMILY);
        let locale = wide_null(LOCALE);
        let format = unsafe {
            self.dwrite
                .CreateTextFormat(
                    PCWSTR(family.as_ptr()),
                    None,
                    if bold {
                        DWRITE_FONT_WEIGHT_BOLD
                    } else {
                        DWRITE_FONT_WEIGHT_NORMAL
                    },
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    point_size,
                    PCWSTR(locale.as_ptr()),
                )
                .context("CreateTextFormat")?
        };
        unsafe {
            format
                .SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)
                .context("SetTextAlignment")?;
            format
                .SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)
                .context("SetParagraphAlignment")?;
            format
                .SetWordWrapping(if wrap {
                    DWRITE_WORD_WRAPPING_WRAP
                } else {
                    DWRITE_WORD_WRAPPING_NO_WRAP
                })
                .context("SetWordWrapping")?;
        }
        Ok(format)
    }

    fn draw_text(
        &self,
        target: &ID2D1DCRenderTarget,
        format: &IDWriteTextFormat,
        text: &str,
        rect: D2D_RECT_F,
        rgb: u32,
    ) -> Result<()> {
        let brush = unsafe {
            target
                .CreateSolidColorBrush(&color(rgb, 1.0), None)
                .context("CreateSolidColorBrush text")?
        };
        let text = utf16(text);
        unsafe {
            target.DrawText(
                &text,
                format,
                &rect,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        Ok(())
    }
}

fn square(cx: f32, cy: f32, radius: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: cx - radius,
        top: cy - radius,
        right: cx + radius,
        bottom: cy + radius,
    }
}

unsafe fn fill_round_rect(
    target: &ID2D1DCRenderTarget,
    rect: D2D_RECT_F,
    radius: f32,
    brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
) {
    target.FillRoundedRectangle(
        &D2D1_ROUNDED_RECT {
            rect,
            radiusX: radius,
            radiusY: radius,
        },
        brush,
    );
}

struct LayeredSurface {
    width: i32,
    height: i32,
    screen_dc: HDC,
    memory_dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    bits: *mut u8,
    target: ID2D1DCRenderTarget,
}

impl LayeredSurface {
    fn new(
        factory: &ID2D1Factory,
        width: i32,
        height: i32,
        metrics: WindowMetrics,
    ) -> Result<Self> {
        let screen_dc = unsafe { GetDC(None) };
        if screen_dc.is_invalid() {
            anyhow::bail!("GetDC returned null");
        }

        let memory_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
        if memory_dc.is_invalid() {
            unsafe {
                ReleaseDC(None, screen_dc);
            }
            anyhow::bail!("CreateCompatibleDC returned null");
        }

        let mut bits = std::ptr::null_mut();
        let bitmap_info = bitmap_info(width, height);
        let bitmap = unsafe {
            CreateDIBSection(
                Some(screen_dc),
                &bitmap_info,
                DIB_RGB_COLORS,
                &mut bits,
                None,
                0,
            )
            .context("CreateDIBSection")?
        };
        if bits.is_null() {
            unsafe {
                let _ = DeleteDC(memory_dc);
                ReleaseDC(None, screen_dc);
            }
            anyhow::bail!("CreateDIBSection returned null bits");
        }

        let old_bitmap = unsafe { SelectObject(memory_dc, HGDIOBJ(bitmap.0)) };
        if old_bitmap.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                let _ = DeleteDC(memory_dc);
                ReleaseDC(None, screen_dc);
            }
            anyhow::bail!("SelectObject returned null");
        }

        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: metrics.dpi as f32,
            dpiY: metrics.dpi as f32,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let target = unsafe {
            factory
                .CreateDCRenderTarget(&props)
                .context("CreateDCRenderTarget")?
        };
        let rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        unsafe {
            target.BindDC(memory_dc, &rect).context("BindDC")?;
        }

        Ok(Self {
            width,
            height,
            screen_dc,
            memory_dc,
            bitmap,
            old_bitmap,
            bits: bits.cast(),
            target,
        })
    }

    fn clear_pixels(&self) {
        let len = self.width.max(0) as usize * self.height.max(0) as usize * 4;
        unsafe {
            std::ptr::write_bytes(self.bits, 0, len);
        }
    }

    fn update_window(&self, hwnd: HWND) -> Result<()> {
        let size = SIZE {
            cx: self.width,
            cy: self.height,
        };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                hwnd,
                Some(self.screen_dc),
                None,
                Some(&size),
                Some(self.memory_dc),
                Some(&src),
                windows::Win32::Foundation::COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
            .context("UpdateLayeredWindow")?;
        }
        Ok(())
    }
}

impl Drop for LayeredSurface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.memory_dc, self.old_bitmap);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.memory_dc);
            ReleaseDC(None, self.screen_dc);
        }
    }
}

fn bitmap_info(width: i32, height: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: width.max(0) as u32 * height.max(0) as u32 * 4,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn transparent_color() -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    }
}

fn color(rgb: u32, alpha: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((rgb >> 16) & 0xff) as f32 / 255.0,
        g: ((rgb >> 8) & 0xff) as f32 / 255.0,
        b: (rgb & 0xff) as f32 / 255.0,
        a: alpha,
    }
}

fn utf16(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_normalizes_rgb_channels() {
        let c = color(0x336699, 0.5);
        assert!((c.r - 0.2).abs() < 0.01);
        assert!((c.g - 0.4).abs() < 0.01);
        assert!((c.b - 0.6).abs() < 0.01);
        assert_eq!(to_colorref(0x336699), 0x996633);
    }

    #[test]
    fn bitmap_info_uses_top_down_32bpp_surface() {
        let info = bitmap_info(640, 120);
        assert_eq!(info.bmiHeader.biWidth, 640);
        assert_eq!(info.bmiHeader.biHeight, -120);
        assert_eq!(info.bmiHeader.biBitCount, 32);
        assert_eq!(info.bmiHeader.biSizeImage, 640 * 120 * 4);
    }

    #[test]
    fn utf16_has_no_trailing_nul_for_directwrite_slice() {
        assert_eq!(utf16("ab"), vec![97, 98]);
    }
}
