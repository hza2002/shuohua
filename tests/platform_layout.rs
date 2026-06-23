use std::path::{Path, PathBuf};

fn assert_first_screen_summary_invoke_is_explicit(frontend: &str) {
    let Some(one_shot_invoke_pos) =
        frontend.find("invoke(\"gui_first_screen_summary_request_once\"")
    else {
        return;
    };
    let click_handler_pos = frontend
        .find("function handleExplicitRefresh")
        .expect("one-shot summary invoke should only exist with explicit refresh handler");
    assert!(
        one_shot_invoke_pos > click_handler_pos,
        "one-shot summary invoke must stay inside the explicit click handler"
    );
    assert!(
        !frontend.contains("invoke('gui_first_screen_summary_request_once'"),
        "placeholder should not use alternate one-shot summary invoke outside the audited explicit handler"
    );
}

fn tauri_application_command_permission(command_name: &str) -> String {
    format!("allow-{}", command_name.replace('_', "-"))
}

fn section_body<'a>(document: &'a str, header: &str) -> &'a str {
    let start = document
        .find(header)
        .unwrap_or_else(|| panic!("missing section {header}"));
    let after_header = &document[start + header.len()..];
    let end = after_header.find("\n[").unwrap_or(after_header.len());
    &after_header[..end]
}

#[test]
fn linux_cross_check_does_not_download_vad_runtime_at_build_time() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();

    let linux_section = section_body(
        &manifest,
        r#"[target.'cfg(target_os = "linux")'.dependencies]"#,
    );
    assert!(
        !linux_section.contains(r#"voice_activity_detector"#),
        "Linux cross checks must not compile voice_activity_detector because it enables ort-sys/download-binaries"
    );

    let non_linux_section = section_body(
        &manifest,
        r#"[target.'cfg(not(target_os = "linux"))'.dependencies]"#,
    );
    assert!(
        non_linux_section.contains(r#"voice_activity_detector = "0.2.1""#),
        "macOS/Windows VAD dependency should keep the current default runtime behavior"
    );

    let common_dependencies = section_body(&manifest, "[dependencies]");
    assert!(
        !common_dependencies.contains("voice_activity_detector"),
        "voice_activity_detector must stay target-specific so Linux cross checks can avoid build-time runtime downloads"
    );
}

#[test]
fn linux_capability_snapshot_marks_compile_checked_unix_primitives() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let capability = std::fs::read_to_string(root.join("src/platform/capability.rs")).unwrap();

    for token in [
        "fn linux_capabilities()",
        "PlatformKind::Linux",
        "CapabilityId::IpcTransport",
        "unix_domain_socket",
        "CapabilityId::DaemonSingleInstance",
        "lock_file",
        "CapabilityId::ProcessProbe",
        "unix_process_probe",
        "CapabilityId::ServiceManager",
        "systemd_user_skeleton",
        "CapabilityId::AudioCapture",
        "cpal_alsa",
        "compile_checked",
    ] {
        assert!(
            capability.contains(token),
            "Linux capability snapshot should document compile-checked primitive token `{token}`"
        );
    }
}

#[test]
fn linux_service_manager_capability_reports_dry_run_skeleton() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let capability = std::fs::read_to_string(root.join("src/platform/capability.rs")).unwrap();

    for token in [
        "CapabilityId::ServiceManager",
        "systemd_user_dry_run",
        "CapabilityStatusKind::Partial",
        "dry_run_status_only",
        "Validate systemd user service install/start/stop on Linux",
    ] {
        assert!(
            capability.contains(token),
            "Linux service.manager capability should report dry-run skeleton token `{token}`"
        );
    }
}

#[test]
fn path_open_reveal_lives_behind_platform_facade() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let platform_path = root.join("src/platform/path.rs");
    assert!(
        platform_path.exists(),
        "Phase 10g path open/reveal facade should live at src/platform/path.rs"
    );

    let platform_mod = std::fs::read_to_string(root.join("src/platform/mod.rs")).unwrap();
    assert!(
        platform_mod.contains("pub(crate) mod path;"),
        "src/platform/mod.rs must expose the path facade"
    );

    let facade = std::fs::read_to_string(platform_path).unwrap();
    for token in [
        "pub(crate) fn open_path(",
        "pub(crate) fn reveal_path(",
        "#[cfg(target_os = \"macos\")]",
        "#[cfg(target_os = \"linux\")]",
        "#[cfg(target_os = \"windows\")]",
        "xdg-open",
        "explorer.exe",
    ] {
        assert!(
            facade.contains(token),
            "platform path facade should contain token `{token}`"
        );
    }

    for file in ["src/tui/audio.rs", "src/tui/config_actions.rs"] {
        let body = std::fs::read_to_string(root.join(file)).unwrap();
        for forbidden in ["Command::new(\"open\")", "Command::new(\"/usr/bin/open\")"] {
            assert!(
                !body.contains(forbidden),
                "{file} should use platform::path instead of direct `{forbidden}`"
            );
        }
    }
}

#[test]
fn linux_path_open_reveal_capability_reports_xdg_open_partial() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let capability = std::fs::read_to_string(root.join("src/platform/capability.rs")).unwrap();

    for token in [
        "CapabilityId::PathOpenReveal",
        "xdg_open",
        "CapabilityStatusKind::Partial",
        "reveal_opens_parent_dir",
    ] {
        assert!(
            capability.contains(token),
            "Linux path.open_reveal capability should report xdg-open partial token `{token}`"
        );
    }
}

#[test]
fn windows_path_open_reveal_capability_reports_explorer_partial() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let capability = std::fs::read_to_string(root.join("src/platform/capability.rs")).unwrap();

    for token in [
        "CapabilityId::PathOpenReveal",
        "explorer",
        "CapabilityStatusKind::Partial",
        "Validate explorer.exe open/reveal behavior on Windows",
    ] {
        assert!(
            capability.contains(token),
            "Windows path.open_reveal capability should report explorer partial token `{token}`"
        );
    }
}

#[test]
fn audio_conversion_lives_behind_platform_facade() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let audio_convert_path = root.join("src/platform/audio_convert.rs");
    assert!(
        audio_convert_path.exists(),
        "Phase 10i retained audio conversion facade should live at src/platform/audio_convert.rs"
    );

    let platform_mod = std::fs::read_to_string(root.join("src/platform/mod.rs")).unwrap();
    assert!(
        platform_mod.contains("pub(crate) mod audio_convert;"),
        "src/platform/mod.rs must expose the audio conversion facade"
    );

    let facade = std::fs::read_to_string(audio_convert_path).unwrap();
    for token in [
        "pub(crate) fn convert_retained_audio(",
        "#[cfg(target_os = \"macos\")]",
        "/usr/bin/afconvert",
        "afconvert_args",
    ] {
        assert!(
            facade.contains(token),
            "platform audio conversion facade should contain token `{token}`"
        );
    }

    let voice_audio = std::fs::read_to_string(root.join("src/voice/audio.rs")).unwrap();
    for forbidden in [
        "/usr/bin/afconvert",
        "afconvert_args",
        "std::process::Command",
    ] {
        assert!(
            !voice_audio.contains(forbidden),
            "voice audio code should use platform::audio_convert instead of `{forbidden}`"
        );
    }
    assert!(
        voice_audio.contains("crate::platform::audio_convert::convert_retained_audio"),
        "voice audio finish path should call platform::audio_convert"
    );
}

#[test]
fn shared_macos_adapters_live_under_platform_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in [
        "src/platform/mod.rs",
        "src/platform/capability.rs",
        "src/platform/macos/mod.rs",
        "src/platform/macos/app_context.rs",
        "src/platform/macos/autotype.rs",
        "src/platform/macos/clipboard.rs",
        "src/platform/macos/window.rs",
    ] {
        assert!(root.join(file).exists(), "missing {file}");
    }

    for file in [
        "src/app_context_darwin.rs",
        "src/autotype_darwin.rs",
        "src/clipboard_darwin.rs",
        "src/focused_window_darwin.rs",
    ] {
        assert!(!root.join(file).exists(), "root adapter remains: {file}");
    }
}

#[test]
fn business_layers_use_platform_facades_not_macos_modules() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in [
        "src/voice/dispatch.rs",
        "src/tui/history/mod.rs",
        "src/cli/doctor.rs",
    ] {
        let body = std::fs::read_to_string(root.join(file)).unwrap();
        assert!(
            !body.contains("platform::macos"),
            "{file} should depend on platform facades, not macOS modules directly"
        );
    }
}

#[test]
fn macos_platform_module_is_cfg_gated() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let body = std::fs::read_to_string(root.join("src/platform/mod.rs")).unwrap();

    assert!(
        body.contains("pub(crate) mod capability;"),
        "src/platform/mod.rs must expose the shared capability model"
    );

    assert!(
        body.contains("#[cfg(target_os = \"macos\")]"),
        "src/platform/mod.rs must cfg-gate the macOS backend"
    );
    assert!(
        body.contains("pub mod macos;"),
        "src/platform/mod.rs must expose the macOS backend only through the cfg gate"
    );
}

#[test]
fn shared_platform_facades_do_not_import_apple_sdks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let forbidden = [
        "objc2",
        "objc2_",
        "core_graphics",
        "NSWorkspace",
        "NSPasteboard",
        "CGEvent",
    ];

    for file in [
        "src/platform/autotype.rs",
        "src/platform/clipboard.rs",
        "src/platform/desktop.rs",
        "src/platform/permissions.rs",
        "src/platform/daemon.rs",
    ] {
        let body = std::fs::read_to_string(root.join(file)).unwrap();
        for token in forbidden {
            assert!(
                !body.contains(token),
                "{file} should stay a facade; Apple SDK token `{token}` belongs in src/platform/macos/**"
            );
        }
    }
}

#[test]
fn business_layers_do_not_import_macos_backend_directly() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for file in rust_files_under(&root.join("src")) {
        let relative = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if allows_direct_macos_backend(&relative) {
            continue;
        }

        let body = std::fs::read_to_string(&file).unwrap();
        if body.contains("platform::macos") || body.contains("crate::platform::macos") {
            offenders.push(relative);
        }
    }
    offenders.sort();

    assert!(
        offenders.is_empty(),
        "business layers should use platform facades, not src/platform/macos directly:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn ipc_protocol_and_handlers_do_not_own_transport_backend() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    assert!(
        root.join("src/ipc/transport.rs").exists(),
        "Phase 3 IPC transport facade should live at src/ipc/transport.rs"
    );

    for file in [
        "src/ipc/protocol.rs",
        "src/ipc/client.rs",
        "src/ipc/server.rs",
    ] {
        let body = std::fs::read_to_string(root.join(file)).unwrap();
        for token in [
            "tokio::net::UnixListener",
            "tokio::net::UnixStream",
            "std::os::unix::net::UnixStream",
            "std::os::unix::net::UnixListener",
        ] {
            assert!(
                !body.contains(token),
                "{file} should depend on ipc::transport instead of `{token}`"
            );
        }
    }

    let transport = std::fs::read_to_string(root.join("src/ipc/transport.rs")).unwrap();
    assert!(
        transport.contains("UnixStream") || transport.contains("NamedPipe"),
        "transport facade should own concrete IPC backend types"
    );
}

