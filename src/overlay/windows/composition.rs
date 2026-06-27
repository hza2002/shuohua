use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND as WindowsHwnd;
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Factory, ID2D1RenderTarget, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionAnimation, IDCompositionDevice, IDCompositionSurface,
    IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM,
};
use windows::Win32::Graphics::Dxgi::IDXGISurface;
use windows_sys::Win32::Foundation::HWND;

use super::icons::{icon_font_fallback_order, state_icon_plan};
use super::scene::WindowsOverlayScene;
use super::{wide_null, WindowMetrics};
use crate::overlay::OverlayState;

const UI_FONT_FAMILY: &str = "Segoe UI Variable";
const LOCALE: &str = "en-us";

pub(super) const BACKEND_NAME: &str = "win32_composition_planned";
pub(super) const FALLBACK_BACKEND_NAME: &str = "win32_direct2d_per_pixel";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompositionReadiness {
    Planned,
    ProbeReady,
    Disabled,
}

pub(super) struct CompositionRenderer {
    device: IDCompositionDevice,
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    _target: IDCompositionTarget,
    tree: CompositionVisualTree,
}

impl CompositionRenderer {
    pub(super) fn new(hwnd: HWND) -> Result<Self> {
        let hwnd = WindowsHwnd(hwnd.cast());
        let device = unsafe {
            DCompositionCreateDevice::<_, IDCompositionDevice>(None)
                .context("DCompositionCreateDevice")?
        };
        let d2d = unsafe {
            D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
                .context("D2D1CreateFactory composition surface")?
        };
        let dwrite = unsafe {
            DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED)
                .context("DWriteCreateFactory composition surface")?
        };
        let target = unsafe {
            device
                .CreateTargetForHwnd(hwnd, true)
                .context("IDCompositionDevice::CreateTargetForHwnd")?
        };
        let root = unsafe {
            device
                .CreateVisual()
                .context("IDCompositionDevice::CreateVisual")?
        };
        let tree = CompositionVisualTree::new(&device, root)?;
        unsafe {
            target
                .SetRoot(&tree.root)
                .context("IDCompositionTarget::SetRoot")?;
            device.Commit().context("IDCompositionDevice::Commit")?;
        }
        Ok(Self {
            device,
            d2d,
            dwrite,
            _target: target,
            tree,
        })
    }

    pub(super) fn update_reserved_scene(
        &mut self,
        scene: &WindowsOverlayScene,
        metrics: WindowMetrics,
    ) -> Result<()> {
        self.tree
            .apply_scene_contract(&self.device, &self.d2d, &self.dwrite, scene, metrics)?;
        unsafe {
            self.device
                .Commit()
                .context("IDCompositionDevice::Commit reserved scene")
        }
    }
}

pub(super) struct CompositionVisualTree {
    root: IDCompositionVisual,
    animations: CompositionAnimations,
    surfaces: CompositionSurfaces,
    shadow: IDCompositionVisual,
    panel: IDCompositionVisual,
    content: IDCompositionVisual,
    icon: IDCompositionVisual,
    status: IDCompositionVisual,
    stats: IDCompositionVisual,
    meta: IDCompositionVisual,
    body: IDCompositionVisual,
}

