use anyhow::Result;

use crate::platform::capability::{
    CapabilityId, CapabilityStatus, CapabilityStatusKind, PlatformKind,
};

use super::OverlayReceiver;

pub(super) fn renderer_capabilities() -> &'static [CapabilityStatus] {
    &LINUX_RENDERER_CAPABILITIES
}

pub(super) fn run(
    _rx: OverlayReceiver,
    _cfg: crate::config::theme::EffectiveOverlayCfg,
) -> Result<()> {
    anyhow::bail!("Linux overlay renderer skeleton is present but not implemented")
}

static LINUX_RENDERER_CAPABILITIES: [CapabilityStatus; 5] = {
    use CapabilityId as Id;
    use CapabilityStatusKind as Kind;

    [
        CapabilityStatus {
            id: Id::OverlayRenderer,
            platform: PlatformKind::Linux,
            backend: "wayland_overlay_skeleton",
            status: Kind::Unsupported,
            summary: "Linux overlay renderer skeleton is present but not implemented",
            reason: "backend_skeleton_only",
            next_step: Some("Validate Wayland layer-shell support on wlroots, KDE, and GNOME"),
        },
        CapabilityStatus {
            id: Id::OverlayMaterial,
            platform: PlatformKind::Linux,
            backend: "wayland_overlay_skeleton",
            status: Kind::Unsupported,
            summary: "Linux overlay material fallback is not implemented yet",
            reason: "backend_skeleton_only",
            next_step: Some("Start with readable solid or translucent fallback"),
        },
        CapabilityStatus {
            id: Id::OverlayAlwaysOnTop,
            platform: PlatformKind::Linux,
            backend: "wayland_overlay_skeleton",
            status: Kind::Unsupported,
            summary: "Wayland top-layer behavior depends on compositor support",
            reason: "backend_skeleton_only",
            next_step: Some("Validate wlr-layer-shell and record compositor support"),
        },
        CapabilityStatus {
            id: Id::OverlayInputPassthrough,
            platform: PlatformKind::Linux,
            backend: "wayland_overlay_skeleton",
            status: Kind::Unsupported,
            summary: "Linux click-through overlay behavior depends on compositor/toolkit support",
            reason: "backend_skeleton_only",
            next_step: Some("Validate pointer behavior separately from keyboard interactivity"),
        },
        CapabilityStatus {
            id: Id::OverlayWindowAnchor,
            platform: PlatformKind::Linux,
            backend: "wayland_overlay_skeleton",
            status: Kind::Degraded,
            summary:
                "Wayland focused-window anchoring is expected to fall back to screen anchoring",
            reason: "screen_anchor_expected",
            next_step: Some("Do not depend on private compositor protocols in shared code"),
        },
    ]
};