#[test]
fn ipc_transport_backends_are_cfg_gated() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let transport = std::fs::read_to_string(root.join("src/ipc/transport.rs")).unwrap();

    for token in [
        "#[cfg(unix)]",
        "#[cfg(windows)]",
        "mod imp",
        "pub use imp::{",
        "default_endpoint",
        "bind_default",
        "connect",
        "Listener",
        "ReadHalf",
        "Stream",
        "WriteHalf",
    ] {
        assert!(
            transport.contains(token),
            "src/ipc/transport.rs should cfg-gate transport backend token `{token}`"
        );
    }

    let first_cfg = transport
        .find("#[cfg(unix)]")
        .expect("missing unix transport cfg");
    let pre_cfg = &transport[..first_cfg];
    for token in [
        "std::os::unix",
        "tokio::net::UnixListener",
        "tokio::net::UnixStream",
    ] {
        assert!(
            !pre_cfg.contains(token),
            "Unix-only token `{token}` must live inside a cfg-gated transport backend"
        );
    }
}

#[test]
fn windows_ipc_transport_uses_tokio_named_pipe_backend() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let transport = std::fs::read_to_string(root.join("src/ipc/transport.rs")).unwrap();
    let windows_section = transport
        .split("#[cfg(windows)]")
        .nth(1)
        .expect("missing windows IPC transport cfg section");

    for token in [
        "tokio::net::windows::named_pipe",
        "NamedPipeClient",
        "NamedPipeServer",
        "ServerOptions::new()",
        "ClientOptions::new()",
        ".first_pipe_instance(true)",
        ".connect().await",
        "ERROR_PIPE_BUSY",
    ] {
        assert!(
            windows_section.contains(token),
            "Windows IPC transport should use Tokio Named Pipe token `{token}`"
        );
    }

    assert!(
        !windows_section.contains("DuplexStream"),
        "Windows IPC transport should no longer use the placeholder duplex stream"
    );
}

#[test]
fn windows_capability_snapshot_marks_named_pipe_transport_partial() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let capability = std::fs::read_to_string(root.join("src/platform/capability.rs")).unwrap();

    for token in [
        "fn windows_capabilities()",
        "CapabilityId::IpcTransport",
        "CapabilityStatusKind::Partial",
        "named_pipe",
        "runtime_not_verified",
        "Validate Named Pipe transport on Windows",
    ] {
        assert!(
            capability.contains(token),
            "Windows capability snapshot should report Named Pipe compile backend token `{token}`"
        );
    }
}

#[test]
fn windows_lifecycle_primitives_have_compile_backend() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lifecycle = std::fs::read_to_string(root.join("src/platform/lifecycle.rs")).unwrap();
    let windows_section = lifecycle
        .split("#[cfg(windows)]")
        .nth(1)
        .expect("missing windows lifecycle cfg section");

    for token in [
        "CreateMutexW",
        "WaitForSingleObject",
        "ReleaseMutex",
        "OpenProcess",
        "CloseHandle",
        "PROCESS_QUERY_LIMITED_INFORMATION",
    ] {
        assert!(
            windows_section.contains(token),
            "Windows lifecycle backend should contain Win32 token `{token}`"
        );
    }
    assert!(
        !windows_section.contains("Windows daemon lock is not implemented"),
        "Windows daemon lock should no longer be a pure unsupported placeholder"
    );
    assert!(
        !windows_section.contains("Windows process probing is not implemented"),
        "Windows process probe should no longer be a pure unsupported placeholder"
    );

    let capability = std::fs::read_to_string(root.join("src/platform/capability.rs")).unwrap();
    for token in [
        "CapabilityId::DaemonSingleInstance",
        "named_mutex",
        "CapabilityId::ProcessProbe",
        "open_process_probe",
    ] {
        assert!(
            capability.contains(token),
            "Windows capability snapshot should reflect lifecycle compile backend token `{token}`"
        );
    }
}

#[test]
fn network_clients_use_rustls_for_cross_platform_checks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();

    for token in [
        "[target.'cfg(target_os = \"linux\")'.dependencies]",
        "reqwest         = { version = \"0.12\", default-features = false, features = [\"json\", \"rustls-tls\"] }",
        "tokio-tungstenite = { version = \"0.29\", default-features = false, features = [\"connect\", \"rustls-tls-webpki-roots\"] }",
        "[target.'cfg(not(target_os = \"linux\"))'.dependencies]",
        "reqwest         = { version = \"0.12\", default-features = false, features = [\"json\", \"native-tls\"] }",
        "tokio-tungstenite = { version = \"0.29\", features = [\"native-tls\"] }",
    ] {
        assert!(
            cargo.contains(token),
            "Cargo.toml should keep target-specific network TLS token `{token}`"
        );
    }

    let linux_section = cargo
        .split("[target.'cfg(target_os = \"linux\")'.dependencies]")
        .nth(1)
        .and_then(|section| {
            section
                .split("[target.'cfg(not(target_os = \"linux\"))'.dependencies]")
                .next()
        })
        .expect("missing linux dependency section");
    assert!(
        !linux_section.contains("native-tls"),
        "Linux dependencies must not use OpenSSL-backed native TLS"
    );
}

#[test]
fn daemon_lifecycle_primitives_live_behind_platform_facade() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    assert!(
        root.join("src/platform/lifecycle.rs").exists(),
        "Phase 4 daemon lifecycle facade should live at src/platform/lifecycle.rs"
    );

    let platform_mod = std::fs::read_to_string(root.join("src/platform/mod.rs")).unwrap();
    assert!(
        platform_mod.contains("pub(crate) mod lifecycle;"),
        "src/platform/mod.rs must expose the daemon lifecycle facade"
    );

    for (file, forbidden) in [
        ("src/daemon/process.rs", "DaemonLock"),
        ("src/platform/service.rs", "libc::kill"),
    ] {
        let body = std::fs::read_to_string(root.join(file)).unwrap();
        assert!(
            !body.contains(forbidden),
            "{file} should use platform::lifecycle instead of `{forbidden}`"
        );
    }
}

#[test]
fn service_manager_lives_behind_platform_facade() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    assert!(
        root.join("src/platform/service.rs").exists(),
        "Phase 4b service manager facade should live at src/platform/service.rs"
    );

    let platform_mod = std::fs::read_to_string(root.join("src/platform/mod.rs")).unwrap();
    assert!(
        platform_mod.contains("pub(crate) mod service;"),
        "src/platform/mod.rs must expose the service manager facade"
    );

    for file in ["src/cli/service/macos.rs", "src/cli/service/unsupported.rs"] {
        assert!(
            !root.join(file).exists(),
            "cli service should use platform::service instead of owning {file}"
        );
    }

    let cli_service = std::fs::read_to_string(root.join("src/cli/service/mod.rs")).unwrap();
    for token in ["launchctl", "plist_body", "gui_domain"] {
        assert!(
            !cli_service.contains(token),
            "src/cli/service/mod.rs should dispatch to platform::service instead of owning `{token}`"
        );
    }
}

#[test]
fn linux_service_manager_has_systemd_user_dry_run_skeleton() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let service = std::fs::read_to_string(root.join("src/platform/service.rs")).unwrap();

    for token in [
        "#[cfg(target_os = \"linux\")]",
        "fn unit_name()",
        "fn unit_path()",
        "fn unit_body(",
        "systemd.user: dry-run",
        "Restart=on-failure",
        "systemctl --user is intentionally not called",
    ] {
        assert!(
            service.contains(token),
            "Linux service manager skeleton should contain `{token}`"
        );
    }

    let linux_cfg = service
        .find("#[cfg(target_os = \"linux\")]")
        .expect("missing linux service cfg");
    let fallback_cfg = service
        .find("#[cfg(not(any(target_os = \"macos\", target_os = \"linux\", target_os = \"windows\")))]")
        .expect("missing non-linux fallback cfg");
    assert!(
        linux_cfg < fallback_cfg,
        "Linux service backend should be explicit, not folded into generic unsupported fallback"
    );
}

#[test]
fn windows_service_manager_has_dry_run_status_skeleton() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let service = std::fs::read_to_string(root.join("src/platform/service.rs")).unwrap();

    for token in [
        "#[cfg(target_os = \"windows\")]",
        "fn service_strategy()",
        "fn daemon_command(",
        "windows.user: dry-run",
        "install_start=unsupported",
        "Task Scheduler, SCM, PowerShell, and registry APIs are intentionally not called",
    ] {
        assert!(
            service.contains(token),
            "Windows service manager skeleton should contain `{token}`"
        );
    }

    let windows_cfg = service
        .find("#[cfg(target_os = \"windows\")]")
        .expect("missing windows service cfg");
    let fallback_cfg = service
        .find("#[cfg(not(any(target_os = \"macos\", target_os = \"linux\", target_os = \"windows\")))]")
        .expect("missing non-windows fallback cfg");
    assert!(
        windows_cfg < fallback_cfg,
        "Windows service backend should be explicit, not folded into generic unsupported fallback"
    );

    let capability = std::fs::read_to_string(root.join("src/platform/capability.rs")).unwrap();
    for token in [
        "CapabilityId::ServiceManager",
        "windows_user_dry_run",
        "dry_run_status_only",
        "Validate Windows user service install/start/stop strategy",
    ] {
        assert!(
            capability.contains(token),
            "Windows capability snapshot should reflect service dry-run token `{token}`"
        );
    }
}

#[test]
fn non_macos_desktop_capabilities_match_current_facade_behavior() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let capability = std::fs::read_to_string(root.join("src/platform/capability.rs")).unwrap();

    for token in [
        "non_macos_desktop_capabilities",
        "CapabilityId::DesktopHotkey",
        "CapabilityId::DesktopHotkeySuppression",
        "CapabilityId::DesktopClipboard",
        "CapabilityId::DesktopTextInjection",
        "CapabilityId::DesktopActiveApp",
        "default_context",
        "default_context_only",
        "CapabilityId::DesktopPermissions",
        "permission_probe_missing",
    ] {
        assert!(
            capability.contains(token),
            "Linux/Windows capability snapshots should explicitly model desktop facade token `{token}`"
        );
    }

    let desktop = std::fs::read_to_string(root.join("src/platform/desktop.rs")).unwrap();
    for token in [
        "AppContext::default()",
        "microphone_authorization() -> Option",
    ] {
        assert!(
            desktop.contains(token),
            "desktop facade current behavior should contain `{token}`"
        );
    }
}