impl CompositionVisualTree {
    fn new(device: &IDCompositionDevice, root: IDCompositionVisual) -> Result<Self> {
        let animations = CompositionAnimations::new(device)?;
        let surfaces = CompositionSurfaces::new(device)?;
        let shadow = create_visual(device, "shadow")?;
        let panel = create_visual(device, "panel")?;
        let content = create_visual(device, "content")?;
        let icon = create_visual(device, "icon")?;
        let status = create_visual(device, "status")?;
        let stats = create_visual(device, "stats")?;
        let meta = create_visual(device, "meta")?;
        let body = create_visual(device, "body")?;

        unsafe {
            root.SetOffsetX(&animations.root_static_offset)
                .context("IDCompositionVisual::SetOffsetX root static animation")?;
            panel
                .SetContent(&surfaces.panel)
                .context("IDCompositionVisual::SetContent panel surface")?;
            root.AddVisual(&shadow, false, None)
                .context("IDCompositionVisual::AddVisual shadow")?;
            root.AddVisual(&panel, false, None)
                .context("IDCompositionVisual::AddVisual panel")?;
            root.AddVisual(&content, false, None)
                .context("IDCompositionVisual::AddVisual content")?;
            content
                .AddVisual(&icon, false, None)
                .context("IDCompositionVisual::AddVisual icon")?;
            content
                .AddVisual(&status, false, None)
                .context("IDCompositionVisual::AddVisual status")?;
            content
                .AddVisual(&stats, false, None)
                .context("IDCompositionVisual::AddVisual stats")?;
            content
                .AddVisual(&meta, false, None)
                .context("IDCompositionVisual::AddVisual meta")?;
            content
                .AddVisual(&body, false, None)
                .context("IDCompositionVisual::AddVisual body")?;
        }

        Ok(Self {
            root,
            animations,
            surfaces,
            shadow,
            panel,
            content,
            icon,
            status,
            stats,
            meta,
            body,
        })
    }

    fn apply_scene_contract(
        &mut self,
        device: &IDCompositionDevice,
        d2d: &ID2D1Factory,
        dwrite: &IDWriteFactory,
        scene: &WindowsOverlayScene,
        metrics: WindowMetrics,
    ) -> Result<()> {
        self.animations.keep_alive();
        self.surfaces.ensure_panel_surface(
            device,
            &self.panel,
            metrics.px(scene.panel_width).max(1) as u32,
            metrics.px(scene.frames.height).max(1) as u32,
        )?;
        self.surfaces.draw_panel_probe(CompositionDrawContext {
            d2d,
            dwrite,
            scene,
            metrics,
            width: metrics.px(scene.panel_width).max(1) as u32,
            height: metrics.px(scene.frames.height).max(1) as u32,
            radius: metrics.px(scene.corner_radius).max(0) as f32,
        })?;
        let icon = visual_offset(metrics, scene.frames.height, scene.frames.row.icon);
        let status = visual_offset(metrics, scene.frames.height, scene.frames.row.status);
        let stats = visual_offset(metrics, scene.frames.height, scene.frames.row.stats);
        let meta = visual_offset(metrics, scene.frames.height, scene.frames.row.meta);
        let body = visual_offset(metrics, scene.frames.height, scene.frames.body);
        unsafe {
            self.shadow
                .SetOffsetX2(0.0)
                .context("IDCompositionVisual::SetOffsetX shadow")?;
            self.shadow
                .SetOffsetY2(0.0)
                .context("IDCompositionVisual::SetOffsetY shadow")?;
            self.panel
                .SetOffsetX2(0.0)
                .context("IDCompositionVisual::SetOffsetX panel")?;
            self.panel
                .SetOffsetY2(0.0)
                .context("IDCompositionVisual::SetOffsetY panel")?;
            self.content
                .SetOffsetX2(0.0)
                .context("IDCompositionVisual::SetOffsetX content")?;
            self.content
                .SetOffsetY2(0.0)
                .context("IDCompositionVisual::SetOffsetY content")?;
            self.icon
                .SetOffsetX2(icon.0)
                .context("IDCompositionVisual::SetOffsetX icon")?;
            self.icon
                .SetOffsetY2(icon.1)
                .context("IDCompositionVisual::SetOffsetY icon")?;
            self.status
                .SetOffsetX2(status.0)
                .context("IDCompositionVisual::SetOffsetX status")?;
            self.status
                .SetOffsetY2(status.1)
                .context("IDCompositionVisual::SetOffsetY status")?;
            self.stats
                .SetOffsetX2(stats.0)
                .context("IDCompositionVisual::SetOffsetX stats")?;
            self.stats
                .SetOffsetY2(stats.1)
                .context("IDCompositionVisual::SetOffsetY stats")?;
            self.meta
                .SetOffsetX2(meta.0)
                .context("IDCompositionVisual::SetOffsetX meta")?;
            self.meta
                .SetOffsetY2(meta.1)
                .context("IDCompositionVisual::SetOffsetY meta")?;
            self.body
                .SetOffsetX2(body.0)
                .context("IDCompositionVisual::SetOffsetX body")?;
            self.body
                .SetOffsetY2(body.1)
                .context("IDCompositionVisual::SetOffsetY body")?;
        }
        Ok(())
    }
}

