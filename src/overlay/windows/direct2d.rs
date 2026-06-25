use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1RenderTarget,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_IMMEDIATELY,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;

use super::{to_colorref, wide_null, WindowMetrics};
use crate::overlay::layout as L;
use crate::overlay::OverlayModel;

const FONT_FAMILY: &str = "Segoe UI";
const LOCALE: &str = "en-us";

pub(super) struct Direct2dRenderer {
    hwnd: HWND,
    factory: ID2D1Factory,
    dwrite: IDWriteFactory,
    target: Option<ID2D1HwndRenderTarget>,
    size: D2D_SIZE_U,
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
            target: None,
            size: D2D_SIZE_U {
                width: 0,
                height: 0,
            },
        })
    }

    pub(super) fn paint(
        &mut self,
        model: &OverlayModel,
        cfg: &crate::config::theme::EffectiveOverlayCfg,
        metrics: WindowMetrics,
    ) -> Result<()> {
        let width = metrics.px(L::constants::WIDTH).max(1) as u32;
        let height = metrics.px(super::overlay_height(model, cfg)).max(1) as u32;
        self.ensure_target(width, height, metrics)?;
        let target = self.target.as_ref().context("Direct2D render target")?;

        unsafe {
            target.BeginDraw();
            target.Clear(Some(&transparent_color()));

            let background = target
                .CreateSolidColorBrush(&color(cfg.core.background_rgb, 1.0), None)
                .context("CreateSolidColorBrush background")?;
            target.FillRoundedRectangle(
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

            let state_format = self.text_format(13.0, true, false)?;
            let meta_format = self.text_format(12.0, false, false)?;
            let body_format = self.text_format(14.0, false, true)?;

            self.draw_text(
                target,
                &state_format,
                &model.state_label,
                metrics.rect_f(16.0, 11.0, 128.0, 34.0),
                model.state_color,
            )?;

            let app = model.app_name.as_deref().unwrap_or_default();
            let stats = L::stats_text(
                &L::format_duration(model.dur_ms),
                &crate::t!("overlay.word_count", n = model.words),
                app,
            );
            self.draw_text(
                target,
                &meta_format,
                &stats,
                metrics.rect_f(132.0, 11.0, 430.0, 34.0),
                cfg.core.text.secondary,
            )?;

            let (meta, meta_color) = if let Some(notice) = &model.notice {
                (notice.text.as_str(), cfg.core.text.notice)
            } else {
                (model.chain_summary.as_str(), cfg.core.text.tertiary)
            };
            self.draw_text(
                target,
                &meta_format,
                meta,
                metrics.rect_f(430.0, 11.0, L::constants::WIDTH - 16.0, 34.0),
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
                L::constants::CHARS_PER_LINE,
            );
            self.draw_text(
                target,
                &body_format,
                &text,
                metrics.rect_f(
                    16.0,
                    36.0,
                    L::constants::WIDTH - 16.0,
                    super::overlay_height(model, cfg) - 8.0,
                ),
                text_color,
            )?;

            target.EndDraw(None, None).context("Direct2D EndDraw")?;
        }

        Ok(())
    }

    fn ensure_target(&mut self, width: u32, height: u32, metrics: WindowMetrics) -> Result<()> {
        let size = D2D_SIZE_U { width, height };
        if let Some(target) = &self.target {
            if self.size != size {
                unsafe {
                    target.Resize(&size).context("Direct2D Resize")?;
                }
                self.size = size;
            }
            return Ok(());
        }

        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_UNKNOWN,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: metrics.dpi as f32,
            dpiY: metrics.dpi as f32,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd: self.hwnd,
            pixelSize: size,
            presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
        };
        let target = unsafe {
            self.factory
                .CreateHwndRenderTarget(&props, &hwnd_props)
                .context("CreateHwndRenderTarget")?
        };
        self.target = Some(target);
        self.size = size;
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
        target: &ID2D1HwndRenderTarget,
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
    fn utf16_has_no_trailing_nul_for_directwrite_slice() {
        assert_eq!(utf16("ab"), vec![97, 98]);
    }
}