#[test]
fn desktop_capabilities_live_behind_platform_desktop_facade() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    assert!(
        root.join("src/platform/desktop.rs").exists(),
        "Phase 5 desktop capability facade should live at src/platform/desktop.rs"
    );

    let platform_mod = std::fs::read_to_string(root.join("src/platform/mod.rs")).unwrap();
    assert!(
        platform_mod.contains("pub(crate) mod desktop;"),
        "src/platform/mod.rs must expose the desktop capability facade"
    );

    for (file, forbidden) in [
        ("src/voice/dispatch.rs", "platform::{autotype, clipboard}"),
        ("src/voice/engine.rs", "post::app_context::frontmost_app"),
        ("src/platform/daemon.rs", "post::app_context::frontmost_app"),
        (
            "src/tui/history/mod.rs",
            "platform::clipboard::write_string",
        ),
        ("src/cli/doctor.rs", "platform::permissions"),
    ] {
        let body = std::fs::read_to_string(root.join(file)).unwrap();
        assert!(
            !body.contains(forbidden),
            "{file} should use platform::desktop instead of `{forbidden}`"
        );
    }
}

#[test]
fn hotkey_provider_lives_behind_platform_hotkey_facade() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    assert!(
        root.join("src/platform/hotkey.rs").exists(),
        "Phase 5b hotkey provider facade should live at src/platform/hotkey.rs"
    );

    let platform_mod = std::fs::read_to_string(root.join("src/platform/mod.rs")).unwrap();
    assert!(
        platform_mod.contains("pub(crate) mod hotkey;"),
        "src/platform/mod.rs must expose the hotkey provider facade"
    );

    let daemon_platform = std::fs::read_to_string(root.join("src/platform/daemon.rs")).unwrap();
    for token in [
        "provider_darwin",
        "hotkey-eventtap",
        "thread::Builder",
        "daemon hotkey event tap is not implemented",
    ] {
        assert!(
            !daemon_platform.contains(token),
            "src/platform/daemon.rs should delegate hotkey provider details to platform::hotkey instead of owning `{token}`"
        );
    }
}

#[test]
fn overlay_renderer_lives_behind_renderer_facade() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    assert!(
        root.join("src/overlay/renderer.rs").exists(),
        "Phase 6 overlay renderer facade should live at src/overlay/renderer.rs"
    );

    let overlay_mod = std::fs::read_to_string(root.join("src/overlay/mod.rs")).unwrap();
    assert!(
        overlay_mod.contains("mod renderer;"),
        "src/overlay/mod.rs must include the renderer facade"
    );
    assert!(
        !overlay_mod.contains("pub use macos::run"),
        "src/overlay/mod.rs should expose run() through overlay::renderer, not re-export macOS"
    );

    let renderer = std::fs::read_to_string(root.join("src/overlay/renderer.rs")).unwrap();
    assert!(
        renderer.contains("macos::run"),
        "overlay::renderer should own the current macOS renderer selection"
    );

    for file in [
        "src/overlay/command.rs",
        "src/overlay/model.rs",
        "src/overlay/layout.rs",
    ] {
        let body = std::fs::read_to_string(root.join(file)).unwrap();
        assert!(
            !body.contains("overlay::macos") && !body.contains("crate::overlay::macos"),
            "{file} must stay shared and not import the macOS renderer"
        );
    }
}

#[test]
fn overlay_renderer_capabilities_live_with_renderer_facade() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let renderer = std::fs::read_to_string(root.join("src/overlay/renderer.rs")).unwrap();

    for token in [
        "renderer_capabilities",
        "CapabilityStatus",
        "CapabilityId::OverlayRenderer",
        "CapabilityId::OverlayMaterial",
        "CapabilityId::OverlayAlwaysOnTop",
        "CapabilityId::OverlayInputPassthrough",
        "CapabilityId::OverlayWindowAnchor",
        "MaterialPreference",
        "MATERIAL_FALLBACK_ORDER",
    ] {
        assert!(
            renderer.contains(token),
            "overlay::renderer should own renderer capability skeleton token `{token}`"
        );
    }

    for file in [
        "src/overlay/command.rs",
        "src/overlay/model.rs",
        "src/overlay/layout.rs",
    ] {
        let body = std::fs::read_to_string(root.join(file)).unwrap();
        assert!(
            !body.contains("CapabilityStatus") && !body.contains("CapabilityId"),
            "{file} must stay shared overlay behavior, not renderer capability reporting"
        );
    }
}

#[test]
fn overlay_renderer_capabilities_are_consumed_by_doctor_and_tui_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();

    for file in rust_files_under(&root.join("src")) {
        let relative = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if matches!(
            relative.as_str(),
            "src/overlay/mod.rs"
                | "src/overlay/renderer.rs"
                | "src/overlay/windows.rs"
                | "src/overlay/linux.rs"
                | "src/cli/doctor.rs"
                | "src/tui/status/render.rs"
        ) {
            continue;
        }

        let body = std::fs::read_to_string(&file).unwrap();
        if body.contains("renderer_capabilities") {
            offenders.push(relative);
        }
    }
    offenders.sort();

    assert!(
        offenders.is_empty(),
        "overlay renderer capability snapshot should only feed doctor and TUI status diagnostics:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn tui_status_consumes_capability_snapshots_without_gui_or_ipc() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let status_render = std::fs::read_to_string(root.join("src/tui/status/render.rs")).unwrap();

    for token in [
        "crate::platform::capability::current_platform_capabilities()",
        "crate::overlay::renderer_capabilities()",
        "platform_capability_lines",
    ] {
        assert!(
            status_render.contains(token),
            "TUI status should expose platform capability summary token `{token}`"
        );
    }

    for forbidden in [
        "tauri",
        "wry",
        "webview",
        "DaemonClient",
        "connect_default",
        "send_command",
        "subscribe_events",
        "tokio::spawn",
        "std::thread::spawn",
    ] {
        assert!(
            !status_render.contains(forbidden),
            "TUI status capability summary must stay read-only and avoid `{forbidden}`"
        );
    }
}

#[test]
fn overlay_windows_linux_backend_skeletons_are_cfg_gated_and_gui_free() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    for file in ["src/overlay/windows.rs", "src/overlay/linux.rs"] {
        assert!(
            root.join(file).exists(),
            "Phase 7b/8b overlay backend skeleton should exist at {file}"
        );
    }

    let renderer = std::fs::read_to_string(root.join("src/overlay/renderer.rs")).unwrap();
    for token in [
        "#[cfg(target_os = \"windows\")]",
        "#[cfg(target_os = \"linux\")]",
        "windows::run",
        "linux::run",
        "windows::renderer_capabilities",
        "linux::renderer_capabilities",
    ] {
        assert!(
            renderer.contains(token),
            "overlay::renderer should cfg-dispatch backend skeleton token `{token}`"
        );
    }

    let overlay_mod = std::fs::read_to_string(root.join("src/overlay/mod.rs")).unwrap();
    for token in [
        "#[cfg(target_os = \"windows\")]",
        "mod windows;",
        "#[cfg(target_os = \"linux\")]",
        "mod linux;",
    ] {
        assert!(
            overlay_mod.contains(token),
            "src/overlay/mod.rs should cfg-gate backend module token `{token}`"
        );
    }

    for file in rust_files_under(&root.join("src/overlay")) {
        let body = std::fs::read_to_string(&file).unwrap();
        let relative = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for token in ["tauri", "wry", "webview", "WebView", "tao"] {
            assert!(
                !body.contains(token),
                "{relative} must not import GUI/WebView runtime token `{token}` into overlay"
            );
        }
    }
}

#[test]
fn gui_client_api_boundary_stays_out_of_daemon_hot_path() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    assert!(
        root.join("src/client_api.rs").exists(),
        "Phase 9b shared GUI/TUI daemon client API should live at src/client_api.rs"
    );

    let main_rs = std::fs::read_to_string(root.join("src/main.rs")).unwrap();
    assert!(
        main_rs.contains("mod client_api;"),
        "src/main.rs must mount the shared daemon client API module"
    );

    let tui = std::fs::read_to_string(root.join("src/tui/mod.rs")).unwrap();
    assert!(
        tui.contains("crate::client_api::DaemonClient"),
        "TUI should obtain its daemon client type through client_api so GUI can reuse the same boundary"
    );
    assert!(
        !tui.contains("crate::ipc::client::IpcClient"),
        "TUI should not depend directly on ipc::client::IpcClient after the shared client API exists"
    );

    let forbidden = ["tauri", "wry", "webview", "WebView", "tao"];
    for file in rust_files_under(&root.join("src/daemon"))
        .into_iter()
        .chain(rust_files_under(&root.join("src/tui")))
        .chain([root.join("src/client_api.rs")])
    {
        let body = std::fs::read_to_string(&file).unwrap();
        let relative = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for token in forbidden {
            assert!(
                !body.contains(token),
                "{relative} must not pull GUI/WebView runtime token `{token}` into daemon/TUI/shared client path"
            );
        }
    }

    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for token in forbidden {
        assert!(
            !cargo.contains(token),
            "Phase 9b must not add GUI/WebView dependency token `{token}` to Cargo.toml"
        );
    }
}

#[test]
fn gui_first_screen_helpers_live_in_client_api_without_gui_runtime() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let client_api = std::fs::read_to_string(root.join("src/client_api.rs")).unwrap();

    for token in [
        "pub fn first_screen_commands",
        "pub enum FirstScreenEvent",
        "pub fn classify_first_screen_event",
        "Command::DaemonStatus",
        "Command::GetHistory",
        "Command::GetHistoryStats",
        "Event::DaemonStatus",
        "Event::History",
        "Event::HistoryStats",
    ] {
        assert!(
            client_api.contains(token),
            "src/client_api.rs should expose first-screen helper token `{token}`"
        );
    }

    assert!(
        !client_api.contains("PROTO_VERSION ="),
        "client_api helpers must not own or bump the IPC protocol version"
    );

    for token in ["tauri", "wry", "webview", "WebView", "tao"] {
        assert!(
            !client_api.contains(token),
            "src/client_api.rs must not pull GUI/WebView runtime token `{token}`"
        );
    }
}

#[test]
fn gui_library_boundary_keeps_root_runtime_free_after_workspace_creation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(
        cargo.contains("[[bin]]") && cargo.contains("name = \"shuo\""),
        "root package should still expose the existing shuo binary target"
    );
    assert!(
        cargo.contains("[lib]") || root.join("src/lib.rs").exists(),
        "root package should keep the reviewed library surface for GUI backend reuse"
    );

    for token in ["tauri", "wry", "webview", "WebView", "tao"] {
        assert!(
            !cargo.contains(token),
            "root Cargo.toml must not add GUI runtime dependency token `{token}`"
        );
    }
}