pub(super) struct CompositionSurfaces {
    panel: IDCompositionSurface,
    panel_size: (u32, u32),
}

impl CompositionSurfaces {
    fn new(device: &IDCompositionDevice) -> Result<Self> {
        Ok(Self {
            panel: create_panel_surface(device, 1, 1)?,
            panel_size: (1, 1),
        })
    }

    fn ensure_panel_surface(
        &mut self,
        device: &IDCompositionDevice,
        visual: &IDCompositionVisual,
        width: u32,
        height: u32,
    ) -> Result<()> {
        if self.panel_size == (width, height) {
            return Ok(());
        }
        self.panel = create_panel_surface(device, width, height)?;
        self.panel_size = (width, height);
        unsafe {
            visual
                .SetContent(&self.panel)
                .context("IDCompositionVisual::SetContent resized panel surface")?;
        }
        Ok(())
    }

    fn draw_panel_probe(&self, ctx: CompositionDrawContext<'_>) -> Result<()> {
        let rect = RECT {
            left: 0,
            top: 0,
            right: ctx.width as i32,
            bottom: ctx.height as i32,
        };
        let mut offset = POINT::default();
        let surface = unsafe {
            self.panel
                .BeginDraw::<IDXGISurface>(Some(&rect), &mut offset)
                .context("IDCompositionSurface::BeginDraw panel")?
        };

        let result = draw_dxgi_scene(&surface, ctx);
        let end = unsafe {
            self.panel
                .EndDraw()
                .context("IDCompositionSurface::EndDraw panel")
        };
        result.and(end)
    }
}

struct CompositionDrawContext<'a> {
    d2d: &'a ID2D1Factory,
    dwrite: &'a IDWriteFactory,
    scene: &'a WindowsOverlayScene,
    metrics: WindowMetrics,
    width: u32,
    height: u32,
    radius: f32,
}

fn create_panel_surface(
    device: &IDCompositionDevice,
    width: u32,
    height: u32,
) -> Result<IDCompositionSurface> {
    unsafe {
        device
            .CreateSurface(
                width,
                height,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_ALPHA_MODE_PREMULTIPLIED,
            )
            .context("IDCompositionDevice::CreateSurface panel")
    }
}

fn draw_dxgi_scene(surface: &IDXGISurface, ctx: CompositionDrawContext<'_>) -> Result<()> {
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    let target = unsafe {
        ctx.d2d
            .CreateDxgiSurfaceRenderTarget(surface, &props)
            .context("ID2D1Factory::CreateDxgiSurfaceRenderTarget panel")?
    };
    unsafe {
        target.BeginDraw();
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        target.Clear(Some(&transparent_color()));
        let brush = target
            .CreateSolidColorBrush(
                &color(
                    ctx.scene.panel_color,
                    ctx.scene.panel_alpha.clamp(0.0, 1.0) as f32,
                ),
                None,
            )
            .context("CreateSolidColorBrush composition panel")?;
        target.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: ctx.width as f32,
                    bottom: ctx.height as f32,
                },
                radiusX: ctx.radius,
                radiusY: ctx.radius,
            },
            &brush,
        );
        draw_scene_text(&target, ctx.dwrite, ctx.scene, ctx.metrics)?;
        target
            .EndDraw(None, None)
            .context("ID2D1RenderTarget::EndDraw scene")?;
    }
    Ok(())
}

