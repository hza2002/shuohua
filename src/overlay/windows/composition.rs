use anyhow::{bail, Context, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::HWND as WindowsHwnd;
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Factory, ID2D1RenderTarget, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_DEFAULT,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionAnimation, IDCompositionDevice,
    IDCompositionRectangleClip, IDCompositionSurface, IDCompositionTarget, IDCompositionVisual,
    IDCompositionVisual3,
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

use super::icons::{icon_font_fallback_order, state_icon_plan, IconAnimation};
use super::scene::WindowsOverlayScene;
use super::{wide_null, WindowMetrics};
use crate::overlay::OverlayState;

const UI_FONT_FAMILY: &str = "Segoe UI Variable";
const LOCALE: &str = "en-us";
const COMPOSITION_AMBIENT_SHADOW_LAYERS: usize = 18;
const COMPOSITION_AMBIENT_SHADOW_ALPHA: f32 = 0.010;
const COMPOSITION_AMBIENT_SHADOW_SPREAD: f64 = 0.9;
const COMPOSITION_KEY_SHADOW_LAYERS: usize = 10;
const COMPOSITION_KEY_SHADOW_ALPHA: f32 = 0.020;
const COMPOSITION_KEY_SHADOW_SPREAD: f64 = 0.72;
const COMPOSITION_KEY_SHADOW_Y: f64 = 6.0;

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
    clips: CompositionClips,
    surfaces: CompositionSurfaces,
    shadow: IDCompositionVisual,
    panel: IDCompositionVisual,
    panel3: Option<IDCompositionVisual3>,
    content: IDCompositionVisual,
    icon: IDCompositionVisual,
    icon3: Option<IDCompositionVisual3>,
    icon_animation: Option<IconAnimation>,
    status: IDCompositionVisual,
    stats: IDCompositionVisual,
    meta: IDCompositionVisual,
    body: IDCompositionVisual,
}

