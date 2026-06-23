use anyhow::Result;

use crate::platform::capability::{
    CapabilityId, CapabilityStatus, CapabilityStatusKind, PlatformKind,
};

use super::OverlayReceiver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaterialPreference {
    LiquidGlass,
    BlurredGlass,
    Translucent,
    Solid,
}

pub(crate) const MATERIAL_FALLBACK_ORDER: [MaterialPreference; 4] = [
    MaterialPreference::LiquidGlass,
    MaterialPreference::BlurredGlass,
    MaterialPreference::Translucent,
    MaterialPreference::Solid,
];

const OVERLAY_CAPABILITY_IDS: [CapabilityId; 5] = [
    CapabilityId::OverlayRenderer,
    CapabilityId::OverlayMaterial,
    CapabilityId::OverlayAlwaysOnTop,
    CapabilityId::OverlayInputPassthrough,
    CapabilityId::OverlayWindowAnchor,
];

pub(crate) fn renderer_capabilities() -> Vec<CapabilityStatus> {
    debug_assert_eq!(
        MATERIAL_FALLBACK_ORDER.last(),
        Some(&MaterialPreference::Solid)
    );

    #[cfg(target_os = "macos")]
    {
        macos_renderer_capabilities().to_vec()
    }
    #[cfg(target_os = "windows")]
    {
        super::windows::renderer_capabilities().to_vec()
    }
    #[cfg(target_os = "linux")]
    {
        super::linux::renderer_capabilities().to_vec()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let platform = PlatformKind::current();
        OVERLAY_CAPABILITY_IDS
            .into_iter()
            .map(|id| unsupported(id, platform))
            .collect()
    }
}

#[cfg(target_os = "macos")]
pub(super) fn run(
    rx: OverlayReceiver,
    cfg: crate::config::theme::EffectiveOverlayCfg,
) -> Result<()> {
    debug_assert_eq!(renderer_capabilities().len(), OVERLAY_CAPABILITY_IDS.len());
    super::macos::run(rx, cfg);
    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn run(
    rx: OverlayReceiver,
    cfg: crate::config::theme::EffectiveOverlayCfg,
) -> Result<()> {
    debug_assert_eq!(renderer_capabilities().len(), OVERLAY_CAPABILITY_IDS.len());
    super::windows::run(rx, cfg)
}

#[cfg(target_os = "linux")]
pub(super) fn run(
    rx: OverlayReceiver,
    cfg: crate::config::theme::EffectiveOverlayCfg,
) -> Result<()> {
    debug_assert_eq!(renderer_capabilities().len(), OVERLAY_CAPABILITY_IDS.len());
    super::linux::run(rx, cfg)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub(super) fn run(
    _rx: OverlayReceiver,
    _cfg: crate::config::theme::EffectiveOverlayCfg,
) -> Result<()> {
    debug_assert_eq!(renderer_capabilities().len(), OVERLAY_CAPABILITY_IDS.len());
    anyhow::bail!("overlay is not implemented on this platform")
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn unsupported(id: CapabilityId, platform: PlatformKind) -> CapabilityStatus {
    CapabilityStatus {
        id,
        platform,
        backend: "none",
        status: CapabilityStatusKind::Unsupported,
        summary: "Overlay renderer backend is not implemented for this platform",
        reason: "backend_not_implemented",
        next_step: None,
    }
}

#[cfg(target_os = "macos")]
fn macos_renderer_capabilities() -> &'static [CapabilityStatus] {
    &MACOS_RENDERER_CAPABILITIES
}

#[cfg(target_os = "macos")]
static MACOS_RENDERER_CAPABILITIES: [CapabilityStatus; 5] = {
    use CapabilityId as Id;
    use CapabilityStatusKind as Kind;

    [
        CapabilityStatus {
            id: Id::OverlayRenderer,
            platform: PlatformKind::Macos,
            backend: "appkit_panel",
            status: Kind::Available,
            summary: "AppKit overlay renderer is available",
            reason: "available",
            next_step: None,
        },
        CapabilityStatus {
            id: Id::OverlayMaterial,
            platform: PlatformKind::Macos,
            backend: "appkit_glass",
            status: Kind::Degraded,
            summary: "Overlay material may fall back from Liquid Glass",
            reason: "material_fallback_possible",
            next_step: None,
        },
        CapabilityStatus {
            id: Id::OverlayAlwaysOnTop,
            platform: PlatformKind::Macos,
            backend: "nsstatuswindowlevel",
            status: Kind::Available,
            summary: "Overlay top-level window placement is available",
            reason: "available",
            next_step: None,
        },
        CapabilityStatus {
            id: Id::OverlayInputPassthrough,
            platform: PlatformKind::Macos,
            backend: "nonactivating_panel",
            status: Kind::Partial,
            summary: "Overlay avoids activation; input passthrough remains renderer-specific",
            reason: "renderer_specific_input_policy",
            next_step: None,
        },
        CapabilityStatus {
            id: Id::OverlayWindowAnchor,
            platform: PlatformKind::Macos,
            backend: "accessibility_focused_window",
            status: Kind::Degraded,
            summary: "Focused-window anchoring falls back to screen anchoring",
            reason: "screen_anchor_fallback",
            next_step: None,
        },
    ]
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_fallback_order_keeps_readability_last() {
        assert_eq!(
            MATERIAL_FALLBACK_ORDER,
            [
                MaterialPreference::LiquidGlass,
                MaterialPreference::BlurredGlass,
                MaterialPreference::Translucent,
                MaterialPreference::Solid,
            ]
        );
    }

    #[test]
    fn renderer_capabilities_cover_overlay_renderer_surface() {
        let capabilities = renderer_capabilities();
        let ids: Vec<_> = capabilities.iter().map(|status| status.id).collect();
        assert_eq!(ids, OVERLAY_CAPABILITY_IDS);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_renderer_capabilities_describe_current_appkit_backend() {
        let capabilities = renderer_capabilities();
        assert_status(
            &capabilities,
            CapabilityId::OverlayRenderer,
            CapabilityStatusKind::Available,
            "appkit_panel",
        );
        assert_status(
            &capabilities,
            CapabilityId::OverlayMaterial,
            CapabilityStatusKind::Degraded,
            "appkit_glass",
        );
        assert_status(
            &capabilities,
            CapabilityId::OverlayAlwaysOnTop,
            CapabilityStatusKind::Available,
            "nsstatuswindowlevel",
        );
        assert_status(
            &capabilities,
            CapabilityId::OverlayInputPassthrough,
            CapabilityStatusKind::Partial,
            "nonactivating_panel",
        );
        assert_status(
            &capabilities,
            CapabilityId::OverlayWindowAnchor,
            CapabilityStatusKind::Degraded,
            "accessibility_focused_window",
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_renderer_capabilities_are_structured_unsupported() {
        for status in renderer_capabilities() {
            assert_eq!(status.platform, PlatformKind::current());
            assert_eq!(status.status, CapabilityStatusKind::Unsupported);
            assert_eq!(status.reason, "backend_not_implemented");
        }
    }

    #[cfg(target_os = "macos")]
    fn assert_status(
        statuses: &[CapabilityStatus],
        id: CapabilityId,
        expected: CapabilityStatusKind,
        backend: &'static str,
    ) {
        let status = statuses
            .iter()
            .find(|status| status.id == id)
            .unwrap_or_else(|| panic!("missing capability status for {}", id.as_str()));
        assert_eq!(status.status, expected);
        assert_eq!(status.backend, backend);
    }
}