fn draw_scene_text(
    target: &ID2D1RenderTarget,
    dwrite: &IDWriteFactory,
    scene: &WindowsOverlayScene,
    metrics: WindowMetrics,
) -> Result<()> {
    let meta_format = text_format(
        dwrite,
        UI_FONT_FAMILY,
        physical_font_size(
            metrics,
            crate::overlay::layout::scaled_font_size(12.0, scene.text_scale),
        ),
        false,
        false,
        false,
        false,
    )?;
    let state_format = text_format(
        dwrite,
        UI_FONT_FAMILY,
        physical_font_size(
            metrics,
            crate::overlay::layout::scaled_font_size(13.0, scene.text_scale),
        ),
        true,
        false,
        false,
        false,
    )?;
    let body_format = text_format(
        dwrite,
        UI_FONT_FAMILY,
        physical_font_size(
            metrics,
            crate::overlay::layout::scaled_font_size(14.0, scene.text_scale),
        ),
        false,
        true,
        false,
        false,
    )?;
    let trailing_format = text_format(
        dwrite,
        UI_FONT_FAMILY,
        physical_font_size(
            metrics,
            crate::overlay::layout::scaled_font_size(12.0, scene.text_scale),
        ),
        false,
        false,
        true,
        false,
    )?;
    let icon_format = text_format(
        dwrite,
        icon_font_fallback_order()[0],
        physical_font_size(
            metrics,
            crate::overlay::layout::scaled_font_size(18.0, scene.text_scale),
        ),
        false,
        false,
        false,
        true,
    )?;

    draw_text(
        target,
        &icon_format,
        &scene.state_icon.fluent_glyph.to_string(),
        metrics.rect_f_from_frame(scene.frames.height, scene.frames.row.icon),
        scene.state_color,
    )?;
    draw_text(
        target,
        &state_format,
        &scene.state_label,
        text_rect(
            metrics.rect_f_from_frame(scene.frames.height, scene.frames.row.status),
            metrics,
            1.5,
        ),
        scene.state_color,
    )?;
    draw_text(
        target,
        &meta_format,
        &scene.stats,
        text_rect(
            metrics.rect_f_from_frame(scene.frames.height, scene.frames.row.stats),
            metrics,
            1.5,
        ),
        scene.stats_color,
    )?;
    draw_text(
        target,
        &trailing_format,
        &scene.meta,
        text_rect(
            metrics.rect_f_from_frame(scene.frames.height, scene.frames.row.meta),
            metrics,
            1.5,
        ),
        scene.meta_color,
    )?;
    draw_text(
        target,
        &body_format,
        &scene.text.text,
        text_rect(
            metrics.rect_f_from_frame(scene.frames.height, scene.frames.body),
            metrics,
            2.0,
        ),
        scene.body_color,
    )?;
    Ok(())
}

fn text_format(
    dwrite: &IDWriteFactory,
    family_name: &str,
    point_size: f32,
    bold: bool,
    wrap: bool,
    align_trailing: bool,
    center: bool,
) -> Result<IDWriteTextFormat> {
    let family = wide_null(family_name);
    let locale = wide_null(LOCALE);
    let format = unsafe {
        dwrite
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
            .context("CreateTextFormat composition")?
    };
    unsafe {
        format
            .SetTextAlignment(if center {
                DWRITE_TEXT_ALIGNMENT_CENTER
            } else if align_trailing {
                DWRITE_TEXT_ALIGNMENT_TRAILING
            } else {
                DWRITE_TEXT_ALIGNMENT_LEADING
            })
            .context("SetTextAlignment composition")?;
        format
            .SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)
            .context("SetParagraphAlignment composition")?;
        format
            .SetWordWrapping(if wrap {
                DWRITE_WORD_WRAPPING_WRAP
            } else {
                DWRITE_WORD_WRAPPING_NO_WRAP
            })
            .context("SetWordWrapping composition")?;
    }
    Ok(format)
}