#[test]
fn gui_library_split_audit_records_minimal_surface_and_blockers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9e",
        "client_api",
        "ipc::client",
        "ipc::protocol",
        "ipc::transport",
        "history",
        "state",
        "Unix-only transport",
        "daemon runtime",
        "Tauri workspace",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9e library split audit token `{token}`"
        );
    }
}

#[test]
fn gui_minimal_library_split_exposes_only_client_protocol_surface() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    let lib_path = root.join("src/lib.rs");
    assert!(
        lib_path.exists(),
        "Phase 9f should add src/lib.rs as the minimal reusable GUI client library surface"
    );
    let lib = std::fs::read_to_string(&lib_path).unwrap();

    for token in [
        "pub mod client_api;",
        "pub mod history;",
        "pub mod ipc;",
        "pub mod paths;",
        "pub mod state;",
        "pub mod text_stats;",
    ] {
        assert!(
            lib.contains(token),
            "src/lib.rs should expose required client/protocol DTO surface token `{token}`"
        );
    }

    for token in [
        "mod daemon;",
        "pub mod daemon;",
        "mod cli;",
        "pub mod cli;",
        "mod tui;",
        "pub mod tui;",
        "mod overlay;",
        "pub mod overlay;",
        "mod platform;",
        "pub mod platform;",
        "mod voice;",
        "pub mod voice;",
        "mod hotkey;",
        "pub mod hotkey;",
        "mod config;",
        "pub mod config;",
        "mod reload;",
        "pub mod reload;",
    ] {
        assert!(
            !lib.contains(token),
            "src/lib.rs must not expose daemon/runtime/UI/platform implementation token `{token}`"
        );
    }

    let ipc_mod = std::fs::read_to_string(root.join("src/ipc/mod.rs")).unwrap();
    for token in ["pub mod client;", "pub mod protocol;", "pub mod transport;"] {
        assert!(
            ipc_mod.contains(token),
            "library IPC surface should expose `{token}`"
        );
    }
    assert!(
        !ipc_mod.contains("pub mod server;"),
        "library IPC surface must not expose ipc::server to GUI backend"
    );

    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for token in ["tauri", "wry", "webview", "WebView", "tao"] {
        assert!(
            !cargo.contains(token),
            "Phase 9f must not add GUI runtime dependency token `{token}` to Cargo.toml"
        );
    }
}

#[test]
fn gui_reconnect_state_skeleton_lives_in_client_api_without_runtime_loop() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let client_api = std::fs::read_to_string(root.join("src/client_api.rs")).unwrap();

    for token in [
        "pub enum DaemonConnectionState",
        "pub enum DaemonConnectionProblemKind",
        "pub struct DaemonConnectionProblem",
        "pub const DEFAULT_RECONNECT_DELAYS_MS",
        "pub fn next_reconnect_delay_ms",
        "pub fn reconnecting_state",
        "pub fn daemon_connect_failed_problem",
        "pub fn daemon_event_stream_closed_problem",
        "pub fn daemon_read_failed_problem",
    ] {
        assert!(
            client_api.contains(token),
            "src/client_api.rs should expose GUI reconnect skeleton token `{token}`"
        );
    }

    for token in [
        "tokio::spawn",
        "tokio::time::sleep",
        "connect_default().await",
        "PROTO_VERSION =",
    ] {
        assert!(
            !client_api.contains(token),
            "GUI reconnect skeleton should stay pure and not own runtime/protocol behavior token `{token}`"
        );
    }

    for file in rust_files_under(&root.join("src/daemon"))
        .into_iter()
        .chain(rust_files_under(&root.join("src/tui")))
    {
        let body = std::fs::read_to_string(&file).unwrap();
        let relative = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        assert!(
            !body.contains("DaemonConnectionState")
                && !body.contains("DaemonConnectionProblem")
                && !body.contains("next_reconnect_delay_ms"),
            "{relative} should not consume GUI reconnect skeleton until TUI/daemon behavior is designed"
        );
    }
}

#[test]
fn gui_backend_event_bridge_lives_in_client_api_without_gui_runtime() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let client_api = std::fs::read_to_string(root.join("src/client_api.rs")).unwrap();

    for token in [
        "pub enum GuiBackendEvent",
        "Daemon(FirstScreenEvent",
        "ConnectionState(&'a DaemonConnectionState)",
        "ConnectionProblem(&'a DaemonConnectionProblem)",
        "pub fn gui_backend_event_from_daemon_event",
        "pub fn gui_backend_event_from_connection_state",
        "pub fn gui_backend_event_from_connection_problem",
        "classify_first_screen_event(event).map(GuiBackendEvent::Daemon)",
    ] {
        assert!(
            client_api.contains(token),
            "src/client_api.rs should expose GUI backend event bridge token `{token}`"
        );
    }

    for token in [
        "tauri",
        "wry",
        "webview",
        "WebView",
        "tao",
        "tokio::spawn",
        "connect_default().await",
        "PROTO_VERSION =",
    ] {
        assert!(
            !client_api.contains(token),
            "GUI backend event bridge must not own GUI/runtime/protocol token `{token}`"
        );
    }
}

#[test]
fn gui_first_screen_metrics_timing_stays_pure_client_api() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let client_api = std::fs::read_to_string(root.join("src/client_api.rs")).unwrap();

    for token in [
        "pub struct FirstScreenReadiness",
        "pub struct FirstScreenTimingMarks",
        "pub struct FirstScreenTiming",
        "pub fn record_event",
        "pub fn is_ready",
        "pub fn from_marks",
        "saturating_sub",
        "FirstScreenEvent::DaemonStatus",
        "FirstScreenEvent::HistoryPage",
        "FirstScreenEvent::HistoryStats",
    ] {
        assert!(
            client_api.contains(token),
            "src/client_api.rs should expose GUI first-screen timing token `{token}`"
        );
    }

    for token in [
        "tauri",
        "wry",
        "webview",
        "WebView",
        "tao",
        "tokio::spawn",
        "tokio::time",
        "std::time::Instant",
        "Instant::now",
        "connect_default().await",
        "PROTO_VERSION =",
    ] {
        assert!(
            !client_api.contains(token),
            "GUI first-screen metrics timing must stay pure and not own runtime/protocol token `{token}`"
        );
    }
}

#[test]
fn gui_tauri_permissions_preflight_is_documented_without_root_runtime() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9j",
        "capabilities",
        "permissions",
        "windows/webviews",
        "主 window/webview",
        "scopes",
        "shell",
        "filesystem",
        "http",
        "sidecar",
        "core:default",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9j Tauri permissions preflight token `{token}`"
        );
    }

    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for token in ["tauri", "wry", "webview", "WebView", "tao"] {
        assert!(
            !cargo.contains(token),
            "Phase 9j must not add GUI runtime dependency token `{token}` to Cargo.toml"
        );
    }
}

#[test]
fn gui_tauri_workspace_pre_creation_acceptance_is_documented_without_root_runtime() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9k",
        "允许新增路径",
        "scope creep",
        "自动验收",
        "release build",
        "tauri build",
        "tauri bundle",
        "build.frontendDist",
        "bundle path/type",
        "cold start",
        "首屏 ready",
        "idle RSS/CPU",
        "daemon 未打开 GUI 时无 WebView/Tauri 进程",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9k workspace pre-creation token `{token}`"
        );
    }

    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for token in ["tauri", "wry", "webview", "WebView", "tao"] {
        assert!(
            !cargo.contains(token),
            "Phase 9k must not add GUI runtime dependency token `{token}` to Cargo.toml"
        );
    }
}

#[test]
fn gui_reconnect_supervisor_ownership_is_documented_without_root_runtime_loop() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9l",
        "connection supervisor",
        "GUI window/app lifecycle",
        "session id/generation",
        "connect failed",
        "event stream closed",
        "read failed",
        "不能自动启动 daemon",
        "不能安装或重启 service",
        "timer、spawn、channel",
        "Tauri event emission",
        "FirstScreenTiming::from_marks",
        "metrics sink",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9l reconnect supervisor token `{token}`"
        );
    }

    let client_api = std::fs::read_to_string(root.join("src/client_api.rs")).unwrap();
    for token in [
        "tokio::spawn",
        "tokio::time",
        "std::thread::spawn",
        "std::time::Instant",
        "connect_default().await",
        "tauri",
        "wry",
        "webview",
        "WebView",
        "tao",
    ] {
        assert!(
            !client_api.contains(token),
            "Phase 9l must keep shared client_api pure and free of runtime/GUI token `{token}`"
        );
    }
}

#[test]
fn gui_minimal_tauri_workspace_skeleton_is_isolated_from_root_runtime() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    for file in [
        "src-tauri/Cargo.toml",
        "src-tauri/tauri.conf.json",
        "src-tauri/build.rs",
        "src-tauri/src/main.rs",
        "src-tauri/src/lib.rs",
        "src-tauri/capabilities/default.json",
        "src-tauri/Cargo.lock",
    ] {
        assert!(
            root.join(file).exists(),
            "Phase 9m should create minimal Tauri skeleton file {file}"
        );
    }

    let tauri_cargo = std::fs::read_to_string(root.join("src-tauri/Cargo.toml")).unwrap();
    for token in [
        "name = \"shuohua-gui\"",
        "tauri = { version = \"2\"",
        "tauri-build = { version = \"2\"",
        "shuohua = { path = \"..\" }",
    ] {
        assert!(
            tauri_cargo.contains(token),
            "src-tauri/Cargo.toml should contain minimal GUI skeleton token `{token}`"
        );
    }

    let tauri_conf = std::fs::read_to_string(root.join("src-tauri/tauri.conf.json")).unwrap();
    for token in [
        "\"productName\": \"Shuohua\"",
        "\"identifier\": \"dev.shuohua.app\"",
        "\"frontendDist\": \"../gui-dist\"",
        "\"active\": false",
        "\"icon\": [\"../assets/icon/shuohua-icon-1024.png\"]",
        "\"label\": \"main\"",
        "\"title\": \"Shuohua\"",
    ] {
        assert!(
            tauri_conf.contains(token),
            "src-tauri/tauri.conf.json should contain minimal app config token `{token}`"
        );
    }

    let capability =
        std::fs::read_to_string(root.join("src-tauri/capabilities/default.json")).unwrap();
    for token in [
        "\"identifier\": \"main\"",
        "\"windows\": [\"main\"]",
        "\"permissions\"",
        "\"core:event:default\"",
    ] {
        assert!(
            capability.contains(token),
            "src-tauri/capabilities/default.json should contain minimal capability token `{token}`"
        );
    }

    let root_cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for token in ["tauri", "wry", "webview", "WebView", "tao"] {
        assert!(
            !root_cargo.contains(token),
            "root Cargo.toml must not depend on GUI runtime token `{token}`"
        );
    }

    for file in rust_files_under(&root.join("src/daemon"))
        .into_iter()
        .chain(rust_files_under(&root.join("src/tui")))
        .chain([root.join("src/client_api.rs")])
    {
        let body = std::fs::read_to_string(&file).unwrap();
        let relative = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for token in ["tauri", "wry", "webview", "WebView", "tao"] {
            assert!(
                !body.contains(token),
                "{relative} must not import GUI runtime token `{token}`"
            );
        }
    }
}

