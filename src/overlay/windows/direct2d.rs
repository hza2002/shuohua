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
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
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

const FONT_FAMILY: &str = "Microsoft YaHei UI";
const LOCALE: &str = "en-us";
const LAYERED_RENDER_TARGET_DPI: f32 = 96.0;
const AMBIENT_SHADOW_LAYERS: usize = 22;
const AMBIENT_SHADOW_ALPHA: f32 = 0.012;
const AMBIENT_SHADOW_SPREAD: f64 = 0.86;
const KEY_SHADOW_LAYERS: usize = 12;
const KEY_SHADOW_ALPHA: f32 = 0.024;
const KEY_SHADOW_SPREAD: f64 = 0.7;
const KEY_SHADOW_Y: f64 = 6.0;
const MEASURE_HEIGHT: f32 = 16_384.0;

#[derive(Debug, Clone)]
pub(super) struct TextPlan {
    pub(super) text: String,
    pub(super) lines: usize,
}

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
        panel_width_logical: f64,
    ) -> Result<()> {
        let text_plan = self.text_plan(model, cfg, metrics, panel_width_logical)?;
        let panel_width = metrics.px(panel_width_logical).max(1);
        let panel_height = metrics
            .px(L::overlay_frames(panel_width_logical, cfg.core.text_scale, text_plan.lines).height)
            .max(1);
        let outset = metrics.px(super::DIRECT2D_SHADOW_OUTSET).max(0);
        let width = panel_width + outset * 2;
        let height = panel_height + outset * 2;
        let frames = L::overlay_frames(panel_width_logical, cfg.core.text_scale, text_plan.lines);
        self.ensure_surface(width, height, metrics)?;
        let surface = self.surface.as_ref().context("Direct2D layered surface")?;
        surface.clear_pixels();

        unsafe {
            surface.target.BeginDraw();
            surface.target.Clear(Some(&transparent_color()));

            let background_alpha = cfg.core.background_alpha.clamp(0.0, 1.0) as f32;
            let panel_rect = D2D_RECT_F {
                left: outset as f32,
                top: outset as f32,
                right: (outset + panel_width) as f32,
                bottom: (outset + panel_height) as f32,
            };
            let radius = metrics.px(cfg.core.corner_radius) as f32;
            self.draw_shadow(&surface.target, panel_rect, radius, metrics)?;

            let background = surface
                .target
                .CreateSolidColorBrush(&color(cfg.core.background_rgb, background_alpha), None)
                .context("CreateSolidColorBrush background")?;
            surface.target.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: panel_rect,
                    radiusX: radius,
                    radiusY: radius,
                },
                &background,
            );

            let meta_format = self.text_format(
                physical_font_size(metrics, L::scaled_font_size(12.0, cfg.core.text_scale)),
                false,
                false,
                false,
            )?;
            let state_format = self.text_format(
                physical_font_size(metrics, L::scaled_font_size(13.0, cfg.core.text_scale)),
                true,
                false,
                false,
            )?;
            let body_format = self.text_format(
                physical_font_size(metrics, L::scaled_font_size(14.0, cfg.core.text_scale)),
                false,
                true,
                false,
            )?;
            let trailing_meta_format = self.text_format(
                physical_font_size(metrics, L::scaled_font_size(12.0, cfg.core.text_scale)),
                false,
                false,
                true,
            )?;

            self.draw_state_icon(
                &surface.target,
                inset_rect(
                    metrics.rect_f_from_frame(frames.height, frames.row.icon),
                    outset,
                ),
                model.state,
                model.state_color,
            )?;

            self.draw_text(
                &surface.target,
                &state_format,
                &model.state_label,
                text_rect(
                    inset_rect(
                        metrics.rect_f_from_frame(frames.height, frames.row.status),
                        outset,
                    ),
                    metrics,
                    1.5,
                ),
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
                text_rect(
                    inset_rect(
                        metrics.rect_f_from_frame(frames.height, frames.row.stats),
                        outset,
                    ),
                    metrics,
                    1.5,
                ),
                cfg.core.text.secondary,
            )?;

            let (meta, meta_color) = if let Some(notice) = &model.notice {
                (notice.text.as_str(), cfg.core.text.notice)
            } else {
                (model.chain_summary.as_str(), cfg.core.text.tertiary)
            };
            self.draw_text(
                &surface.target,
                &trailing_meta_format,
                meta,
                text_rect(
                    inset_rect(
                        metrics.rect_f_from_frame(frames.height, frames.row.meta),
                        outset,
                    ),
                    metrics,
                    1.5,
                ),
                meta_color,
            )?;

            let text_color = if model.error_text.is_empty() {
                cfg.core.text.primary
            } else {
                cfg.core.text.error
            };
            self.draw_text(
                &surface.target,
                &body_format,
                &text_plan.text,
                text_rect(
                    inset_rect(
                        metrics.rect_f_from_frame(frames.height, frames.body),
                        outset,
                    ),
                    metrics,
                    2.0,
                ),
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

    pub(super) fn text_plan(
        &self,
        model: &OverlayModel,
        cfg: &crate::config::theme::EffectiveOverlayCfg,
        metrics: WindowMetrics,
        panel_width_logical: f64,
    ) -> Result<TextPlan> {
        let max_lines = cfg.core.max_text_lines.clamp(1, 8);
        let format = self.text_format(
            physical_font_size(metrics, L::scaled_font_size(14.0, cfg.core.text_scale)),
            false,
            true,
            false,
        )?;
        let width = metrics.px(L::body_width(panel_width_logical)).max(1) as f32;
        let full = model.display_text();
        let full_lines = self.measure_line_count(&format, &full, width)?;
        if full_lines <= max_lines {
            return Ok(TextPlan {
                text: full,
                lines: full_lines.max(1),
            });
        }

        let chars: Vec<char> = full.chars().collect();
        let mut lo = 0usize;
        let mut hi = chars.len();
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let candidate = tail_text(&chars, mid);
            if self.measure_line_count(&format, &candidate, width)? <= max_lines {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }

        Ok(TextPlan {
            text: tail_text(&chars, lo),
            lines: max_lines,
        })
    }

    fn draw_shadow(
        &self,
        target: &ID2D1DCRenderTarget,
        panel: D2D_RECT_F,
        radius: f32,
        metrics: WindowMetrics,
    ) -> Result<()> {
        self.draw_shadow_pass(
            target,
            ShadowPass {
                panel,
                radius,
                layers: AMBIENT_SHADOW_LAYERS,
                alpha: AMBIENT_SHADOW_ALPHA,
                spread_step: AMBIENT_SHADOW_SPREAD,
                y_offset: 0.0,
                falloff: 2.2,
            },
            metrics,
        )?;
        self.draw_shadow_pass(
            target,
            ShadowPass {
                panel,
                radius,
                layers: KEY_SHADOW_LAYERS,
                alpha: KEY_SHADOW_ALPHA,
                spread_step: KEY_SHADOW_SPREAD,
                y_offset: KEY_SHADOW_Y,
                falloff: 1.8,
            },
            metrics,
        )?;
        Ok(())
    }

    fn draw_shadow_pass(
        &self,
        target: &ID2D1DCRenderTarget,
        pass: ShadowPass,
        metrics: WindowMetrics,
    ) -> Result<()> {
        let y_offset = metrics.px(pass.y_offset) as f32;
        for layer in (1..=pass.layers).rev() {
            let spread = metrics.px(layer as f64 * pass.spread_step).max(1) as f32;
            let alpha = shadow_layer_alpha(pass.alpha, layer, pass.layers, pass.falloff);
            self.fill_shadow_layer(target, pass.panel, pass.radius, spread, y_offset, alpha)?;
        }
        Ok(())
    }

    fn fill_shadow_layer(
        &self,
        target: &ID2D1DCRenderTarget,
        panel: D2D_RECT_F,
        radius: f32,
        spread: f32,
        y_offset: f32,
        alpha: f32,
    ) -> Result<()> {
        let brush = unsafe {
            target
                .CreateSolidColorBrush(&color(0x000000, alpha), None)
                .context("CreateSolidColorBrush shadow")?
        };
        unsafe {
            target.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: panel.left - spread,
                        top: panel.top - spread + y_offset,
                        right: panel.right + spread,
                        bottom: panel.bottom + spread + y_offset,
                    },
                    radiusX: radius + spread,
                    radiusY: radius + spread,
                },
                &brush,
            );
        }
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

    fn text_format(
        &self,
        point_size: f32,
        bold: bool,
        wrap: bool,
        align_trailing: bool,
    ) -> Result<IDWriteTextFormat> {
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
                .SetTextAlignment(if align_trailing {
                    DWRITE_TEXT_ALIGNMENT_TRAILING
                } else {
                    DWRITE_TEXT_ALIGNMENT_LEADING
                })
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

    fn measure_line_count(
        &self,
        format: &IDWriteTextFormat,
        text: &str,
        width: f32,
    ) -> Result<usize> {
        let text = utf16(text);
        let layout = unsafe {
            self.dwrite
                .CreateTextLayout(&text, format, width, MEASURE_HEIGHT)
                .context("CreateTextLayout")?
        };
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe {
            layout
                .GetMetrics(&mut metrics)
                .context("IDWriteTextLayout::GetMetrics")?;
        }
        Ok(metrics.lineCount.max(1) as usize)
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

#[derive(Clone, Copy)]
struct ShadowPass {
    panel: D2D_RECT_F,
    radius: f32,
    layers: usize,
    alpha: f32,
    spread_step: f64,
    y_offset: f64,
    falloff: f32,
}

fn inset_rect(rect: D2D_RECT_F, inset: i32) -> D2D_RECT_F {
    let inset = inset as f32;
    D2D_RECT_F {
        left: rect.left + inset,
        top: rect.top + inset,
        right: rect.right + inset,
        bottom: rect.bottom + inset,
    }
}

fn text_rect(rect: D2D_RECT_F, metrics: WindowMetrics, vertical_pad: f64) -> D2D_RECT_F {
    let pad = metrics.px(vertical_pad) as f32;
    D2D_RECT_F {
        left: rect.left,
        top: rect.top - pad,
        right: rect.right,
        bottom: rect.bottom + pad,
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
        _metrics: WindowMetrics,
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
            dpiX: LAYERED_RENDER_TARGET_DPI,
            dpiY: LAYERED_RENDER_TARGET_DPI,
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

fn physical_font_size(metrics: WindowMetrics, logical_size: f64) -> f32 {
    metrics.px(logical_size).max(1) as f32
}

fn tail_text(chars: &[char], keep: usize) -> String {
    if keep >= chars.len() {
        return chars.iter().collect();
    }
    let keep = keep.min(chars.len());
    let tail: String = chars[chars.len() - keep..].iter().collect();
    format!("…{tail}")
}

fn shadow_layer_alpha(alpha: f32, layer: usize, layers: usize, falloff: f32) -> f32 {
    if layers == 0 {
        return 0.0;
    }
    alpha * (layer as f32 / layers as f32).powf(falloff)
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

    #[test]
    fn inset_rect_moves_text_into_panel_rect_without_resizing() {
        let rect = inset_rect(
            D2D_RECT_F {
                left: 1.0,
                top: 2.0,
                right: 11.0,
                bottom: 22.0,
            },
            10,
        );

        assert_eq!(rect.left, 11.0);
        assert_eq!(rect.top, 12.0);
        assert_eq!(rect.right, 21.0);
        assert_eq!(rect.bottom, 32.0);
    }

    #[test]
    fn shadow_layer_alpha_tapers_outer_layers() {
        let inner = shadow_layer_alpha(0.1, 5, 5, 2.0);
        let outer = shadow_layer_alpha(0.1, 1, 5, 2.0);

        assert!((inner - 0.1).abs() < 0.001);
        assert!(outer < 0.01);
    }

    #[test]
    fn layered_dib_render_target_uses_physical_pixel_coordinates() {
        assert_eq!(LAYERED_RENDER_TARGET_DPI, 96.0);
    }

    #[test]
    fn layered_dib_text_uses_physical_pixel_font_sizes() {
        let metrics = WindowMetrics {
            dpi: 144,
            scale: 1.5,
            work_area: windows_sys::Win32::Foundation::RECT::default(),
        };

        assert_eq!(physical_font_size(metrics, 14.0), 21.0);
    }
}
