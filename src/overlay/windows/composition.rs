use anyhow::{Context, Result};
use windows::Win32::Foundation::HWND as WindowsHwnd;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows_sys::Win32::Foundation::HWND;

use super::icons::{icon_font_fallback_order, state_icon_plan};
use super::scene::WindowsOverlayScene;
use super::WindowMetrics;
use crate::overlay::OverlayState;

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
            _target: target,
            tree,
        })
    }

    pub(super) fn update_reserved_scene(
        &self,
        scene: &WindowsOverlayScene,
        metrics: WindowMetrics,
    ) -> Result<()> {
        self.tree.apply_scene_contract(scene, metrics)?;
        unsafe {
            self.device
                .Commit()
                .context("IDCompositionDevice::Commit reserved scene")
        }
    }
}

pub(super) struct CompositionVisualTree {
    root: IDCompositionVisual,
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
        let shadow = create_visual(device, "shadow")?;
        let panel = create_visual(device, "panel")?;
        let content = create_visual(device, "content")?;
        let icon = create_visual(device, "icon")?;
        let status = create_visual(device, "status")?;
        let stats = create_visual(device, "stats")?;
        let meta = create_visual(device, "meta")?;
        let body = create_visual(device, "body")?;

        unsafe {
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
        &self,
        scene: &WindowsOverlayScene,
        metrics: WindowMetrics,
    ) -> Result<()> {
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

fn visual_offset(
    metrics: WindowMetrics,
    surface_height: f64,
    frame: crate::overlay::layout::LayoutFrame,
) -> (f32, f32) {
    let top = surface_height - frame.y - frame.h;
    (metrics.px(frame.x) as f32, metrics.px(top) as f32)
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