impl CompositionVisualTree {
    fn new(device: &IDCompositionDevice, root: IDCompositionVisual) -> Result<Self> {
        let animations = CompositionAnimations::new(device)?;
        let clips = CompositionClips::new(device)?;
        let surfaces = CompositionSurfaces::new();
        let shadow = create_visual(device, "shadow")?;
        let panel = create_visual(device, "panel")?;
        let panel3 = panel.cast::<IDCompositionVisual3>().ok();
        let content = create_visual(device, "content")?;
        let icon = create_visual(device, "icon")?;
        let icon3 = icon.cast::<IDCompositionVisual3>().ok();
        let status = create_visual(device, "status")?;
        let stats = create_visual(device, "stats")?;
        let meta = create_visual(device, "meta")?;
        let body = create_visual(device, "body")?;

        unsafe {
            root.SetOffsetX(&animations.root_static_offset)
                .context("IDCompositionVisual::SetOffsetX root static animation")?;
            panel
                .SetClip(&clips.panel)
                .context("IDCompositionVisual::SetClip panel rounded clip")?;
            content
                .SetClip(&clips.content)
                .context("IDCompositionVisual::SetClip content rounded clip")?;
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

        let tree = Self {
            root,
            animations,
            clips,
            surfaces,
            shadow,
            panel,
            panel3,
            content,
            icon,
            icon3,
            icon_animation: None,
            status,
            stats,
            meta,
            body,
        };
        tree.bind_animation_probes()?;
        Ok(tree)
    }

    fn bind_animation_probes(&self) -> Result<()> {
        unsafe {
            if let Some(icon3) = &self.icon3 {
                icon3
                    .SetOpacity(&self.animations.icon_static_opacity)
                    .context("IDCompositionVisual3::SetOpacity icon static animation")?;
            }
        }
        Ok(())
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
        let geometry = CompositionGeometry::from_scene(scene, metrics);
        self.surfaces.ensure_shadow_surface(
            device,
            &self.shadow,
            geometry.surface_width as u32,
            geometry.surface_height as u32,
        )?;
        self.surfaces.ensure_panel_surface(
            device,
            &self.panel,
            geometry.surface_width as u32,
            geometry.surface_height as u32,
        )?;
        let icon_width = metrics.px(scene.frames.row.icon.w).max(1) as u32;
        let icon_height = metrics.px(scene.frames.row.icon.h).max(1) as u32;
        self.surfaces
            .ensure_icon_surface(device, &self.icon, icon_width, icon_height)?;
        self.surfaces.draw_panel_probe(CompositionDrawContext {
            d2d,
            dwrite,
            scene,
            metrics,
            geometry,
        })?;
        self.surfaces.draw_icon_probe(CompositionDrawContext {
            d2d,
            dwrite,
            scene,
            metrics,
            geometry,
        })?;
        self.surfaces.draw_shadow_probe(CompositionDrawContext {
            d2d,
            dwrite,
            scene,
            metrics,
            geometry,
        })?;
        let icon = visual_offset(metrics, scene.frames.height, scene.frames.row.icon);
        let status = visual_offset(metrics, scene.frames.height, scene.frames.row.status);
        let stats = visual_offset(metrics, scene.frames.height, scene.frames.row.stats);
        let meta = visual_offset(metrics, scene.frames.height, scene.frames.row.meta);
        let body = visual_offset(metrics, scene.frames.height, scene.frames.body);
        self.clips.apply_panel_clip(geometry)?;
        self.bind_icon_animation_for_state(device, scene.state_icon.animation)?;
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
            if let Some(panel3) = &self.panel3 {
                panel3
                    .SetOpacity2(1.0)
                    .context("IDCompositionVisual3::SetOpacity2 panel")?;
            }
            self.content
                .SetOffsetX2(geometry.outset as f32)
                .context("IDCompositionVisual::SetOffsetX content")?;
            self.content
                .SetOffsetY2(geometry.outset as f32)
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

    fn bind_icon_animation_for_state(
        &mut self,
        device: &IDCompositionDevice,
        animation: IconAnimation,
    ) -> Result<()> {
        if self.icon_animation == Some(animation) {
            return Ok(());
        }
        self.icon_animation = Some(animation);
        let Some(icon3) = &self.icon3 else {
            return Ok(());
        };
        unsafe {
            match animation {
                IconAnimation::None => icon3
                    .SetOpacity2(1.0)
                    .context("IDCompositionVisual3::SetOpacity2 icon no animation")?,
                IconAnimation::Breathe | IconAnimation::Pulse | IconAnimation::Rotate => {
                    let opacity = repeating_opacity_keyframe_animation(
                        device,
                        "icon state opacity pulse",
                        &[(0.0, 1.0), (0.42, 0.62), (0.84, 1.0)],
                        0.84,
                    )?;
                    icon3
                        .SetOpacity(&opacity)
                        .context("IDCompositionVisual3::SetOpacity icon pulse animation")?;
                }
                IconAnimation::Dots => {
                    let opacity = repeating_opacity_keyframe_animation(
                        device,
                        "icon thinking opacity dots",
                        &[
                            (0.0, 0.55),
                            (0.22, 1.0),
                            (0.44, 0.55),
                            (0.66, 1.0),
                            (0.88, 0.8),
                        ],
                        0.88,
                    )?;
                    icon3
                        .SetOpacity(&opacity)
                        .context("IDCompositionVisual3::SetOpacity icon dots animation")?;
                }
                IconAnimation::Shake => {
                    let opacity = repeating_opacity_keyframe_animation(
                        device,
                        "icon error opacity alert",
                        &[
                            (0.0, 1.0),
                            (0.12, 0.35),
                            (0.24, 1.0),
                            (0.36, 0.45),
                            (0.54, 1.0),
                        ],
                        0.54,
                    )?;
                    icon3
                        .SetOpacity(&opacity)
                        .context("IDCompositionVisual3::SetOpacity icon alert animation")?;
                }
            }
        }
        Ok(())
    }
}

pub(super) struct CompositionClips {
    panel: IDCompositionRectangleClip,
    content: IDCompositionRectangleClip,
}

impl CompositionClips {
    fn new(device: &IDCompositionDevice) -> Result<Self> {
        let panel = create_rectangle_clip(device, "panel")?;
        let content = create_rectangle_clip(device, "content")?;
        Ok(Self { panel, content })
    }

    fn apply_panel_clip(&self, geometry: CompositionGeometry) -> Result<()> {
        apply_rounded_clip(
            &self.panel,
            geometry.panel_left,
            geometry.panel_top,
            geometry.panel_right,
            geometry.panel_bottom,
            geometry.radius,
            "panel",
        )?;
        apply_rounded_clip(
            &self.content,
            0.0,
            0.0,
            geometry.panel_width as f32,
            geometry.panel_height as f32,
            geometry.radius,
            "content",
        )
    }
}

pub(super) struct CompositionSurfaces {
    shadow: Option<IDCompositionSurface>,
    shadow_size: (u32, u32),
    panel: Option<IDCompositionSurface>,
    panel_size: (u32, u32),
    icon: Option<IDCompositionSurface>,
    icon_size: (u32, u32),
}

impl CompositionSurfaces {
    fn new() -> Self {
        Self {
            shadow: None,
            shadow_size: (0, 0),
            panel: None,
            panel_size: (0, 0),
            icon: None,
            icon_size: (0, 0),
        }
    }

    fn ensure_shadow_surface(
        &mut self,
        device: &IDCompositionDevice,
        visual: &IDCompositionVisual,
        width: u32,
        height: u32,
    ) -> Result<()> {
        if self.shadow_size == (width, height) && self.shadow.is_some() {
            return Ok(());
        }
        let surface = create_surface(device, width, height, "shadow")?;
        self.shadow_size = (width, height);
        unsafe {
            visual
                .SetContent(&surface)
                .context("IDCompositionVisual::SetContent resized shadow surface")?;
        }
        self.shadow = Some(surface);
        Ok(())
    }

    fn ensure_panel_surface(
        &mut self,
        device: &IDCompositionDevice,
        visual: &IDCompositionVisual,
        width: u32,
        height: u32,
    ) -> Result<()> {
        if self.panel_size == (width, height) && self.panel.is_some() {
            return Ok(());
        }
        let surface = create_surface(device, width, height, "panel")?;
        self.panel_size = (width, height);
        unsafe {
            visual
                .SetContent(&surface)
                .context("IDCompositionVisual::SetContent resized panel surface")?;
        }
        self.panel = Some(surface);
        Ok(())
    }

    fn ensure_icon_surface(
        &mut self,
        device: &IDCompositionDevice,
        visual: &IDCompositionVisual,
        width: u32,
        height: u32,
    ) -> Result<()> {
        if self.icon_size == (width, height) && self.icon.is_some() {
            return Ok(());
        }
        let surface = create_surface(device, width, height, "icon")?;
        self.icon_size = (width, height);
        unsafe {
            visual
                .SetContent(&surface)
                .context("IDCompositionVisual::SetContent resized icon surface")?;
        }
        self.icon = Some(surface);
        Ok(())
    }

    fn draw_panel_probe(&self, ctx: CompositionDrawContext<'_>) -> Result<()> {
        let Some(panel) = &self.panel else {
            bail!("composition panel surface is not initialized");
        };
        let rect = RECT {
            left: 0,
            top: 0,
            right: ctx.geometry.surface_width,
            bottom: ctx.geometry.surface_height,
        };
        let mut offset = POINT::default();
        let surface = unsafe {
            panel
                .BeginDraw::<IDXGISurface>(Some(&rect), &mut offset)
                .context("IDCompositionSurface::BeginDraw panel")?
        };

        let result = draw_dxgi_scene(&surface, ctx);
        let end = unsafe {
            panel
                .EndDraw()
                .context("IDCompositionSurface::EndDraw panel")
        };
        result.and(end)
    }

    fn draw_icon_probe(&self, ctx: CompositionDrawContext<'_>) -> Result<()> {
        let Some(icon) = &self.icon else {
            bail!("composition icon surface is not initialized");
        };
        let rect = RECT {
            left: 0,
            top: 0,
            right: self.icon_size.0 as i32,
            bottom: self.icon_size.1 as i32,
        };
        let mut offset = POINT::default();
        let surface = unsafe {
            icon.BeginDraw::<IDXGISurface>(Some(&rect), &mut offset)
                .context("IDCompositionSurface::BeginDraw icon")?
        };

        let result = draw_dxgi_icon(&surface, ctx, self.icon_size);
        let end = unsafe { icon.EndDraw().context("IDCompositionSurface::EndDraw icon") };
        result.and(end)
    }

    fn draw_shadow_probe(&self, ctx: CompositionDrawContext<'_>) -> Result<()> {
        let Some(shadow) = &self.shadow else {
            bail!("composition shadow surface is not initialized");
        };
        let rect = RECT {
            left: 0,
            top: 0,
            right: ctx.geometry.surface_width,
            bottom: ctx.geometry.surface_height,
        };
        let mut offset = POINT::default();
        let surface = unsafe {
            shadow
                .BeginDraw::<IDXGISurface>(Some(&rect), &mut offset)
                .context("IDCompositionSurface::BeginDraw shadow")?
        };

        let result = draw_dxgi_shadow(&surface, ctx);
        let end = unsafe {
            shadow
                .EndDraw()
                .context("IDCompositionSurface::EndDraw shadow")
        };
        result.and(end)
    }
}

struct CompositionDrawContext<'a> {
    d2d: &'a ID2D1Factory,
    dwrite: &'a IDWriteFactory,
    scene: &'a WindowsOverlayScene,
    metrics: WindowMetrics,
    geometry: CompositionGeometry,
}

#[derive(Clone, Copy)]
struct CompositionGeometry {
    surface_width: i32,
    surface_height: i32,
    panel_width: i32,
    panel_height: i32,
    panel_left: f32,
    panel_top: f32,
    panel_right: f32,
    panel_bottom: f32,
    radius: f32,
    outset: i32,
}

impl CompositionGeometry {
    fn from_scene(scene: &WindowsOverlayScene, metrics: WindowMetrics) -> Self {
        let panel_width = metrics.px(scene.panel_width).max(1);
        let panel_height = metrics.px(scene.frames.height).max(1);
        let outset = metrics.px(super::DIRECT2D_SHADOW_OUTSET).max(0);
        let surface_width = panel_width + outset * 2;
        let surface_height = panel_height + outset * 2;
        let panel_left = outset as f32;
        let panel_top = outset as f32;
        Self {
            surface_width,
            surface_height,
            panel_width,
            panel_height,
            panel_left,
            panel_top,
            panel_right: (outset + panel_width) as f32,
            panel_bottom: (outset + panel_height) as f32,
            radius: metrics.px(scene.corner_radius).max(0) as f32,
            outset,
        }
    }
}

fn create_surface(
    device: &IDCompositionDevice,
    width: u32,
    height: u32,
    name: &'static str,
) -> Result<IDCompositionSurface> {
    unsafe {
        device
            .CreateSurface(
                width,
                height,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_ALPHA_MODE_PREMULTIPLIED,
            )
            .with_context(|| format!("IDCompositionDevice::CreateSurface {name}"))
    }
}

fn create_rectangle_clip(
    device: &IDCompositionDevice,
    name: &'static str,
) -> Result<IDCompositionRectangleClip> {
    unsafe {
        device
            .CreateRectangleClip()
            .with_context(|| format!("IDCompositionDevice::CreateRectangleClip {name}"))
    }
}

fn apply_rounded_clip(
    clip: &IDCompositionRectangleClip,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
    name: &'static str,
) -> Result<()> {
    unsafe {
        clip.SetLeft2(left)
            .with_context(|| format!("IDCompositionRectangleClip::SetLeft2 {name}"))?;
        clip.SetTop2(top)
            .with_context(|| format!("IDCompositionRectangleClip::SetTop2 {name}"))?;
        clip.SetRight2(right)
            .with_context(|| format!("IDCompositionRectangleClip::SetRight2 {name}"))?;
        clip.SetBottom2(bottom)
            .with_context(|| format!("IDCompositionRectangleClip::SetBottom2 {name}"))?;
        clip.SetTopLeftRadiusX2(radius)
            .with_context(|| format!("IDCompositionRectangleClip::SetTopLeftRadiusX2 {name}"))?;
        clip.SetTopLeftRadiusY2(radius)
            .with_context(|| format!("IDCompositionRectangleClip::SetTopLeftRadiusY2 {name}"))?;
        clip.SetTopRightRadiusX2(radius)
            .with_context(|| format!("IDCompositionRectangleClip::SetTopRightRadiusX2 {name}"))?;
        clip.SetTopRightRadiusY2(radius)
            .with_context(|| format!("IDCompositionRectangleClip::SetTopRightRadiusY2 {name}"))?;
        clip.SetBottomLeftRadiusX2(radius)
            .with_context(|| format!("IDCompositionRectangleClip::SetBottomLeftRadiusX2 {name}"))?;
        clip.SetBottomLeftRadiusY2(radius)
            .with_context(|| format!("IDCompositionRectangleClip::SetBottomLeftRadiusY2 {name}"))?;
        clip.SetBottomRightRadiusX2(radius).with_context(|| {
            format!("IDCompositionRectangleClip::SetBottomRightRadiusX2 {name}")
        })?;
        clip.SetBottomRightRadiusY2(radius).with_context(|| {
            format!("IDCompositionRectangleClip::SetBottomRightRadiusY2 {name}")
        })?;
    }
    Ok(())
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
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_DEFAULT);
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
                    left: ctx.geometry.panel_left,
                    top: ctx.geometry.panel_top,
                    right: ctx.geometry.panel_right,
                    bottom: ctx.geometry.panel_bottom,
                },
                radiusX: ctx.geometry.radius,
                radiusY: ctx.geometry.radius,
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