#[test]
fn gui_backend_shell_placeholder_stays_local_to_tauri_app() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9n",
        "metadata command",
        "gui-dist/index.html",
        "不接 daemon",
        "不读配置/history",
        "不实现 reconnect supervisor",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9n GUI shell token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "#[tauri::command]",
        "gui_shell_metadata",
        "GuiShellMetadata",
        "invoke_handler",
        "tauri::generate_handler!",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should wire minimal GUI shell command token `{token}`"
        );
    }

    for token in ["tokio::spawn", "tokio::time", "std::thread::spawn"] {
        assert!(
            !tauri_lib.contains(token),
            "Phase 9n GUI shell must not connect daemon or own runtime loop token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "Shuohua",
        "gui_shell_metadata",
        "__TAURI__",
        "daemonConnected",
        "false",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should contain minimal placeholder token `{token}`"
        );
    }

    for path in ["package.json", "vite.config.js", "gui-dist/package.json"] {
        assert!(
            !root.join(path).exists(),
            "Phase 9n should not introduce frontend package/build config {path}"
        );
    }

    let root_cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for token in ["tauri", "wry", "webview", "WebView", "tao"] {
        assert!(
            !root_cargo.contains(token),
            "root Cargo.toml must remain free of GUI runtime token `{token}`"
        );
    }

    for file in rust_files_under(&root.join("src/daemon"))
        .into_iter()
        .chain(rust_files_under(&root.join("src/tui")))
        .chain([root.join("src/client_api.rs")])
    {
        let body = std::fs::read_to_string(&file).unwrap();
        let relative = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for token in ["tauri", "wry", "webview", "WebView", "tao"] {
            assert!(
                !body.contains(token),
                "{relative} must not import GUI runtime token `{token}`"
            );
        }
    }
}

#[test]
fn gui_first_screen_request_plan_reuses_client_api_without_sending_ipc() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9o",
        "first-screen request plan command",
        "first_screen_commands()",
        "不发送 IPC",
        "不订阅 daemon event stream",
        "不启动 reconnect loop",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9o request plan token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "gui_first_screen_request_plan",
        "GuiFirstScreenRequestPlan",
        "GuiFirstScreenCommandSummary",
        "shuohua::client_api::first_screen_commands",
        "tauri::generate_handler!",
        "gui_shell_metadata",
        "history_limit",
        "requires_daemon_connection",
        "transport_opened",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should expose Phase 9o request plan token `{token}`"
        );
    }

    for token in [
        "subscribe_events",
        "tokio::spawn",
        "tokio::time",
        "std::thread::spawn",
    ] {
        assert!(
            !tauri_lib.contains(token),
            "Phase 9o request plan must not connect daemon or own runtime loop token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "gui_first_screen_request_plan",
        "historyLimit",
        "requiresDaemonConnection",
        "transportOpened",
        "request-plan-count",
        "request-plan-kinds",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should display request plan summary token `{token}`"
        );
    }

    for file in rust_files_under(&root.join("src/daemon"))
        .into_iter()
        .chain(rust_files_under(&root.join("src/tui")))
        .chain([root.join("src/client_api.rs")])
    {
        let body = std::fs::read_to_string(&file).unwrap();
        let relative = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for token in ["tauri", "wry", "webview", "WebView", "tao"] {
            assert!(
                !body.contains(token),
                "{relative} must not import GUI runtime token `{token}`"
            );
        }
    }
}

#[test]
fn gui_daemon_status_snapshot_shape_does_not_send_ipc() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9p",
        "daemon status snapshot command",
        "shape preflight",
        "Command::DaemonStatus",
        "不发送 IPC",
        "不得调用 `send_command`",
        "不得订阅 daemon event stream",
        "不启动 reconnect loop",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9p daemon status shape token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "gui_daemon_status_snapshot",
        "GuiDaemonStatusSnapshot",
        "GuiDaemonStatusRequestSummary",
        "Command::DaemonStatus",
        "connected",
        "transport_opened",
        "snapshot_available",
        "request_kind",
        "state_label",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should expose Phase 9p daemon status shape token `{token}`"
        );
    }

    for token in [
        "subscribe_events",
        "tokio::spawn",
        "tokio::time",
        "std::thread::spawn",
    ] {
        assert!(
            !tauri_lib.contains(token),
            "Phase 9p daemon status shape must not connect daemon or own runtime loop token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "gui_daemon_status_snapshot",
        "status-connected",
        "status-transport-opened",
        "status-snapshot-available",
        "status-request-kind",
        "status-state-label",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should display daemon status shape token `{token}`"
        );
    }

    for file in rust_files_under(&root.join("src/daemon"))
        .into_iter()
        .chain(rust_files_under(&root.join("src/tui")))
        .chain([root.join("src/client_api.rs")])
    {
        let body = std::fs::read_to_string(&file).unwrap();
        let relative = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for token in ["tauri", "wry", "webview", "WebView", "tao"] {
            assert!(
                !body.contains(token),
                "{relative} must not import GUI runtime token `{token}`"
            );
        }
    }
}

#[test]
fn gui_daemon_status_event_mapper_is_pure_and_local_to_tauri_app() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9q",
        "daemon status event mapper",
        "Event::DaemonStatus",
        "connected=true",
        "transportOpened=true",
        "snapshotAvailable=true",
        "不得调用 `send_command`",
        "不得订阅 daemon event stream",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9q daemon status mapper token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "gui_daemon_status_snapshot_from_event",
        "Event::DaemonStatus",
        "WireState::Idle",
        "WireState::Recording",
        "WireState::Stopping",
        "WireState::Error",
        "pid",
        "uptime_ms",
        "recording_id",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should expose Phase 9q daemon status mapper token `{token}`"
        );
    }

    for token in [
        "subscribe_events",
        "tokio::spawn",
        "tokio::time",
        "std::thread::spawn",
    ] {
        assert!(
            !tauri_lib.contains(token),
            "Phase 9q daemon status mapper must not connect daemon or own runtime loop token `{token}`"
        );
    }
}

#[test]
fn gui_daemon_status_one_shot_request_is_explicit_and_bounded() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9r",
        "gui_daemon_status_request_once",
        "Command::DaemonStatus",
        "Event::DaemonStatus",
        "placeholder 当前不自动调用",
        "recoverable",
        "不订阅 daemon event stream",
        "不启动 reconnect loop",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9r one-shot status token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "gui_daemon_status_request_once",
        "GuiDaemonStatusRequestError",
        "DaemonClient::connect_default()",
        ".await",
        ".send(&Command::DaemonStatus)",
        ".recv_until(",
        "gui_daemon_status_snapshot_from_event",
        "tauri::generate_handler!",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should expose Phase 9r one-shot status token `{token}`"
        );
    }

    for token in [
        "client.send(&Command::Subscribe)",
        "subscribe_events",
        "tokio::spawn",
        "tokio::time",
        "std::thread::spawn",
    ] {
        assert!(
            !tauri_lib.contains(token),
            "Phase 9r one-shot status request must not subscribe, reconnect, or spawn token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    assert!(
        !frontend.contains("gui_daemon_status_request_once"),
        "placeholder must not auto-call the one-shot daemon status request"
    );
}

#[test]
fn gui_history_summary_one_shot_request_is_explicit_and_bounded() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9s",
        "gui_history_summary_request_once",
        "Command::GetHistory",
        "Command::GetHistoryStats",
        "Event::History",
        "Event::HistoryStats",
        "placeholder 当前不自动调用",
        "recoverable",
        "不订阅 daemon event stream",
        "不启动 reconnect loop",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9s one-shot history summary token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "gui_history_summary_request_once",
        "GuiHistorySummary",
        "GuiHistoryRecordSummary",
        "GuiHistoryAggregateStats",
        "GuiHistorySummaryRequestError",
        "DaemonClient::connect_default()",
        ".send(&history_summary_page_command(",
        ".send(&Command::GetHistoryStats)",
        ".recv_until(",
        "gui_history_summary_from_events",
        "Event::History",
        "Event::HistoryStats",
        "tauri::generate_handler!",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should expose Phase 9s one-shot history summary token `{token}`"
        );
    }

    for token in [
        "client.send(&Command::Subscribe)",
        "subscribe_events",
        "tokio::spawn",
        "tokio::time",
        "std::thread::spawn",
    ] {
        assert!(
            !tauri_lib.contains(token),
            "Phase 9s one-shot history summary request must not subscribe, reconnect, or spawn token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    assert!(
        !frontend.contains("gui_history_summary_request_once"),
        "placeholder must not auto-call the one-shot history summary request"
    );
}

#[test]
fn gui_first_screen_summary_one_shot_request_is_explicit_and_bounded() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9t",
        "gui_first_screen_summary_request_once",
        "Command::DaemonStatus",
        "Command::GetHistory",
        "Command::GetHistoryStats",
        "Event::DaemonStatus",
        "Event::History",
        "Event::HistoryStats",
        "placeholder 当前不自动调用",
        "recoverable",
        "不订阅 daemon event stream",
        "不启动 reconnect loop",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9t first-screen summary token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "gui_first_screen_summary_request_once",
        "GuiFirstScreenSummary",
        "GuiFirstScreenSummaryRequestSummary",
        "GuiFirstScreenSummaryRequestError",
        "DaemonClient::connect_default()",
        ".send(&Command::DaemonStatus)",
        ".send(&history_summary_page_command(",
        ".send(&Command::GetHistoryStats)",
        ".recv_until(",
        "gui_daemon_status_snapshot_from_event",
        "gui_history_summary_from_events",
        "Event::DaemonStatus",
        "Event::History",
        "Event::HistoryStats",
        "tauri::generate_handler!",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should expose Phase 9t first-screen summary token `{token}`"
        );
    }

    for token in [
        "client.send(&Command::Subscribe)",
        "subscribe_events",
        "tokio::spawn",
        "tokio::time",
        "std::thread::spawn",
    ] {
        assert!(
            !tauri_lib.contains(token),
            "Phase 9t one-shot first-screen summary must not subscribe, reconnect, or spawn token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    assert_first_screen_summary_invoke_is_explicit(&frontend);
}

#[test]
fn gui_first_screen_summary_timing_stays_local_to_one_shot_request() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9u",
        "connectDurationMs",
        "firstEventMs",
        "readyMs",
        "requestDurationMs",
        "std::time::Instant",
        "不使用 `tokio::time`",
        "不启动 timer task",
        "不进入 daemon protocol",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9u timing token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "GuiFirstScreenSummaryTiming",
        "timing: GuiFirstScreenSummaryTiming",
        "std::time::Instant::now()",
        "connect_duration_ms",
        "first_event_ms",
        "ready_ms",
        "request_duration_ms",
        "gui_first_screen_summary_from_parts",
        "gui_first_screen_summary_from_events(",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should expose Phase 9u timing token `{token}`"
        );
    }

    for token in [
        "client.send(&Command::Subscribe)",
        "subscribe_events",
        "tokio::spawn",
        "tokio::time",
        "std::thread::spawn",
    ] {
        assert!(
            !tauri_lib.contains(token),
            "Phase 9u timing must not subscribe, reconnect, spawn, or own timer token `{token}`"
        );
    }
}

