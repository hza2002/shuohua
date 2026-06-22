use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilityStatusKind {
    Available,
    Unsupported,
    Unavailable,
    Partial,
    Degraded,
    Unknown,
}

impl CapabilityStatusKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unsupported => "unsupported",
            Self::Unavailable => "unavailable",
            Self::Partial => "partial",
            Self::Degraded => "degraded",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlatformKind {
    Macos,
    Linux,
    Windows,
    Unknown,
}

impl PlatformKind {
    pub(crate) const fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::Macos
        }
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Self::Unknown
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Windows => "windows",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilityId {
    IpcTransport,
    DaemonSingleInstance,
    ServiceManager,
    ProcessProbe,
    DesktopHotkey,
    DesktopHotkeySuppression,
    DesktopClipboard,
    DesktopTextInjection,
    DesktopActiveApp,
    DesktopPermissions,
    OverlayRenderer,
    OverlayMaterial,
    OverlayAlwaysOnTop,
    OverlayInputPassthrough,
    OverlayWindowAnchor,
    AudioCapture,
    AudioConvert,
    PathOpenReveal,
}

impl CapabilityId {
    pub(crate) const ALL: [Self; 18] = [
        Self::IpcTransport,
        Self::DaemonSingleInstance,
        Self::ServiceManager,
        Self::ProcessProbe,
        Self::DesktopHotkey,
        Self::DesktopHotkeySuppression,
        Self::DesktopClipboard,
        Self::DesktopTextInjection,
        Self::DesktopActiveApp,
        Self::DesktopPermissions,
        Self::OverlayRenderer,
        Self::OverlayMaterial,
        Self::OverlayAlwaysOnTop,
        Self::OverlayInputPassthrough,
        Self::OverlayWindowAnchor,
        Self::AudioCapture,
        Self::AudioConvert,
        Self::PathOpenReveal,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::IpcTransport => "ipc.transport",
            Self::DaemonSingleInstance => "daemon.single_instance",
            Self::ServiceManager => "service.manager",
            Self::ProcessProbe => "process.probe",
            Self::DesktopHotkey => "desktop.hotkey",
            Self::DesktopHotkeySuppression => "desktop.hotkey_suppression",
            Self::DesktopClipboard => "desktop.clipboard",
            Self::DesktopTextInjection => "desktop.text_injection",
            Self::DesktopActiveApp => "desktop.active_app",
            Self::DesktopPermissions => "desktop.permissions",
            Self::OverlayRenderer => "overlay.renderer",
            Self::OverlayMaterial => "overlay.material",
            Self::OverlayAlwaysOnTop => "overlay.always_on_top",
            Self::OverlayInputPassthrough => "overlay.input_passthrough",
            Self::OverlayWindowAnchor => "overlay.window_anchor",
            Self::AudioCapture => "audio.capture",
            Self::AudioConvert => "audio.convert",
            Self::PathOpenReveal => "path.open_reveal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CapabilityStatus {
    pub(crate) id: CapabilityId,
    pub(crate) platform: PlatformKind,
    pub(crate) backend: &'static str,
    pub(crate) status: CapabilityStatusKind,
    pub(crate) summary: &'static str,
    pub(crate) reason: &'static str,
    pub(crate) next_step: Option<&'static str>,
}

impl CapabilityStatus {
    const fn available(id: CapabilityId, backend: &'static str, summary: &'static str) -> Self {
        Self {
            id,
            platform: PlatformKind::Macos,
            backend,
            status: CapabilityStatusKind::Available,
            summary,
            reason: "available",
            next_step: None,
        }
    }

    const fn macos(
        id: CapabilityId,
        backend: &'static str,
        status: CapabilityStatusKind,
        summary: &'static str,
        reason: &'static str,
        next_step: Option<&'static str>,
    ) -> Self {
        Self {
            id,
            platform: PlatformKind::Macos,
            backend,
            status,
            summary,
            reason,
            next_step,
        }
    }