fn draw_dxgi_icon(
    surface: &IDXGISurface,
    ctx: CompositionDrawContext<'_>,
    icon_size: (u32, u32),
) -> Result<()> {
    let target = create_dxgi_render_target(ctx.d2d, surface, "icon")?;
    unsafe {
        target.BeginDraw();
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_DEFAULT);
        target.Clear(Some(&transparent_color()));
    }

    let icon_format = text_format(
        ctx.dwrite,
        icon_font_fallback_order()[0],
        physical_font_size(
            ctx.metrics,
            crate::overlay::layout::scaled_font_size(18.0, ctx.scene.text_scale),
        ),
        false,
        false,
        false,
        true,
    )?;
    draw_text(
        &target,
        &icon_format,
        &ctx.scene.state_icon.fluent_glyph.to_string(),
        D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: icon_size.0 as f32,
            bottom: icon_size.1 as f32,
        },
        ctx.scene.state_color,
    )?;

    unsafe {
        target
            .EndDraw(None, None)
            .context("ID2D1RenderTarget::EndDraw icon")?;
    }
    Ok(())
}

fn draw_dxgi_shadow(surface: &IDXGISurface, ctx: CompositionDrawContext<'_>) -> Result<()> {
    let target = create_dxgi_render_target(ctx.d2d, surface, "shadow")?;
    unsafe {
        target.BeginDraw();
        target.Clear(Some(&transparent_color()));
        draw_shadow_pass(
            &target,
            ctx.geometry,
            ShadowPass {
                layers: COMPOSITION_AMBIENT_SHADOW_LAYERS,
                alpha: COMPOSITION_AMBIENT_SHADOW_ALPHA,
                spread_step: COMPOSITION_AMBIENT_SHADOW_SPREAD,
                y_offset: 0.0,
                falloff: 2.2,
            },
            ctx.metrics,
        )?;
        draw_shadow_pass(
            &target,
            ctx.geometry,
            ShadowPass {
                layers: COMPOSITION_KEY_SHADOW_LAYERS,
                alpha: COMPOSITION_KEY_SHADOW_ALPHA,
                spread_step: COMPOSITION_KEY_SHADOW_SPREAD,
                y_offset: COMPOSITION_KEY_SHADOW_Y,
                falloff: 1.8,
            },
            ctx.metrics,
        )?;
        target
            .EndDraw(None, None)
            .context("ID2D1RenderTarget::EndDraw shadow")?;
    }
    Ok(())
}