fn draw_text(
    target: &ID2D1RenderTarget,
    format: &IDWriteTextFormat,
    text: &str,
    rect: D2D_RECT_F,
    rgb: u32,
) -> Result<()> {
    let brush = unsafe {
        target
            .CreateSolidColorBrush(&color(rgb, 1.0), None)
            .context("CreateSolidColorBrush composition text")?
    };
    let text = utf16(text);
    unsafe {
        target.DrawText(
            &text,
            format,
            &rect,
            &brush,
            if text.is_empty() {
                D2D1_DRAW_TEXT_OPTIONS_NONE
            } else {
                D2D1_DRAW_TEXT_OPTIONS_CLIP
            },
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
    Ok(())
}

pub(super) struct CompositionAnimations {
    root_static_offset: IDCompositionAnimation,
}

impl CompositionAnimations {
    fn new(device: &IDCompositionDevice) -> Result<Self> {
        Ok(Self {
            root_static_offset: static_animation(device, "root static offset")?,
        })
    }

    fn keep_alive(&self) {}
}

fn static_animation(
    device: &IDCompositionDevice,
    name: &'static str,
) -> Result<IDCompositionAnimation> {
    let animation = unsafe {
        device
            .CreateAnimation()
            .with_context(|| format!("IDCompositionDevice::CreateAnimation {name}"))?
    };
    unsafe {
        animation
            .AddCubic(0.0, 0.0, 0.0, 0.0, 0.0)
            .with_context(|| format!("IDCompositionAnimation::AddCubic {name}"))?;
        animation
            .End(1.0, 0.0)
            .with_context(|| format!("IDCompositionAnimation::End {name}"))?;
    }
    Ok(animation)
}

fn visual_offset(
    metrics: WindowMetrics,
    surface_height: f64,
    frame: crate::overlay::layout::LayoutFrame,
) -> (f32, f32) {
    let top = surface_height - frame.y - frame.h;
    (metrics.px(frame.x) as f32, metrics.px(top) as f32)
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

fn physical_font_size(metrics: WindowMetrics, logical_size: f64) -> f32 {
    metrics.px(logical_size).max(1) as f32
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

fn create_visual(device: &IDCompositionDevice, name: &'static str) -> Result<IDCompositionVisual> {
    unsafe {
        device
            .CreateVisual()
            .with_context(|| format!("IDCompositionDevice::CreateVisual {name}"))
    }
}

pub(super) fn readiness() -> CompositionReadiness {
    CompositionReadiness::Planned
}

pub(super) fn design_tokens() -> &'static [&'static str] {
    &[
        "Win32 HWND host",
        "DirectComposition or Windows Composition visuals",
        "composition visual tree: shadow panel content icon status stats meta body",
        "DirectWrite text",
        "Segoe Fluent Icons glyphs",
        "fallback: Direct2D per-pixel layered surface",
    ]
}

pub(super) fn icon_animation_contract(state: OverlayState) -> ([&'static str; 2], &'static str) {
    let _plan = state_icon_plan(state);
    (
        icon_font_fallback_order(),
        "composition transform/opacity animation",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_renderer_is_planned_but_not_default_enabled() {
        assert_eq!(BACKEND_NAME, "win32_composition_planned");
        assert_eq!(FALLBACK_BACKEND_NAME, "win32_direct2d_per_pixel");
        assert_eq!(readiness(), CompositionReadiness::Planned);
    }

    #[test]
    fn composition_direction_keeps_win32_shell_and_directwrite_text() {
        let tokens = design_tokens();
        assert!(tokens.contains(&"Win32 HWND host"));
        assert!(tokens.contains(&"DirectComposition or Windows Composition visuals"));
        assert!(tokens.contains(
            &"composition visual tree: shadow panel content icon status stats meta body"
        ));
        assert!(tokens.contains(&"DirectWrite text"));
        assert!(tokens.contains(&"Segoe Fluent Icons glyphs"));
        assert!(tokens.contains(&"fallback: Direct2D per-pixel layered surface"));
    }

    #[test]
    fn composition_source_names_reserved_visual_layers() {
        let source = include_str!("composition.rs");
        for token in [
            "CompositionVisualTree",
            "CompositionSurfaces",
            "shadow",
            "panel",
            "content",
            "icon",
            "status",
            "stats",
            "meta",
            "body",
            "update_reserved_scene",
        ] {
            assert!(
                source.contains(token),
                "composition source should reserve visual layer token `{token}`"
            );
        }
    }
}
