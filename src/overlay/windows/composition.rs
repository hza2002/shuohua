use anyhow::{Context, Result};
use windows::Win32::Foundation::HWND as WindowsHwnd;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows_sys::Win32::Foundation::HWND;

use super::icons::{icon_font_fallback_order, state_icon_plan};
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
    _root: IDCompositionVisual,
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
        unsafe {
            target
                .SetRoot(&root)
                .context("IDCompositionTarget::SetRoot")?;
            device.Commit().context("IDCompositionDevice::Commit")?;
        }
        Ok(Self {
            device,
            _target: target,
            _root: root,
        })
    }

    pub(super) fn keep_reserved_root_hidden(&self) -> Result<()> {
        unsafe {
            self.device
                .Commit()
                .context("IDCompositionDevice::Commit hidden root")
        }
    }
}

pub(super) fn readiness() -> CompositionReadiness {
    CompositionReadiness::Planned
}

pub(super) fn design_tokens() -> &'static [&'static str] {
    &[
        "Win32 HWND host",
        "DirectComposition or Windows Composition visuals",
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
        assert!(tokens.contains(&"DirectWrite text"));
        assert!(tokens.contains(&"Segoe Fluent Icons glyphs"));
        assert!(tokens.contains(&"fallback: Direct2D per-pixel layered surface"));
    }
}