fn create_dxgi_render_target(
    d2d: &ID2D1Factory,
    surface: &IDXGISurface,
    name: &'static str,
) -> Result<ID2D1RenderTarget> {
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
    unsafe {
        d2d.CreateDxgiSurfaceRenderTarget(surface, &props)
            .with_context(|| format!("ID2D1Factory::CreateDxgiSurfaceRenderTarget {name}"))
    }
}

#[derive(Clone, Copy)]
struct ShadowPass {
    layers: usize,
    alpha: f32,
    spread_step: f64,
    y_offset: f64,
    falloff: f32,
}

fn draw_shadow_pass(
    target: &ID2D1RenderTarget,
    geometry: CompositionGeometry,
    pass: ShadowPass,
    metrics: WindowMetrics,
) -> Result<()> {
    let y_offset = metrics.px(pass.y_offset) as f32;
    for layer in (1..=pass.layers).rev() {
        let spread = metrics.px(layer as f64 * pass.spread_step).max(1) as f32;
        let alpha = shadow_layer_alpha(pass.alpha, layer, pass.layers, pass.falloff);
        let brush = unsafe {
            target
                .CreateSolidColorBrush(&color(0x000000, alpha), None)
                .context("CreateSolidColorBrush composition shadow")?
        };
        unsafe {
            target.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: geometry.panel_left - spread,
                        top: geometry.panel_top - spread + y_offset,
                        right: geometry.panel_right + spread,
                        bottom: geometry.panel_bottom + spread + y_offset,
                    },
                    radiusX: geometry.radius + spread,
                    radiusY: geometry.radius + spread,
                },
                &brush,
            );
        }
    }
    Ok(())
}