#[test]
fn gui_first_screen_refresh_shape_is_static_and_explicit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9v",
        "gui_first_screen_refresh_shape",
        "refresh preflight",
        "gui_first_screen_summary_request_once",
        "不得调用 `gui_first_screen_summary_request_once`",
        "不得订阅 daemon",
        "不启动 reconnect loop",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9v refresh shape token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "gui_first_screen_refresh_shape",
        "GuiFirstScreenRefreshShape",
        "explicit_trigger_required",
        "default_history_limit",
        "requires_daemon_connection",
        "transport_opened",
        "invoke_target",
        "gui_first_screen_summary_request_once",
        "tauri::generate_handler!",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should expose Phase 9v refresh shape token `{token}`"
        );
    }

    for token in [
        "client.send(&Command::Subscribe)",
        "subscribe_events",
        "tokio::spawn",
        "tokio::time",
        "std::thread::spawn",
    ] {
        assert!(
            !tauri_lib.contains(token),
            "Phase 9v refresh shape must not subscribe, reconnect, spawn, or own timer token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "gui_first_screen_refresh_shape",
        "refresh-explicit-trigger",
        "refresh-default-history-limit",
        "refresh-invoke-target",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should display refresh shape token `{token}`"
        );
    }
    assert_first_screen_summary_invoke_is_explicit(&frontend);
}

#[test]
fn gui_first_screen_readiness_shape_is_static_display_preflight() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9w",
        "gui_first_screen_readiness_shape",
        "display preflight",
        "ready=false",
        "timing 字段暂不可用",
        "std::time::Instant::now()",
        "不得启动 timer/reconnect loop",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9w readiness display token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "gui_first_screen_readiness_shape",
        "GuiFirstScreenReadinessShape",
        "GuiFirstScreenReadinessInputs",
        "GuiFirstScreenReadinessTimingShape",
        "ready",
        "daemon_status_received",
        "history_page_received",
        "history_stats_received",
        "connect_duration_ms",
        "first_event_ms",
        "ready_ms",
        "source",
        "placeholder",
        "tauri::generate_handler!",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should expose Phase 9w readiness display token `{token}`"
        );
    }

    for token in [
        "client.send(&Command::Subscribe)",
        "subscribe_events",
        "tokio::spawn",
        "tokio::time",
        "std::thread::spawn",
    ] {
        assert!(
            !tauri_lib.contains(token),
            "Phase 9w readiness shape must not subscribe, reconnect, spawn, or own timer token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "gui_first_screen_readiness_shape",
        "readiness-ready",
        "readiness-required-inputs",
        "readiness-timing",
        "readiness-source",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should display readiness shape token `{token}`"
        );
    }
    assert_first_screen_summary_invoke_is_explicit(&frontend);
}

#[test]
fn gui_first_screen_offline_shape_is_static_display_preflight() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9x",
        "gui_first_screen_offline_shape",
        "offline/error display preflight",
        "recoverable",
        "不允许自动启动 daemon",
        "不得安装/重启 service",
        "不得启动 timer/reconnect loop",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9x offline display token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "gui_first_screen_offline_shape",
        "GuiFirstScreenOfflineShape",
        "connected",
        "problem_kind",
        "recoverable",
        "retry_allowed",
        "auto_start_allowed",
        "service_management_allowed",
        "source",
        "placeholder",
        "tauri::generate_handler!",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should expose Phase 9x offline display token `{token}`"
        );
    }

    for token in [
        "client.send(&Command::Subscribe)",
        "subscribe_events",
        "tokio::spawn",
        "tokio::time",
        "std::thread::spawn",
        "install_service",
        "restart_service",
    ] {
        assert!(
            !tauri_lib.contains(token),
            "Phase 9x offline shape must not subscribe, reconnect, spawn, or manage service token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "gui_first_screen_offline_shape",
        "offline-problem-kind",
        "offline-recoverable",
        "offline-retry-allowed",
        "offline-auto-start",
        "offline-service-management",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should display offline shape token `{token}`"
        );
    }
    assert_first_screen_summary_invoke_is_explicit(&frontend);
}

#[test]
fn gui_first_screen_command_policy_shape_keeps_one_shots_explicit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9y",
        "gui_first_screen_command_policy_shape",
        "policy preflight",
        "允许自动调用",
        "explicit-only",
        "不得调用 `gui_first_screen_summary_request_once`",
        "不得启动 timer/reconnect loop",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9y command policy token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "gui_first_screen_command_policy_shape",
        "GuiFirstScreenCommandPolicyShape",
        "GuiFirstScreenCommandPolicyEntry",
        "command_name",
        "auto_invocation_allowed",
        "requires_explicit_trigger",
        "opens_daemon_transport",
        "policy_reason",
        "gui_shell_metadata",
        "gui_first_screen_summary_request_once",
        "tauri::generate_handler!",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should expose Phase 9y command policy token `{token}`"
        );
    }

    for token in [
        "client.send(&Command::Subscribe)",
        "subscribe_events",
        "tokio::spawn",
        "tokio::time",
        "std::thread::spawn",
    ] {
        assert!(
            !tauri_lib.contains(token),
            "Phase 9y policy shape must not subscribe, reconnect, spawn, or own timer token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "gui_first_screen_command_policy_shape",
        "policy-auto-count",
        "policy-explicit-count",
        "policy-one-shot-commands",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should display command policy token `{token}`"
        );
    }
    assert_first_screen_summary_invoke_is_explicit(&frontend);
}

#[test]
fn gui_first_screen_refresh_affordance_shape_stays_static() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9z",
        "gui_first_screen_refresh_affordance_shape",
        "affordance preflight",
        "不得注册真实 click handler",
        "不得调用 `gui_first_screen_summary_request_once`",
        "loading=false",
        "不得启动 timer/reconnect loop",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9z refresh affordance token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "gui_first_screen_refresh_affordance_shape",
        "GuiFirstScreenRefreshAffordanceShape",
        "button_label",
        "enabled",
        "explicit_trigger_required",
        "invoke_target",
        "default_history_limit",
        "loading",
        "source",
        "tauri::generate_handler!",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should expose Phase 9z refresh affordance token `{token}`"
        );
    }

    for token in [
        "client.send(&Command::Subscribe)",
        "subscribe_events",
        "tokio::spawn",
        "tokio::time",
        "std::thread::spawn",
    ] {
        assert!(
            !tauri_lib.contains(token),
            "Phase 9z refresh affordance shape must not subscribe, reconnect, spawn, or own timer token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "gui_first_screen_refresh_affordance_shape",
        "refresh-affordance-label",
        "refresh-affordance-enabled",
        "refresh-affordance-loading",
        "refresh-affordance-source",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should display refresh affordance token `{token}`"
        );
    }
    assert!(
        !frontend.contains("onclick="),
        "placeholder must not register inline refresh click handlers"
    );
    assert_first_screen_summary_invoke_is_explicit(&frontend);
}

#[test]
fn gui_first_screen_refresh_click_wiring_is_explicit_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9aa",
        "explicit refresh click wiring",
        "只在用户显式点击后调用",
        "初始加载仍不得自动调用该 one-shot command",
        "不得订阅 daemon event stream",
        "不得启动 timer/reconnect loop",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9aa click wiring token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "refresh-action-button",
        "refresh-action-status",
        "refresh-action-result",
        "handleExplicitRefresh",
        "addEventListener(\"click\", handleExplicitRefresh)",
        "gui_first_screen_summary_request_once",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should expose explicit refresh click token `{token}`"
        );
    }

    let one_shot_invoke_pos = frontend
        .find("invoke(\"gui_first_screen_summary_request_once\"")
        .expect("explicit refresh target should be present");
    let click_handler_pos = frontend
        .find("function handleExplicitRefresh")
        .expect("explicit refresh handler should be present");
    assert!(
        one_shot_invoke_pos > click_handler_pos,
        "one-shot summary invoke must stay inside the explicit click handler"
    );

    for token in [
        "setInterval",
        "setTimeout",
        "subscribe_events",
        "client.send(&Command::Subscribe)",
        "install_service",
        "restart_service",
    ] {
        assert!(
            !frontend.contains(token),
            "Phase 9aa frontend must not subscribe, loop, or manage service token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "client.send(&Command::Subscribe)",
        "subscribe_events",
        "tokio::spawn",
        "tokio::time",
        "std::thread::spawn",
        "install_service",
        "restart_service",
    ] {
        assert!(
            !tauri_lib.contains(token),
            "Phase 9aa must not add backend subscription, reconnect, spawn, timer, or service token `{token}`"
        );
    }
}

#[test]
fn gui_first_screen_refresh_result_projection_stays_click_scoped() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9ab",
        "explicit refresh result projection",
        "projection 必须只发生在 explicit refresh click 成功路径内",
        "不新增 Tauri command",
        "不建立完整",
        "不启动 daemon/GUI",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9ab projection token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "projectExplicitRefreshSummary",
        "projectExplicitRefreshSummary(summary)",
        "summary.status.stateLabel",
        "summary.history.pageRecordCount",
        "summary.summaryAvailable",
        "refresh-action-result",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should expose Phase 9ab projection token `{token}`"
        );
    }

    let projection_call_pos = frontend
        .find("projectExplicitRefreshSummary(summary)")
        .expect("explicit refresh projection call should be present");
    let click_handler_pos = frontend
        .find("function handleExplicitRefresh")
        .expect("explicit refresh handler should be present");
    let catch_pos = frontend
        .find("catch (error)")
        .expect("explicit refresh error path should be present");
    assert!(
        projection_call_pos > click_handler_pos && projection_call_pos < catch_pos,
        "summary projection must stay in the explicit refresh success path"
    );

    for token in [
        "setInterval",
        "setTimeout",
        "subscribe_events",
        "client.send(&Command::Subscribe)",
        "install_service",
        "restart_service",
    ] {
        assert!(
            !frontend.contains(token),
            "Phase 9ab frontend must not subscribe, loop, or manage service token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    assert!(
        !tauri_lib.contains("gui_first_screen_refresh_result_projection"),
        "Phase 9ab must not add a backend projection command"
    );
}

