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
    #[cfg(target_os = "windows")]
    {
        windows_capabilities()
    }
    #[cfg(target_os = "linux")]
    {
        linux_capabilities()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        unsupported_capabilities(PlatformKind::current())
    }
}

#[cfg(not(target_os = "macos"))]
fn unsupported_capabilities(platform: PlatformKind) -> Vec<CapabilityStatus> {
    CapabilityId::ALL
        .into_iter()
        .map(|id| CapabilityStatus::unsupported(id, platform))
        .collect()
}

#[cfg(target_os = "linux")]
fn linux_capabilities() -> Vec<CapabilityStatus> {
    let mut capabilities = unsupported_capabilities(PlatformKind::Linux);
    for replacement in [
        CapabilityStatus {
            id: CapabilityId::IpcTransport,
            platform: PlatformKind::Linux,
            backend: "unix_domain_socket",
            status: CapabilityStatusKind::Available,
            summary: "Unix domain socket transport compiles for Linux",
            reason: "compile_checked",
            next_step: Some("Validate daemon/client IPC on Linux"),
        },
        CapabilityStatus {
            id: CapabilityId::DaemonSingleInstance,
            platform: PlatformKind::Linux,
            backend: "lock_file",
            status: CapabilityStatusKind::Available,
            summary: "Unix lock file single-instance guard compiles for Linux",
            reason: "compile_checked",
            next_step: Some("Validate daemon lock behavior on Linux"),
        },
        CapabilityStatus {
            id: CapabilityId::ProcessProbe,
            platform: PlatformKind::Linux,
            backend: "unix_process_probe",
            status: CapabilityStatusKind::Available,
            summary: "Unix process probing compiles for Linux",
            reason: "compile_checked",
            next_step: Some("Validate process probing on Linux"),
        },
        CapabilityStatus {
            id: CapabilityId::ServiceManager,
            platform: PlatformKind::Linux,
            backend: "systemd_user_dry_run",
            status: CapabilityStatusKind::Partial,
            summary: "systemd user service status can report dry-run unit information",
            reason: "dry_run_status_only",
            next_step: Some("Validate systemd user service install/start/stop on Linux"),
        },
        CapabilityStatus {
            id: CapabilityId::AudioCapture,
            platform: PlatformKind::Linux,
            backend: "cpal_alsa",
            status: CapabilityStatusKind::Partial,
            summary: "ALSA audio capture compiles but is not runtime-verified",
            reason: "compile_checked",
            next_step: Some("Validate audio device enumeration and recording on Linux"),
        },
        CapabilityStatus {
            id: CapabilityId::PathOpenReveal,
            platform: PlatformKind::Linux,
            backend: "xdg_open",
            status: CapabilityStatusKind::Partial,
            summary: "xdg-open can open paths; reveal falls back to opening the parent directory",
            reason: "reveal_opens_parent_dir",
            next_step: Some("Validate xdg-open behavior across Linux desktops"),
        },
    ]
    .into_iter()
    .chain(non_macos_desktop_capabilities(PlatformKind::Linux))
    {
        replace_capability(&mut capabilities, replacement);
    }
    capabilities
}