fn shadow_layer_alpha(alpha: f32, layer: usize, layers: usize, falloff: f32) -> f32 {
    if layers == 0 {
        return 0.0;
    }
    alpha * (layer as f32 / layers as f32).powf(falloff)
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
    icon_static_opacity: IDCompositionAnimation,
}

impl CompositionAnimations {
    fn new(device: &IDCompositionDevice) -> Result<Self> {
        Ok(Self {
            root_static_offset: static_animation(device, "root static offset")?,
            icon_static_opacity: static_animation(device, "icon static opacity")?,
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

fn opacity_keyframe_animation(
    device: &IDCompositionDevice,
    name: &'static str,
    keyframes: &[(f64, f32)],
) -> Result<IDCompositionAnimation> {
    build_opacity_keyframe_animation(device, name, keyframes, None)
}

fn repeating_opacity_keyframe_animation(
    device: &IDCompositionDevice,
    name: &'static str,
    keyframes: &[(f64, f32)],
    duration: f64,
) -> Result<IDCompositionAnimation> {
    if duration <= 0.0 {
        bail!("repeating opacity animation `{name}` requires positive duration");
    }
    build_opacity_keyframe_animation(device, name, keyframes, Some(duration))
}

fn build_opacity_keyframe_animation(
    device: &IDCompositionDevice,
    name: &'static str,
    keyframes: &[(f64, f32)],
    repeat_duration: Option<f64>,
) -> Result<IDCompositionAnimation> {
    let animation = unsafe {
        device
            .CreateAnimation()
            .with_context(|| format!("IDCompositionDevice::CreateAnimation {name}"))?
    };
    let Some((first_time, first_value)) = keyframes.first().copied() else {
        bail!("opacity animation `{name}` requires at least one keyframe");
    };
    let mut previous_time = first_time;
    let mut previous_value = first_value;
    unsafe {
        animation
            .AddCubic(first_time, first_value, 0.0, 0.0, 0.0)
            .with_context(|| format!("IDCompositionAnimation::AddCubic {name} first"))?;
        for (time, value) in keyframes.iter().copied().skip(1) {
            let duration = (time - previous_time).max(0.0);
            let slope = if duration > 0.0 {
                ((value - previous_value) as f64 / duration) as f32
            } else {
                0.0
            };
            animation
                .AddCubic(time, value, slope, 0.0, 0.0)
                .with_context(|| format!("IDCompositionAnimation::AddCubic {name} keyframe"))?;
            previous_time = time;
            previous_value = value;
        }
        if let Some(duration) = repeat_duration {
            animation
                .AddRepeat(0.0, duration)
                .with_context(|| format!("IDCompositionAnimation::AddRepeat {name}"))?;
        }
        animation
            .End(
                repeat_duration.unwrap_or_else(|| previous_time.max(1.0)),
                previous_value,
            )
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
    use super::super::scene::TextPlan;
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
            "CompositionGeometry",
            "draw_shadow_probe",
            "shadow_layer_alpha",
            "bind_icon_animation_for_state",
            "opacity_keyframe_animation",
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

    #[test]
    fn composition_geometry_keeps_panel_inside_shadow_outset_surface() {
        let cfg = crate::config::theme::EffectiveOverlayCfg::default();
        let model = crate::overlay::OverlayModel::new(&cfg.core.state);
        let metrics = WindowMetrics {
            dpi: 144,
            scale: 1.5,
            work_area: windows_sys::Win32::Foundation::RECT::default(),
        };
        let text = TextPlan {
            text: String::new(),
            lines: 1,
        };
        let scene = WindowsOverlayScene::from_model(&model, &cfg, cfg.core.width, text);

        let geometry = CompositionGeometry::from_scene(&scene, metrics);
        let outset = metrics.px(super::super::DIRECT2D_SHADOW_OUTSET);

        assert_eq!(geometry.outset, outset);
        assert_eq!(geometry.surface_width, geometry.panel_width + outset * 2);
        assert_eq!(geometry.surface_height, geometry.panel_height + outset * 2);
        assert_eq!(geometry.panel_left, outset as f32);
        assert_eq!(geometry.panel_top, outset as f32);
    }

    #[test]
    fn composition_shadow_alpha_tapers_outer_layers() {
        let inner = shadow_layer_alpha(0.1, 5, 5, 2.0);
        let outer = shadow_layer_alpha(0.1, 1, 5, 2.0);

        assert!((inner - 0.1).abs() < 0.001);
        assert!(outer < 0.01);
    }
}