    #[cfg(not(target_os = "macos"))]
    const fn unsupported(id: CapabilityId, platform: PlatformKind) -> Self {
        Self {
            id,
            platform,
            backend: "none",
            status: CapabilityStatusKind::Unsupported,
            summary: "Backend is not implemented for this platform",
            reason: "backend_not_implemented",
            next_step: None,
        }
    }
}

pub(crate) fn current_platform_capabilities() -> Vec<CapabilityStatus> {
    #[cfg(target_os = "macos")]
    {
        let capabilities = macos_capabilities().to_vec();
        debug_assert_eq!(capabilities.len(), CapabilityId::ALL.len());
        capabilities
    }
    #[cfg(not(target_os = "macos"))]
    {
        let platform = PlatformKind::current();
        CapabilityId::ALL
            .into_iter()
            .map(|id| CapabilityStatus::unsupported(id, platform))
            .collect()
    }
}

#[cfg(target_os = "macos")]
fn macos_capabilities() -> &'static [CapabilityStatus] {
    &MACOS_CAPABILITIES
}

#[cfg(target_os = "macos")]
static MACOS_CAPABILITIES: [CapabilityStatus; 18] = {
    use CapabilityId as Id;
    use CapabilityStatusKind as Kind;

    [
        CapabilityStatus::available(
            Id::IpcTransport,
            "unix_domain_socket",
            "UDS transport is available",
        ),
        CapabilityStatus::available(
            Id::DaemonSingleInstance,
            "lock_file",
            "Lock file single-instance guard is available",
        ),
        CapabilityStatus::available(
            Id::ServiceManager,
            "launchd_user_agent",
            "launchd user agent service management is available",
        ),
        CapabilityStatus::available(
            Id::ProcessProbe,
            "unix_process_probe",
            "Unix process probing is available",
        ),
        CapabilityStatus::available(
            Id::DesktopHotkey,
            "cgeventtap",
            "CGEventTap hotkey capture is available",
        ),
        CapabilityStatus::available(
            Id::DesktopHotkeySuppression,
            "cgeventtap_drop",
            "CGEventTap event suppression is available",
        ),
        CapabilityStatus::available(
            Id::DesktopClipboard,
            "nspasteboard",
            "NSPasteboard clipboard writes are available",
        ),
        CapabilityStatus::available(
            Id::DesktopTextInjection,
            "cgevent_paste",
            "CGEvent paste injection is available",
        ),
        CapabilityStatus::available(
            Id::DesktopActiveApp,
            "nsworkspace",
            "NSWorkspace frontmost app lookup is available",
        ),
        CapabilityStatus::available(
            Id::DesktopPermissions,
            "accessibility_microphone",
            "Accessibility and microphone permission checks are available",
        ),
        CapabilityStatus::available(
            Id::OverlayRenderer,
            "appkit_panel",
            "AppKit overlay renderer is available",
        ),
        CapabilityStatus::macos(
            Id::OverlayMaterial,
            "appkit_glass",
            Kind::Degraded,
            "Overlay material may fall back from Liquid Glass",
            "material_fallback_possible",
            None,
        ),
        CapabilityStatus::available(
            Id::OverlayAlwaysOnTop,
            "nsstatuswindowlevel",
            "Overlay top-level window placement is available",
        ),
        CapabilityStatus::macos(
            Id::OverlayInputPassthrough,
            "nonactivating_panel",
            Kind::Partial,
            "Overlay avoids activation; input passthrough remains renderer-specific",
            "renderer_specific_input_policy",
            None,
        ),
        CapabilityStatus::macos(
            Id::OverlayWindowAnchor,
            "accessibility_focused_window",
            Kind::Degraded,
            "Focused-window anchoring falls back to screen anchoring",
            "screen_anchor_fallback",
            None,
        ),
        CapabilityStatus::available(Id::AudioCapture, "cpal", "Audio capture is available"),
        CapabilityStatus::available(
            Id::AudioConvert,
            "afconvert",
            "Retained audio conversion is available",
        ),
        CapabilityStatus::available(
            Id::PathOpenReveal,
            "open_command",
            "Desktop open/reveal commands are available",
        ),
    ]
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_kinds_have_stable_snake_case_names() {
        assert_eq!(CapabilityStatusKind::Available.as_str(), "available");
        assert_eq!(CapabilityStatusKind::Unsupported.as_str(), "unsupported");
        assert_eq!(CapabilityStatusKind::Unavailable.as_str(), "unavailable");
        assert_eq!(CapabilityStatusKind::Partial.as_str(), "partial");
        assert_eq!(CapabilityStatusKind::Degraded.as_str(), "degraded");
        assert_eq!(CapabilityStatusKind::Unknown.as_str(), "unknown");
    }

    #[test]
    fn capability_ids_match_design_document_names() {
        let names: Vec<_> = CapabilityId::ALL
            .into_iter()
            .map(CapabilityId::as_str)
            .collect();
        assert_eq!(
            names,
            [
                "ipc.transport",
                "daemon.single_instance",
                "service.manager",
                "process.probe",
                "desktop.hotkey",
                "desktop.hotkey_suppression",
                "desktop.clipboard",
                "desktop.text_injection",
                "desktop.active_app",
                "desktop.permissions",
                "overlay.renderer",
                "overlay.material",
                "overlay.always_on_top",
                "overlay.input_passthrough",
                "overlay.window_anchor",
                "audio.capture",
                "audio.convert",
                "path.open_reveal",
            ]
        );
    }

    #[test]
    fn current_platform_snapshot_has_one_status_per_capability() {
        let statuses = current_platform_capabilities();
        assert_eq!(statuses.len(), CapabilityId::ALL.len());
        for id in CapabilityId::ALL {
            assert!(
                statuses.iter().any(|status| status.id == id),
                "missing capability status for {}",
                id.as_str()
            );
        }
    }

    #[test]
    fn current_platform_snapshot_uses_current_platform() {
        let platform = PlatformKind::current();
        assert_eq!(platform.as_str(), current_platform_name());
        for status in current_platform_capabilities() {
            assert_eq!(status.platform, platform);
        }
    }

    #[test]
    fn status_model_can_express_all_phase_one_states() {
        let statuses = [
            CapabilityStatusKind::Available,
            CapabilityStatusKind::Unsupported,
            CapabilityStatusKind::Unavailable,
            CapabilityStatusKind::Partial,
            CapabilityStatusKind::Degraded,
            CapabilityStatusKind::Unknown,
        ];
        assert_eq!(statuses.len(), 6);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_snapshot_maps_existing_backend_capabilities() {
        let statuses = current_platform_capabilities();
        assert_status(
            &statuses,
            CapabilityId::DesktopHotkey,
            CapabilityStatusKind::Available,
            "cgeventtap",
        );
        assert_status(
            &statuses,
            CapabilityId::OverlayMaterial,
            CapabilityStatusKind::Degraded,
            "appkit_glass",
        );
        assert_status(
            &statuses,
            CapabilityId::OverlayInputPassthrough,
            CapabilityStatusKind::Partial,
            "nonactivating_panel",
        );
        assert_status(
            &statuses,
            CapabilityId::OverlayWindowAnchor,
            CapabilityStatusKind::Degraded,
            "accessibility_focused_window",
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_snapshot_is_structured_unsupported() {
        for status in current_platform_capabilities() {
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

    fn current_platform_name() -> &'static str {
        #[cfg(target_os = "macos")]
        {
            "macos"
        }
        #[cfg(target_os = "linux")]
        {
            "linux"
        }
        #[cfg(target_os = "windows")]
        {
            "windows"
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            "unknown"
        }
    }
}