#[test]
fn gui_first_screen_refresh_error_projection_stays_catch_scoped() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9ac",
        "explicit refresh error projection",
        "error projection 必须只发生在 explicit refresh click 的 catch 路径内",
        "不得实现 retry loop",
        "不新增 Tauri command",
        "不启动 daemon/GUI",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9ac error projection token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "projectExplicitRefreshError",
        "projectExplicitRefreshError(error)",
        "error?.kind",
        "error?.recoverable",
        "offline-problem-kind",
        "offline-recoverable",
        "offline-retry-allowed",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should expose Phase 9ac error projection token `{token}`"
        );
    }

    let error_projection_pos = frontend
        .find("projectExplicitRefreshError(error)")
        .expect("explicit refresh error projection call should be present");
    let catch_pos = frontend
        .find("catch (error)")
        .expect("explicit refresh error path should be present");
    let finally_pos = frontend
        .find("finally")
        .expect("explicit refresh finally path should be present");
    assert!(
        error_projection_pos > catch_pos && error_projection_pos < finally_pos,
        "error projection must stay in the explicit refresh catch path"
    );

    for token in [
        "setInterval",
        "setTimeout",
        "subscribe_events",
        "client.send(&Command::Subscribe)",
        "install_service",
        "restart_service",
    ] {
        assert!(
            !frontend.contains(token),
            "Phase 9ac frontend must not subscribe, loop, or manage service token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    assert!(
        !tauri_lib.contains("gui_first_screen_refresh_error_projection"),
        "Phase 9ac must not add a backend error projection command"
    );
}

#[test]
fn gui_first_screen_refresh_success_clears_offline_display() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9ad",
        "success clears offline display",
        "清理必须只发生在 explicit refresh click 成功路径内",
        "stale error",
        "不新增 Tauri command",
        "不启动 daemon/GUI",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9ad success clear token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "projectExplicitRefreshSummary",
        "projectExplicitRefreshError",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should expose Phase 9ad success clear token `{token}`"
        );
    }

    let success_projection_pos = frontend
        .find("function projectExplicitRefreshSummary")
        .expect("success projection function should be present");
    let error_projection_pos = frontend
        .find("function projectExplicitRefreshError")
        .expect("error projection function should be present");
    assert!(
        success_projection_pos < error_projection_pos,
        "success clear logic should stay in the success projection, not the error projection"
    );
    let success_projection = &frontend[success_projection_pos..error_projection_pos];
    for token in [
        "offline-problem-kind",
        "connected",
        "offline-recoverable",
        "false",
        "offline-retry-allowed",
    ] {
        assert!(
            success_projection.contains(token),
            "success projection should clear stale offline display token `{token}`"
        );
    }

    for token in [
        "setInterval",
        "setTimeout",
        "subscribe_events",
        "client.send(&Command::Subscribe)",
        "install_service",
        "restart_service",
    ] {
        assert!(
            !frontend.contains(token),
            "Phase 9ad frontend must not subscribe, loop, or manage service token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    assert!(
        !tauri_lib.contains("gui_first_screen_refresh_clear_offline"),
        "Phase 9ad must not add a backend clear-offline command"
    );
}

#[test]
fn gui_frontend_invokes_are_authorized_and_init_errors_are_visible() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9ae",
        "application command",
        "allow-<command-name-kebab-case>",
        "refresh click handler 必须在任何 awaited initialization invoke 之前绑定",
        "不能静默吞掉",
        "recording state streaming",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9ae init/permission token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    let capability =
        std::fs::read_to_string(root.join("src-tauri/capabilities/default.json")).unwrap();
    let permissions = std::fs::read_to_string(root.join("src-tauri/permissions/gui.toml"))
        .expect("Phase 9ae should define app command permissions");

    let invoked_commands = [
        "gui_shell_metadata",
        "gui_first_screen_request_plan",
        "gui_daemon_status_snapshot",
        "gui_first_screen_refresh_shape",
        "gui_first_screen_refresh_affordance_shape",
        "gui_first_screen_readiness_shape",
        "gui_first_screen_offline_shape",
        "gui_first_screen_command_policy_shape",
        "gui_first_screen_summary_request_once",
    ];
    for command in invoked_commands {
        assert!(
            frontend.contains(&format!("\"{command}\"")),
            "gui-dist/index.html should invoke audited command `{command}`"
        );
        let permission = tauri_application_command_permission(command);
        assert!(
            capability.contains(&format!("\"{permission}\"")),
            "src-tauri/capabilities/default.json should authorize frontend command `{command}` with `{permission}`"
        );
        assert!(
            permissions.contains(&format!("identifier = \"{permission}\""))
                && permissions.contains(&format!("commands.allow = [\"{command}\"]")),
            "src-tauri/permissions/gui.toml should define `{permission}` for `{command}`"
        );
    }

    for forbidden in [
        "shell:",
        "fs:",
        "http:",
        "process:",
        "global-shortcut:",
        "\"core:default\"",
    ] {
        assert!(
            !capability.contains(forbidden),
            "Phase 9ae capability must not enable broad permission token `{forbidden}`"
        );
    }

    let bind_pos = frontend
        .find("addEventListener(\"click\", handleExplicitRefresh)")
        .expect("refresh click handler should be bound");
    let first_invoke_pos = frontend
        .find("await invoke(")
        .expect("frontend should await initialization invokes");
    assert!(
        bind_pos < first_invoke_pos,
        "refresh click handler must be registered before awaited initialization invokes can fail"
    );

    for token in [
        "projectInitializationError",
        "projectInitializationError(error)",
        "refresh-action-status",
        "init-error",
        "refresh-action-result",
        "String(error?.message ?? error)",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should expose visible initialization error token `{token}`"
        );
    }
    assert!(
        !frontend.contains("loadShellMetadata().catch(() => {})"),
        "initialization failures must not be swallowed silently"
    );

    for token in [
        "setInterval",
        "setTimeout",
        "subscribe_events",
        "client.send(&Command::Subscribe)",
        "install_service",
        "restart_service",
    ] {
        assert!(
            !frontend.contains(token),
            "Phase 9ae frontend must not subscribe, loop, or manage service token `{token}`"
        );
    }
}

#[test]
fn gui_static_frontend_global_tauri_api_is_enabled_and_missing_api_is_visible() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9af",
        "withGlobalTauri = true",
        "window.__TAURI__.core.invoke",
        "tauri-api-missing",
        "不得静默 return",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9af global API token `{token}`"
        );
    }

    let tauri_conf = std::fs::read_to_string(root.join("src-tauri/tauri.conf.json")).unwrap();
    assert!(
        tauri_conf.contains("\"withGlobalTauri\": true"),
        "src-tauri/tauri.conf.json must enable withGlobalTauri for the static HTML placeholder"
    );

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "tauri-api-missing",
        "requireTauriInvoke",
        "throw new Error(\"tauri-api-missing\")",
        "projectInitializationError(error)",
        "projectExplicitRefreshError(error)",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should expose missing Tauri API token `{token}`"
        );
    }
    assert!(
        !frontend.contains("if (!invoke) {\n          return;\n        }"),
        "missing Tauri invoke API must not silently return from initialization or refresh"
    );
}

#[test]
fn gui_manual_refresh_summary_is_readable_and_click_scoped() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9ag",
        "manual summary",
        "最近一次显式 Refresh",
        "summary projection 只发生在 explicit refresh click",
        "不新增 backend command",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9ag manual summary token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "manual-summary-status",
        "manual-summary-state",
        "manual-summary-history",
        "manual-summary-latest",
        "manual-summary-timing",
        "manual-summary-error",
        "projectManualRefreshSummary",
        "projectManualRefreshError",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should expose readable manual refresh summary token `{token}`"
        );
    }

    let success_call_pos = frontend
        .find("projectManualRefreshSummary(summary)")
        .expect("manual refresh success summary projection call should be present");
    let error_call_pos = frontend
        .find("projectManualRefreshError(error)")
        .expect("manual refresh error summary projection call should be present");
    let click_handler_pos = frontend
        .find("function handleExplicitRefresh")
        .expect("explicit refresh handler should be present");
    let catch_pos = frontend
        .find("catch (error)")
        .expect("explicit refresh catch path should be present");
    assert!(
        success_call_pos > click_handler_pos && success_call_pos < catch_pos,
        "manual success summary projection must stay in the explicit refresh success path"
    );
    assert!(
        error_call_pos > catch_pos,
        "manual error summary projection must stay in the explicit refresh catch path"
    );

    for token in [
        "summary.status.stateLabel",
        "summary.history.pageRecordCount",
        "summary.history.latestRecord",
        "summary.timing.requestDurationMs",
        "String(error?.message ?? error)",
    ] {
        assert!(
            frontend.contains(token),
            "manual refresh summary projection should use token `{token}`"
        );
    }

    for token in [
        "setInterval",
        "setTimeout",
        "subscribe_events",
        "client.send(&Command::Subscribe)",
        "install_service",
        "restart_service",
    ] {
        assert!(
            !frontend.contains(token),
            "Phase 9ag frontend must not subscribe, loop, or manage service token `{token}`"
        );
    }
}

#[test]
fn gui_frontend_first_screen_view_model_is_local_preflight_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9ah",
        "firstScreenViewModel",
        "initialization 和 explicit Refresh success/catch",
        "后续 Tauri event subscription 的前端落点预演",
        "不新增 backend command",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9ah frontend view model token `{token}`"
        );
    }

    let frontend = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "const firstScreenViewModel",
        "lastRefreshStatus",
        "connected",
        "stateLabel",
        "historyRecordCount",
        "latestPreview",
        "requestDurationMs",
        "errorMessage",
        "projectFirstScreenViewModel",
        "updateViewModelFromSummary",
        "updateViewModelFromError",
    ] {
        assert!(
            frontend.contains(token),
            "gui-dist/index.html should expose Phase 9ah view model token `{token}`"
        );
    }

    let summary_update_pos = frontend
        .find("updateViewModelFromSummary(summary)")
        .expect("summary should update frontend view model");
    let success_projection_pos = frontend
        .find("projectExplicitRefreshSummary(summary)")
        .expect("explicit success projection should exist");
    let error_update_pos = frontend
        .find("updateViewModelFromError(error)")
        .expect("error should update frontend view model");
    let catch_pos = frontend
        .find("catch (error)")
        .expect("explicit refresh catch path should exist");
    assert!(
        summary_update_pos > success_projection_pos,
        "summary view model update should stay in explicit refresh success path"
    );
    assert!(
        error_update_pos > catch_pos,
        "error view model update should stay in explicit refresh catch path"
    );

    for token in [
        "setInterval",
        "setTimeout",
        "subscribe_events",
        "client.send(&Command::Subscribe)",
        "install_service",
        "restart_service",
    ] {
        assert!(
            !frontend.contains(token),
            "Phase 9ah frontend must not subscribe, loop, or manage service token `{token}`"
        );
    }
}