#[cfg(target_os = "windows")]
fn windows_capabilities() -> Vec<CapabilityStatus> {
    let mut capabilities = unsupported_capabilities(PlatformKind::Windows);
    for replacement in non_macos_desktop_capabilities(PlatformKind::Windows)
        .into_iter()
        .chain([
            CapabilityStatus {
                id: CapabilityId::DesktopHotkey,
                platform: PlatformKind::Windows,
                backend: "wh_keyboard_ll",
                status: CapabilityStatusKind::Partial,
                summary: "WH_KEYBOARD_LL hook backend is implemented but needs foreground-app validation",
                reason: "runtime_smoke_only",
                next_step: Some("Validate hotkey press/release tracking across real Windows foreground apps"),
            },
            CapabilityStatus {
                id: CapabilityId::DesktopHotkeySuppression,
                platform: PlatformKind::Windows,
                backend: "wh_keyboard_ll",
                status: CapabilityStatusKind::Partial,
                summary: "Low-level hook can suppress matched key events but target-app parity is not validated",
                reason: "runtime_smoke_only",
                next_step: Some("Validate suppressed down/up pairing, stuck modifier prevention, IME, and UAC boundaries"),
            },
            CapabilityStatus {
                id: CapabilityId::IpcTransport,
                platform: PlatformKind::Windows,
                backend: "named_pipe",
                status: CapabilityStatusKind::Partial,
                summary: "Named Pipe transport passed same-user smoke with explicit client access masks but still needs cross-user validation",
                reason: "same_user_elevation_smoke_only",
                next_step: Some("Validate cross-user isolation and longer Windows IPC soak"),
            },
            CapabilityStatus {
                id: CapabilityId::DaemonSingleInstance,
                platform: PlatformKind::Windows,
                backend: "named_mutex",
                status: CapabilityStatusKind::Partial,
                summary: "Named mutex daemon guard passed same-user and elevation smoke but still needs cross-user validation",
                reason: "same_user_elevation_smoke_only",
                next_step: Some("Validate cross-user daemon isolation on Windows"),
            },
            CapabilityStatus {
                id: CapabilityId::ProcessProbe,
                platform: PlatformKind::Windows,
                backend: "open_process_probe",
                status: CapabilityStatusKind::Partial,
                summary: "OpenProcess process probe is used by same-user service lifecycle smoke but still needs crash and PID-reuse validation",
                reason: "service_lifecycle_smoke_only",
                next_step: Some("Validate Windows process probing after daemon crash, abandoned mutex, and PID reuse"),
            },
            CapabilityStatus {
                id: CapabilityId::ServiceManager,
                platform: PlatformKind::Windows,
                backend: "windows_user_session",
                status: CapabilityStatusKind::Partial,
                summary: "Windows user service can start, stop, and restart the current user-session daemon without startup registration",
                reason: "user_session_start_stop_only",
                next_step: Some("Validate Windows user service install/startup registration strategy"),
            },
            CapabilityStatus {
                id: CapabilityId::AudioCapture,
                platform: PlatformKind::Windows,
                backend: "cpal_wasapi",
                status: CapabilityStatusKind::Partial,
                summary: "cpal/WASAPI input diagnostics can report the default device but recording is not runtime-verified",
                reason: "diagnostic_probe_only",
                next_step: Some(
                    "Validate microphone permission behavior and sustained recording on Windows",
                ),
            },
            CapabilityStatus {
                id: CapabilityId::PathOpenReveal,
                platform: PlatformKind::Windows,
                backend: "explorer",
                status: CapabilityStatusKind::Partial,
                summary: "explorer.exe path open/reveal passed basic manual smoke but still needs broader path/session validation",
                reason: "basic_manual_smoke_only",
                next_step: Some("Validate Explorer open/reveal with UNC, missing paths, and non-interactive sessions"),
            },
            CapabilityStatus {
                id: CapabilityId::DesktopClipboard,
                platform: PlatformKind::Windows,
                backend: "win32_clipboard_unicode",
                status: CapabilityStatusKind::Partial,
                summary: "Win32 CF_UNICODETEXT clipboard writes are implemented but need broader desktop-app runtime validation",
                reason: "write_only_runtime_smoke",
                next_step: Some("Validate Unicode clipboard writes across target Windows apps and elevation boundaries"),
            },
            CapabilityStatus {
                id: CapabilityId::DesktopTextInjection,
                platform: PlatformKind::Windows,
                backend: "sendinput_ctrl_v",
                status: CapabilityStatusKind::Partial,
                summary: "SendInput Ctrl+V paste injection is implemented but needs target-app and elevation validation",
                reason: "runtime_smoke_only",
                next_step: Some("Validate Ctrl+V injection across target Windows apps and UAC/elevation boundaries"),
            },
            CapabilityStatus {
                id: CapabilityId::DesktopActiveApp,
                platform: PlatformKind::Windows,
                backend: "foreground_window_process_exe",
                status: CapabilityStatusKind::Partial,
                summary: "Foreground window lookup can resolve the owning process executable name but not AppUserModelID",
                reason: "exe_name_only",
                next_step: Some("Validate foreground app route matching and add AppUserModelID lookup on Windows"),
            },
        ])
    {
        replace_capability(&mut capabilities, replacement);
    }
    capabilities
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn non_macos_desktop_capabilities(platform: PlatformKind) -> [CapabilityStatus; 6] {
    [
        CapabilityStatus {
            id: CapabilityId::DesktopHotkey,
            platform,
            backend: "none",
            status: CapabilityStatusKind::Unsupported,
            summary: "Desktop hotkey capture is not implemented for this platform",
            reason: "backend_not_implemented",
            next_step: Some("Choose and validate a desktop hotkey backend"),
        },
        CapabilityStatus {
            id: CapabilityId::DesktopHotkeySuppression,
            platform,
            backend: "none",
            status: CapabilityStatusKind::Unsupported,
            summary: "Desktop hotkey suppression is not implemented for this platform",
            reason: "backend_not_implemented",
            next_step: Some("Validate key suppression semantics with the chosen hotkey backend"),
        },
        CapabilityStatus {
            id: CapabilityId::DesktopClipboard,
            platform,
            backend: "none",
            status: CapabilityStatusKind::Unsupported,
            summary: "Clipboard writes are not implemented for this platform",
            reason: "backend_not_implemented",
            next_step: Some("Choose and validate a clipboard backend"),
        },
        CapabilityStatus {
            id: CapabilityId::DesktopTextInjection,
            platform,
            backend: "none",
            status: CapabilityStatusKind::Unsupported,
            summary: "Text injection is not implemented for this platform",
            reason: "backend_not_implemented",
            next_step: Some("Choose and validate a text injection backend"),
        },
        CapabilityStatus {
            id: CapabilityId::DesktopActiveApp,
            platform,
            backend: "default_context",
            status: CapabilityStatusKind::Degraded,
            summary: "Active app lookup falls back to an empty/default context",
            reason: "default_context_only",
            next_step: Some("Choose and validate active-window lookup for this platform"),
        },
        CapabilityStatus {
            id: CapabilityId::DesktopPermissions,
            platform,
            backend: "none",
            status: CapabilityStatusKind::Unavailable,
            summary: "Desktop permission probes are not available for this platform",
            reason: "permission_probe_missing",
            next_step: Some("Design platform-specific permission diagnostics"),
        },
    ]
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn replace_capability(capabilities: &mut [CapabilityStatus], replacement: CapabilityStatus) {
    let existing = capabilities
        .iter_mut()
        .find(|capability| capability.id == replacement.id)
        .expect("replacement capability id must exist in the base snapshot");
    *existing = replacement;
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
            "NSWorkspace active app lookup is available",
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

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_snapshot_marks_compile_checked_unix_primitives() {
        let statuses = current_platform_capabilities();
        assert_status(
            &statuses,
            CapabilityId::IpcTransport,
            CapabilityStatusKind::Available,
            "unix_domain_socket",
        );
        assert_status(
            &statuses,
            CapabilityId::DaemonSingleInstance,
            CapabilityStatusKind::Available,
            "lock_file",
        );
        assert_status(
            &statuses,
            CapabilityId::ProcessProbe,
            CapabilityStatusKind::Available,
            "unix_process_probe",
        );
        assert_status(
            &statuses,
            CapabilityId::ServiceManager,
            CapabilityStatusKind::Unsupported,
            "systemd_user_skeleton",
        );
        assert_status(
            &statuses,
            CapabilityId::AudioCapture,
            CapabilityStatusKind::Partial,
            "cpal_alsa",
        );
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    #[test]
    fn non_macos_snapshot_is_structured_unsupported() {
        for status in current_platform_capabilities() {
            assert_eq!(status.status, CapabilityStatusKind::Unsupported);
            assert_eq!(status.reason, "backend_not_implemented");
        }
    }

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
