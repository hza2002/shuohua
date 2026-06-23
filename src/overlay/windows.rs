use anyhow::Result;

use crate::platform::capability::{
    CapabilityId, CapabilityStatus, CapabilityStatusKind, PlatformKind,
};

use super::OverlayReceiver;

pub(super) fn renderer_capabilities() -> &'static [CapabilityStatus] {
    &WINDOWS_RENDERER_CAPABILITIES
}

pub(super) fn run(
    _rx: OverlayReceiver,
    _cfg: crate::config::theme::EffectiveOverlayCfg,
) -> Result<()> {
    anyhow::bail!("Windows overlay renderer skeleton is present but not implemented")
}

static WINDOWS_RENDERER_CAPABILITIES: [CapabilityStatus; 5] = {
    use CapabilityId as Id;
    use CapabilityStatusKind as Kind;

    [
        CapabilityStatus {
            id: Id::OverlayRenderer,
            platform: PlatformKind::Windows,
            backend: "win32_overlay_skeleton",
            status: Kind::Unsupported,
            summary: "Windows overlay renderer skeleton is present but not implemented",
            reason: "backend_skeleton_only",
            next_step: Some("Validate Win32 topmost layered overlay PoC on Windows 11/10"),
        },
        CapabilityStatus {
            id: Id::OverlayMaterial,
            platform: PlatformKind::Windows,
            backend: "win32_overlay_skeleton",
            status: Kind::Unsupported,
            summary: "Windows overlay material fallback is not implemented yet",
            reason: "backend_skeleton_only",
            next_step: Some("Verify translucent/solid fallback before Acrylic or Mica"),
        },
        CapabilityStatus {
            id: Id::OverlayAlwaysOnTop,
            platform: PlatformKind::Windows,
            backend: "win32_overlay_skeleton",
            status: Kind::Unsupported,
            summary: "Windows topmost overlay behavior still needs platform validation",
            reason: "backend_skeleton_only",
            next_step: Some("Validate WS_EX_TOPMOST and SetWindowPos behavior"),
        },
        CapabilityStatus {
            id: Id::OverlayInputPassthrough,
            platform: PlatformKind::Windows,
            backend: "win32_overlay_skeleton",
            status: Kind::Unsupported,
            summary: "Windows click-through overlay behavior still needs platform validation",
            reason: "backend_skeleton_only",
            next_step: Some("Validate WM_NCHITTEST or layered-window input behavior"),
        },
        CapabilityStatus {
            id: Id::OverlayWindowAnchor,
            platform: PlatformKind::Windows,
            backend: "win32_overlay_skeleton",
            status: Kind::Unsupported,
            summary: "Windows focused-window anchoring is not implemented yet",
            reason: "backend_skeleton_only",
            next_step: Some("Define active-window probing and screen-anchor fallback"),
        },
    ]
};