#[test]
fn gui_backend_event_stream_start_is_tauri_owned_and_explicit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9ai",
        "gui_start_daemon_event_stream",
        "GUI-owned background task",
        "Command::Subscribe",
        "Tauri event",
        "不实现 reconnect supervisor",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9ai backend stream token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "GuiDaemonEventStreamStarted",
        "GuiDaemonEventPayload",
        "gui_start_daemon_event_stream",
        "tauri::Emitter",
        "tauri::async_runtime::spawn",
        "DaemonClient::connect_default()",
        ".send(&Command::Subscribe)",
        ".recv().await",
        "gui_daemon_event_payload(&event)",
        "const GUI_DAEMON_EVENT_NAME: &str = \"shuohua://daemon-event\"",
        ".emit(GUI_DAEMON_EVENT_NAME",
        "AtomicBool",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should expose Phase 9ai backend stream token `{token}`"
        );
    }
    for forbidden in [
        "install_service",
        "restart_service",
        "Command::StartRecording",
        "Command::StopRecording",
        "Command::CancelRecording",
        "next_reconnect_delay_ms",
        "setInterval",
        "setTimeout",
    ] {
        assert!(
            !tauri_lib.contains(forbidden),
            "Phase 9ai backend stream must not own service/recording/reconnect token `{forbidden}`"
        );
    }

    let capability =
        std::fs::read_to_string(root.join("src-tauri/capabilities/default.json")).unwrap();
    assert!(
        capability.contains("\"allow-gui-start-daemon-event-stream\""),
        "src-tauri/capabilities/default.json should allow gui_start_daemon_event_stream"
    );

    let permissions = std::fs::read_to_string(root.join("src-tauri/permissions/gui.toml"))
        .expect("GUI app command permissions should exist");
    assert!(
        permissions.contains("identifier = \"allow-gui-start-daemon-event-stream\"")
            && permissions.contains("commands.allow = [\"gui_start_daemon_event_stream\"]"),
        "src-tauri/permissions/gui.toml should define gui_start_daemon_event_stream permission"
    );

    for file in [
        "src/client_api.rs",
        "src/daemon/process.rs",
        "src/tui/mod.rs",
        "src/ipc/server.rs",
    ] {
        let body = std::fs::read_to_string(root.join(file)).unwrap();
        for token in ["tauri::Emitter", "shuohua://daemon-event"] {
            assert!(
                !body.contains(token),
                "{file} must not know GUI Tauri event bridge token `{token}`"
            );
        }
    }
}

#[test]
fn gui_frontend_daemon_event_listener_wiring_is_event_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();

    for token in [
        "Phase 9aj",
        "window.__TAURI__.event.listen",
        "shuohua://daemon-event",
        "gui_start_daemon_event_stream",
        "firstScreenViewModel",
        "不实现 reconnect supervisor",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9aj frontend listener token `{token}`"
        );
    }

    let html = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "window.__TAURI__?.event?.listen",
        "\"shuohua://daemon-event\"",
        "gui_start_daemon_event_stream",
        "handleDaemonEvent",
        "updateViewModelFromDaemonEvent",
        "projectDaemonEvent",
        "firstScreenViewModel.eventStreamStatus",
        "history stale",
    ] {
        assert!(
            html.contains(token),
            "gui-dist/index.html should wire Phase 9aj frontend event token `{token}`"
        );
    }
    for forbidden in [
        "Command::StartRecording",
        "Command::StopRecording",
        "Command::CancelRecording",
        "setInterval(",
        "setTimeout(",
        "install_service",
        "restart_service",
        "start_service",
    ] {
        assert!(
            !html.contains(forbidden),
            "Phase 9aj frontend listener must not own service/recording/reconnect token `{forbidden}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    assert!(
        tauri_lib.contains("gui_start_daemon_event_stream"),
        "Phase 9aj frontend should use the Phase 9ai backend stream command"
    );
}

#[test]
fn gui_backend_event_stream_forwards_recording_state_changes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();
    for token in [
        "Phase 9ak",
        "StateChanged",
        "daemonStatus",
        "不新增 IPC event",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9ak state forwarding token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "Event::StateChanged",
        "kind: \"daemonStatus\"",
        "state_label: Some(wire_state_label(*state))",
        "recording_id: recording_id.clone()",
        "if let Some(payload) = gui_daemon_event_payload(&event)",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should map subscribed state changes for GUI token `{token}`"
        );
    }

    for forbidden in [
        "Command::StartRecording",
        "Command::StopRecording",
        "Command::CancelRecording",
        "PROTO_VERSION = 3",
        "setInterval(",
        "setTimeout(",
        "gui_backend_event_from_daemon_event(&event).is_some()",
    ] {
        assert!(
            !tauri_lib.contains(forbidden),
            "Phase 9ak state forwarding must not add controls/protocol/reconnect token `{forbidden}`"
        );
    }
}

#[test]
fn gui_event_stream_projects_first_screen_data_without_refresh() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_doc = std::fs::read_to_string(root.join("docs/cross-platform/gui.md")).unwrap();
    for token in [
        "Phase 9al",
        "StatsChanged",
        "Partial",
        "Segment",
        "HistoryAppended",
        "不轮询",
    ] {
        assert!(
            gui_doc.contains(token),
            "docs/cross-platform/gui.md should record Phase 9al stream projection token `{token}`"
        );
    }

    let tauri_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap();
    for token in [
        "live_text: Option<String>",
        "duration_ms: Option<u64>",
        "word_count: Option<u32>",
        "history_record_count_delta: i64",
        "Event::StatsChanged",
        "Event::Partial",
        "Event::Segment",
        "Event::HistoryAppended",
    ] {
        assert!(
            tauri_lib.contains(token),
            "src-tauri/src/lib.rs should project Phase 9al stream data token `{token}`"
        );
    }

    let html = std::fs::read_to_string(root.join("gui-dist/index.html")).unwrap();
    for token in [
        "payload.durationMs",
        "payload.wordCount",
        "payload.liveText",
        "payload.historyRecordCountDelta",
        "payload.latestPreview",
        "stream update",
    ] {
        assert!(
            html.contains(token),
            "gui-dist/index.html should project Phase 9al stream data token `{token}`"
        );
    }

    for forbidden in [
        "Command::StartRecording",
        "Command::StopRecording",
        "Command::CancelRecording",
        "setInterval(",
        "setTimeout(",
        "gui_first_screen_summary_request_once()",
    ] {
        assert!(
            !html.contains(forbidden),
            "Phase 9al frontend stream projection must not add controls/polling token `{forbidden}`"
        );
    }
}

#[test]
fn windows_development_design_records_first_runtime_baseline() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let readme = std::fs::read_to_string(root.join("docs/cross-platform/README.md")).unwrap();
    assert!(
        readme.contains("[windows.md](windows.md)"),
        "cross-platform README should route Windows development decisions to windows.md"
    );

    let doc = std::fs::read_to_string(root.join("docs/cross-platform/windows.md")).unwrap();
    for token in [
        "Windows is the next primary cross-platform target",
        "normal per-user desktop application",
        "%APPDATA%\\Shuohua",
        "%LOCALAPPDATA%\\Shuohua",
        "\\\\.\\pipe\\shuohua-<logon-sid-or-session-scoped-hash>",
        "security descriptor/DACL",
        "Local\\shuohua-<logon-sid-or-session-scoped-hash>",
        "Task Scheduler logon task",
        "WH_KEYBOARD_LL",
        "SendInput",
        "native Win32",
        "Windows runtime validation must happen on Windows hardware or a Windows VM",
    ] {
        assert!(
            doc.contains(token),
            "docs/cross-platform/windows.md should record Windows runtime baseline token `{token}`"
        );
    }
}

#[test]
fn app_data_ownership_separates_product_data_from_package_private_data() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let readme = std::fs::read_to_string(root.join("docs/cross-platform/README.md")).unwrap();
    assert!(
        readme.contains("[app-data.md](app-data.md)"),
        "cross-platform README should route data ownership decisions to app-data.md"
    );

    let doc = std::fs::read_to_string(root.join("docs/cross-platform/app-data.md")).unwrap();
    for token in [
        "Product data root",
        "App-private data",
        "CLI, daemon, GUI, and packaged desktop app entries must share one product data model",
        "Packaged app data is not the default source of product data",
        "Config remains terminal-friendly by default",
        "~/.config/shuohua",
        "%APPDATA%\\Shuohua",
        "%LOCALAPPDATA%\\Shuohua",
        "$XDG_CONFIG_HOME/shuohua",
        "AppPaths",
        "app_private_dir",
    ] {
        assert!(
            doc.contains(token),
            "docs/cross-platform/app-data.md should record app data ownership token `{token}`"
        );
    }

    let windows_doc = std::fs::read_to_string(root.join("docs/cross-platform/windows.md")).unwrap();
    for token in [
        "[app-data.md](app-data.md)",
        "must not become a second product data truth source",
        "AppPaths",
    ] {
        assert!(
            windows_doc.contains(token),
            "docs/cross-platform/windows.md should reference app data ownership token `{token}`"
        );
    }
}

#[test]
fn windows_runtime_validation_checklist_stays_bottom_up() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let readme = std::fs::read_to_string(root.join("docs/cross-platform/README.md")).unwrap();
    assert!(
        readme.contains("[windows-runtime-validation.md](windows-runtime-validation.md)"),
        "cross-platform README should route first Windows smoke tests to windows-runtime-validation.md"
    );

    let doc =
        std::fs::read_to_string(root.join("docs/cross-platform/windows-runtime-validation.md"))
            .unwrap();
    for token in [
        "artifact identity",
        "Product Data Paths",
        "Daemon And IPC Smoke",
        "Single Instance Smoke",
        "Service Dry-Run Status",
        "Explorer Open/Reveal",
        "Do not use this checklist to claim audio, overlay, hotkey, clipboard, paste",
        ".\\shuo.exe doctor",
        ".\\shuo.exe daemon",
        ".\\shuo.exe service status",
        "$env:APPDATA\\Shuohua",
        "$env:LOCALAPPDATA\\Shuohua",
    ] {
        assert!(
            doc.contains(token),
            "docs/cross-platform/windows-runtime-validation.md should record bottom-up smoke token `{token}`"
        );
    }
}

fn rust_files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_rust_files(dir, &mut out);
    out
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn allows_direct_macos_backend(relative: &str) -> bool {
    relative.starts_with("src/platform/")
        || relative.starts_with("src/overlay/macos/")
        || relative.starts_with("src/cli/app/platform/")
        || matches!(
            relative,
            "src/hotkey/mod.rs" | "src/hotkey/provider_darwin.rs"
        )
}
