use std::path::{Path, PathBuf};

fn section_body<'a>(document: &'a str, header: &str) -> &'a str {
    let start = document
        .find(header)
        .unwrap_or_else(|| panic!("missing section {header}"));
    let after_header = &document[start + header.len()..];
    let end = after_header.find("\n[").unwrap_or(after_header.len());
    &after_header[..end]
}

#[test]
fn non_macos_checks_do_not_download_vad_runtime_at_build_time() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();

    let linux_section = section_body(
        &manifest,
        r#"[target.'cfg(target_os = "linux")'.dependencies]"#,
    );
    assert!(
        !linux_section.contains(r#"voice_activity_detector"#),
        "Linux checks must not compile voice_activity_detector because it enables ort-sys/download-binaries"
    );

    let windows_section = section_body(
        &manifest,
        r#"[target.'cfg(target_os = "windows")'.dependencies]"#,
    );
    assert!(
        windows_section.contains(r#"voice_activity_detector_windows"#)
            && windows_section.contains(r#"features = ["load-dynamic"]"#)
            && windows_section
                .contains(r#"ort = { version = "=2.0.0-rc.10", default-features = false"#),
        "Windows VAD should use Silero with explicit ORT dynamic loading"
    );

    let macos_section = section_body(
        &manifest,
        r#"[target.'cfg(target_os = "macos")'.dependencies]"#,
    );
    assert!(
        macos_section.contains(r#"voice_activity_detector = "0.2.1""#),
        "macOS VAD dependency should keep the current default runtime behavior"
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
        "basic_manual_smoke_only",
        "Validate Explorer open/reveal with UNC, missing paths, and non-interactive sessions",
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
fn audio_capture_diagnostics_live_behind_platform_facade() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let audio_capture_path = root.join("src/platform/audio_capture.rs");
    assert!(
        audio_capture_path.exists(),
        "Phase 10ah audio capture diagnostics facade should live at src/platform/audio_capture.rs"
    );

    let platform_mod = std::fs::read_to_string(root.join("src/platform/mod.rs")).unwrap();
    assert!(
        platform_mod.contains("pub(crate) mod audio_capture;"),
        "src/platform/mod.rs must expose the audio capture diagnostics facade"
    );

    let facade = std::fs::read_to_string(audio_capture_path).unwrap();
    for token in [
        "pub(crate) struct InputDeviceInfo",
        "pub(crate) struct InputDiagnostics",
        "pub(crate) fn probe_default_input(",
        "pub(crate) fn diagnose_input(",
        "cpal::default_host()",
        ".input_devices()",
        "default_input_device",
        "diagnostic_probe_only",
    ] {
        assert!(
            facade.contains(token),
            "platform audio capture facade should contain token `{token}`"
        );
    }

    let recorder = std::fs::read_to_string(root.join("src/voice/recorder.rs")).unwrap();
    assert!(
        recorder.contains("crate::platform::audio_capture::probe_default_input()"),
        "voice recorder default input probe should route through platform::audio_capture"
    );

    let doctor = std::fs::read_to_string(root.join("src/cli/doctor.rs")).unwrap();
    for token in [
        "crate::platform::audio_capture::diagnose_input()",
        "microphone.input.devices:",
        "microphone.input: backend=",
    ] {
        assert!(
            doctor.contains(token),
            "doctor should expose audio capture diagnostics token `{token}`"
        );
    }
}

#[test]
fn windows_audio_capture_capability_reports_input_stream_smoke() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let capability = std::fs::read_to_string(root.join("src/platform/capability.rs")).unwrap();
    let recorder = std::fs::read_to_string(root.join("src/voice/recorder.rs")).unwrap();

    for token in [
        "CapabilityId::AudioCapture",
        "cpal_wasapi",
        "common_targets_record_paste_smoke",
        "Validate sustained recording stability across Windows devices, privacy states, and remote desktop sessions",
    ] {
        assert!(
            capability.contains(token),
            "Windows audio.capture capability should report input stream smoke token `{token}`"
        );
    }
    assert!(
        recorder.contains("windows_input_stream_runtime_smoke_receives_pcm_chunks"),
        "Windows recorder should keep an ignored runtime smoke for default input stream callbacks"
    );

    let platform_doc =
        std::fs::read_to_string(root.join("docs/cross-platform/platform-capabilities.md")).unwrap();
    for token in [
        "Phase 10bg Windows Audio Input Stream Runtime Smoke",
        "Phase 10bv Windows Full Recording Audio Smoke",
        "common_targets_record_paste_smoke",
        "`audio.capture`：`partial`，backend `cpal_wasapi`，reason `common_targets_record_paste_smoke`",
        "SHUOHUA_WINDOWS_AUDIO_REQUIRE_SIGNAL",
        "Phase 10ah Windows Audio Capture Diagnostics",
    ] {
        assert!(
            platform_doc.contains(token),
            "platform capability docs should record audio stream smoke token `{token}`"
        );
    }
}

#[test]
fn windows_audio_convert_capability_reports_native_compact_backend() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let capability = std::fs::read_to_string(root.join("src/platform/capability.rs")).unwrap();
    let facade = std::fs::read_to_string(root.join("src/platform/audio_convert.rs")).unwrap();
    let voice_audio = std::fs::read_to_string(root.join("src/voice/audio.rs")).unwrap();
    let platform_doc =
        std::fs::read_to_string(root.join("docs/cross-platform/platform-capabilities.md")).unwrap();
    let windows_doc = std::fs::read_to_string(root.join("docs/cross-platform/windows.md")).unwrap();

    for token in [
        "CapabilityId::AudioConvert",
        "media_foundation_aac_flacenc",
        "full_recording_history_smoke",
        "Validate retained FLAC/M4A Explorer open/reveal and playback workflows",
    ] {
        assert!(
            capability.contains(token),
            "Windows audio.convert capability should report split retained-audio token `{token}`"
        );
    }

    for token in [
        "#[cfg(target_os = \"windows\")]",
        "MFCreateSinkWriterFromURL",
        "convert_wav_to_m4a_media_foundation",
        "convert_wav_to_flac_pure_rust",
        "flacenc::source::MemSource",
        "const FFMPEG: &str = \"ffmpeg\"",
        "ffmpeg_args",
        "retain audio on Windows",
        "media_foundation_runtime_smoke_creates_m4a_without_ffmpeg",
        "pure_rust_flac_runtime_smoke_creates_flac_without_ffmpeg",
        "native_compact_finish_creates_retained_audio_and_removes_temporary_wav",
        "native_lossless_finish_creates_retained_audio_and_removes_temporary_wav",
    ] {
        assert!(
            facade.contains(token) || voice_audio.contains(token),
            "Windows retained audio code should contain token `{token}`"
        );
    }

    for token in [
        "Phase 10bj/10bt Windows Native Retained Audio Backend",
        "Phase 10bv Windows Full Recording Audio Smoke",
        "`audio.convert`：`partial`，backend `media_foundation_aac_flacenc`，reason",
        "`full_recording_history_smoke`",
        "`compact` 使用 Windows Media Foundation Sink Writer",
        "`lossless` 使用 pure Rust `flacenc`",
        "不打包 ffmpeg",
        "media_foundation_runtime_smoke_creates_m4a_without_ffmpeg",
        "pure_rust_flac_runtime_smoke_creates_flac_without_ffmpeg",
    ] {
        assert!(
            platform_doc.contains(token),
            "platform capability docs should record Windows retained-audio conversion token `{token}`"
        );
    }

    for token in [
        "Retained audio conversion on Windows is split by retention mode",
        "Windows Media Foundation Sink Writer",
        "pure Rust `flacenc` encoder",
        "does not require `ffmpeg.exe`",
        "single-binary dependency policy",
    ] {
        assert!(
            windows_doc.contains(token),
            "Windows design doc should record retained audio conversion token `{token}`"
        );
    }
}

#[test]
fn windows_silero_vad_matches_macos_dependency_route() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let config = std::fs::read_to_string(root.join("src/config/main.rs")).unwrap();
    let schema = std::fs::read_to_string(root.join("src/config/schema.rs")).unwrap();
    let voice_mod = std::fs::read_to_string(root.join("src/voice/mod.rs")).unwrap();
    let engine = std::fs::read_to_string(root.join("src/voice/engine.rs")).unwrap();
    let lifecycle =
        std::fs::read_to_string(root.join("src/voice/engine_lifecycle_tests.rs")).unwrap();
    let silero = std::fs::read_to_string(root.join("src/voice/silero.rs")).unwrap();
    let windows_doc = std::fs::read_to_string(root.join("docs/cross-platform/windows.md")).unwrap();
    let voice_doc = std::fs::read_to_string(root.join("docs/modules/voice.md")).unwrap();

    for token in [
        r#"[target.'cfg(target_os = "windows")'.dependencies]"#,
        r#"voice_activity_detector_windows"#,
        r#"features = ["load-dynamic"]"#,
        r#"ort = { version = "=2.0.0-rc.10", default-features = false"#,
        "VoiceVadBackend",
        "Silero",
        "allowed_values([\"off\", \"silero\"])",
    ] {
        assert!(
            manifest.contains(token) || config.contains(token) || schema.contains(token),
            "VAD manifest/config/schema should expose Silero backend token `{token}`"
        );
    }

    for token in [
        "pub mod silero",
        "mod silero_runtime",
        "cfg(any(target_os = \"macos\", target_os = \"windows\"))",
        "voice_activity_detector::VoiceActivityDetector",
        "voice_activity_detector_windows::VoiceActivityDetector",
        "VoiceVadBackend::Silero => crate::voice::silero::is_available()",
        "VoiceVadBackend::Silero",
    ] {
        assert!(
            voice_mod.contains(token)
                || silero.contains(token)
                || engine.contains(token)
                || lifecycle.contains(token),
            "Windows Silero VAD implementation should contain token `{token}`"
        );
    }

    for token in [
        "same Silero VAD model/API route as macOS",
        "renamed vendored `voice_activity_detector` dependency",
        "explicit `ort/load-dynamic`",
        "official ONNX Runtime 1.22 DLL",
        "temporary RMS/energy fallback",
        "single-executable acceptance gate",
        "copy only",
        "without manually copying ORT DLLs",
    ] {
        assert!(
            windows_doc.contains(token) || voice_doc.contains(token),
            "docs should record Windows Silero VAD boundary token `{token}`"
        );
    }
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
        "CreateFileW",
        "NamedPipeClient::from_raw_handle",
        "FILE_READ_DATA | FILE_WRITE_DATA",
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
fn windows_ipc_docs_record_raw_client_access_mask() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let windows = std::fs::read_to_string(root.join("docs/cross-platform/windows.md")).unwrap();
    let ipc = std::fs::read_to_string(root.join("docs/cross-platform/ipc-service.md")).unwrap();

    for token in [
        "raw `CreateFileW`/overlapped client path",
        "`NamedPipeClient::from_raw_handle`",
        "`FILE_READ_DATA | FILE_WRITE_DATA`",
        "without generic write rights",
    ] {
        assert!(
            windows.contains(token),
            "Windows design doc should record raw client access-mask token `{token}`"
        );
    }

    for token in [
        "Phase 10af Windows raw Named Pipe client access mask",
        "raw `CreateFileW` + `NamedPipeClient::from_raw_handle`",
        "`FILE_READ_DATA | FILE_WRITE_DATA`",
        "`FILE_FLAG_OVERLAPPED`",
    ] {
        assert!(
            ipc.contains(token),
            "IPC service doc should record raw client access-mask token `{token}`"
        );
    }
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
        "same_user_elevation_smoke_only",
        "same-user smoke",
        "Validate cross-user isolation and longer Windows IPC soak",
        "Validate cross-user daemon isolation on Windows",
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
        "process_probe_runtime_smoke_tracks_child_exit",
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
        "child_process_exit_runtime_smoke",
        "Validate Windows process probing after daemon crash, abandoned mutex, PID reuse, and permission boundaries",
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
        "Command::Shutdown",
        "spawn_daemon_process",
        "fn wait_for_daemon_ready",
        "fn request_daemon_shutdown",
        "fn wait_for_pid_exit",
        "fn service_strategy()",
        "fn daemon_command(",
        "windows.user: dry-run",
        "start=explicit_process",
        "startup_registration=unsupported",
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
        "windows_user_session",
        "user_session_start_stop_only",
        "Validate Windows user service install/startup registration strategy",
    ] {
        assert!(
            capability.contains(token),
            "Windows capability snapshot should reflect service dry-run token `{token}`"
        );
    }
}

#[test]
fn windows_smart_fallback_probes_named_pipe_without_service_manager() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fallback = std::fs::read_to_string(root.join("src/daemon/fallback.rs")).unwrap();
    let ipc = std::fs::read_to_string(root.join("docs/cross-platform/ipc-service.md")).unwrap();

    for token in [
        "socket_status_from_windows_connect_result",
        "crate::ipc::transport::connect(path)",
        "ERROR_PIPE_BUSY",
        "Windows smart fallback Named Pipe probe",
    ] {
        assert!(
            fallback.contains(token) || ipc.contains(token),
            "Windows smart fallback should keep probe token `{token}`"
        );
    }

    let windows_probe = fallback
        .split("#[cfg(windows)]")
        .nth(1)
        .expect("missing Windows smart fallback cfg");
    for forbidden in ["Task Scheduler", "SCM", "registry", "schtasks"] {
        assert!(
            !windows_probe.contains(forbidden),
            "Windows smart fallback probe must not manage services via `{forbidden}`"
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
fn windows_hotkey_backend_uses_low_level_keyboard_hook() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let provider = std::fs::read_to_string(root.join("src/hotkey/provider_windows.rs")).unwrap();
    let platform = std::fs::read_to_string(root.join("src/platform/hotkey.rs")).unwrap();
    let hotkey_mod = std::fs::read_to_string(root.join("src/hotkey/mod.rs")).unwrap();

    for token in [
        "WH_KEYBOARD_LL",
        "SetWindowsHookExW",
        "CallNextHookEx",
        "KBDLLHOOKSTRUCT",
        "RawEvent",
        "Suppressor",
        "hook_runtime_smoke_receives_synthetic_f16_down_up",
        "hook_runtime_smoke_suppresses_synthetic_a_from_win32_edit",
        "GetWindowTextW",
    ] {
        assert!(
            provider.contains(token),
            "Windows hotkey provider should contain token `{token}`"
        );
    }

    assert!(
        platform.contains("crate::hotkey::provider_windows::run(writer, suppressor)"),
        "platform hotkey facade should dispatch to Windows provider"
    );
    assert!(
        hotkey_mod.contains("pub(crate) mod provider_windows;"),
        "hotkey module should expose the cfg-gated Windows provider"
    );
}

#[test]
fn windows_hotkey_capability_reports_hook_partial() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let capability = std::fs::read_to_string(root.join("src/platform/capability.rs")).unwrap();

    for token in [
        "CapabilityId::DesktopHotkey",
        "CapabilityId::DesktopHotkeySuppression",
        "wh_keyboard_ll",
        "common_targets_record_paste_smoke",
        "Validate hotkey press/release tracking across remote desktop, UAC/elevation, and Office/Teams-style apps",
        "Validate suppressed down/up pairing, stuck modifier prevention, remote desktop, UAC/elevation, and Office/Teams-style apps",
    ] {
        assert!(
            capability.contains(token),
            "Windows hotkey capability should report partial hook token `{token}`"
        );
    }
}

#[test]
fn windows_overlay_backend_uses_minimal_win32_window() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let backend = std::fs::read_to_string(root.join("src/overlay/windows.rs")).unwrap();

    for token in [
        "CreateWindowExW",
        "WS_EX_LAYERED",
        "WS_EX_TOPMOST",
        "WS_EX_TOOLWINDOW",
        "WS_EX_NOACTIVATE",
        "SetLayeredWindowAttributes",
        "HTTRANSPARENT",
        "DrawTextW",
        "OverlayModel",
        "runtime_smoke_creates_shows_hides_and_quits_window",
    ] {
        assert!(
            backend.contains(token),
            "Windows overlay backend should contain minimal Win32 token `{token}`"
        );
    }

    for forbidden in ["tauri", "WebView", "Skia", "wgpu"] {
        assert!(
            !backend.contains(forbidden),
            "Windows minimal overlay backend must not depend on `{forbidden}`"
        );
    }
}

#[test]
fn overlay_size_preferences_live_in_main_config_not_theme() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let main_config = std::fs::read_to_string(root.join("src/config/main.rs")).unwrap();
    let schema = std::fs::read_to_string(root.join("src/config/schema.rs")).unwrap();
    let template = std::fs::read_to_string(root.join("src/config/template/registry.rs")).unwrap();
    let theme = std::fs::read_to_string(root.join("src/config/theme.rs")).unwrap();
    let doc = std::fs::read_to_string(root.join("docs/cross-platform/config-theme.md")).unwrap();

    for token in [
        "pub width: f64",
        "pub text_scale: f64",
        "default_overlay_width",
        "default_overlay_text_scale",
    ] {
        assert!(
            main_config.contains(token),
            "main overlay config should own user preference token `{token}`"
        );
    }

    for token in ["overlay.width", "overlay.text_scale"] {
        assert!(
            schema.contains(token),
            "main config schema should declare `{token}`"
        );
        assert!(
            doc.contains(token),
            "config/theme policy should document `{token}`"
        );
    }

    for token in [
        "(\"width\", TemplateValue::Float(572.0))",
        "(\"text_scale\", TemplateValue::Float(1.0))",
    ] {
        assert!(
            template.contains(token),
            "starter config should emit overlay preference token `{token}`"
        );
    }

    for forbidden in ["pub width: Option", "pub text_scale: Option"] {
        assert!(
            !theme.contains(forbidden),
            "theme parser must not own overlay size preference `{forbidden}`"
        );
    }
}

#[test]
fn windows_overlay_capability_reports_minimal_partial_backend() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let backend = std::fs::read_to_string(root.join("src/overlay/windows.rs")).unwrap();
    let capabilities =
        std::fs::read_to_string(root.join("docs/cross-platform/platform-capabilities.md")).unwrap();

    for token in [
        "win32_direct2d_per_pixel",
        "direct2d_per_pixel_runtime_smoke",
        "translucent_shadow_no_blur",
        "win32_topmost_noactivate",
        "win32_httransparent",
        "win32_foreground_monitor_work_area",
        "foreground_monitor_screen_anchor_only",
        "runtime_smoke_only",
        "overlay.renderer",
        "overlay.material",
        "overlay.input_passthrough",
    ] {
        assert!(
            backend.contains(token) || capabilities.contains(token),
            "Windows overlay capability should report minimal partial token `{token}`"
        );
    }
}

#[test]
fn windows_overlay_records_dpi_and_font_baseline() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let backend = std::fs::read_to_string(root.join("src/overlay/windows.rs")).unwrap();
    let overlay_doc = std::fs::read_to_string(root.join("docs/cross-platform/overlay.md")).unwrap();
    let macos_view = std::fs::read_to_string(root.join("src/overlay/macos/view.rs")).unwrap();

    for token in [
        "GetDpiForWindow",
        "SPI_GETWORKAREA",
        "WindowMetrics",
        "work_area_layout",
        "create_ui_font",
        "Segoe UI",
    ] {
        assert!(
            backend.contains(token),
            "Windows overlay DPI/font baseline should contain token `{token}`"
        );
    }

    for token in ["NSFont::systemFontOfSize", "NSFont::boldSystemFontOfSize"] {
        assert!(
            macos_view.contains(token),
            "macOS overlay should continue using system fonts via `{token}`"
        );
    }

    for token in [
        "Do not bundle SF Pro",
        "does not hard-require JetBrains Mono",
        "DirectWrite/Direct2D text quality",
        "must not depend on SF Symbols",
    ] {
        assert!(
            overlay_doc.contains(token),
            "overlay font/DPI policy should document `{token}`"
        );
    }

    for token in ["draw_state_icon_gdi", "OverlayState"] {
        assert!(
            backend.contains(token),
            "Windows overlay should draw state icons without relying on SF Symbols via `{token}`"
        );
    }
}

#[test]
fn windows_overlay_records_rounded_gdi_baseline() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let backend = std::fs::read_to_string(root.join("src/overlay/windows.rs")).unwrap();
    let overlay_doc = std::fs::read_to_string(root.join("docs/cross-platform/overlay.md")).unwrap();
    let capability_doc =
        std::fs::read_to_string(root.join("docs/cross-platform/platform-capabilities.md")).unwrap();

    for token in [
        "CreateRoundRectRgn",
        "SetWindowRgn",
        "CLEARTYPE_QUALITY",
        "background_alpha",
        "corner_radius",
        "apply_rounded_window_region",
    ] {
        assert!(
            backend.contains(token),
            "Windows overlay rounded GDI baseline should contain token `{token}`"
        );
    }

    for token in [
        "Windows Phase 10aq Rounded GDI Baseline",
        "DirectWrite/Direct2D renderer foundation",
        "not the final text/material renderer",
    ] {
        assert!(
            overlay_doc.contains(token),
            "overlay rounded GDI policy should document `{token}`"
        );
    }

    for token in [
        "Phase 10aq Windows Overlay Rounded GDI Baseline",
        "does not change Windows overlay capability levels",
        "still uses GDI `DrawTextW`",
    ] {
        assert!(
            capability_doc.contains(token),
            "platform capability doc should document `{token}`"
        );
    }
}

#[test]
fn windows_overlay_records_direct2d_directwrite_foundation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let backend = std::fs::read_to_string(root.join("src/overlay/windows.rs")).unwrap();
    let direct2d = std::fs::read_to_string(root.join("src/overlay/windows/direct2d.rs")).unwrap();
    let overlay_doc = std::fs::read_to_string(root.join("docs/cross-platform/overlay.md")).unwrap();
    let capability_doc =
        std::fs::read_to_string(root.join("docs/cross-platform/platform-capabilities.md")).unwrap();

    for token in [
        "Win32_Graphics_DirectComposition",
        "Win32_Graphics_Direct2D",
        "Win32_Graphics_DirectWrite",
        "Win32_Graphics_Dxgi",
        "Win32_Graphics_Dxgi_Common",
    ] {
        assert!(
            manifest.contains(token),
            "Windows Direct2D/DirectWrite dependency should enable `{token}`"
        );
    }

    for token in ["mod direct2d;", "WS_EX_NOACTIVATE", "HTTRANSPARENT"] {
        assert!(
            backend.contains(token),
            "Windows overlay shell should route Direct2D without losing Win32 shell token `{token}`"
        );
    }

    for token in [
        "D2D1CreateFactory",
        "DWriteCreateFactory",
        "IDWriteTextFormat",
        "ID2D1RenderTarget",
        "DrawText",
        "Segoe UI Variable",
    ] {
        assert!(
            direct2d.contains(token),
            "Windows Direct2D renderer should contain token `{token}`"
        );
    }

    for forbidden in [
        "crate::daemon",
        "crate::ipc",
        "crate::hotkey",
        "crate::voice",
    ] {
        assert!(
            !direct2d.contains(forbidden),
            "Direct2D renderer must not depend on daemon/business module `{forbidden}`"
        );
    }

    for token in [
        "Windows Phase 10ar Direct2D/DirectWrite Foundation",
        "ID2D1DCRenderTarget",
        "Existing GDI drawing stays as a fallback",
    ] {
        assert!(
            overlay_doc.contains(token),
            "overlay Direct2D policy should document `{token}`"
        );
    }

    for token in [
        "Phase 10ar Windows Direct2D/DirectWrite Renderer Foundation",
        "does not upgrade capability levels yet",
        "GDI remains a fallback",
    ] {
        assert!(
            capability_doc.contains(token),
            "platform capability doc should document `{token}`"
        );
    }
}

#[test]
fn windows_overlay_composition_infrastructure_is_fallback_gated() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let windows_overlay = std::fs::read_to_string(root.join("src/overlay/windows.rs")).unwrap();
    let backend = std::fs::read_to_string(root.join("src/overlay/windows/backend.rs")).unwrap();
    let composition =
        std::fs::read_to_string(root.join("src/overlay/windows/composition.rs")).unwrap();
    let icons = std::fs::read_to_string(root.join("src/overlay/windows/icons.rs")).unwrap();
    let scene = std::fs::read_to_string(root.join("src/overlay/windows/scene.rs")).unwrap();
    let direct2d = std::fs::read_to_string(root.join("src/overlay/windows/direct2d.rs")).unwrap();
    let overlay_doc = std::fs::read_to_string(root.join("docs/cross-platform/overlay.md")).unwrap();

    for token in [
        "mod backend;",
        "mod composition;",
        "mod direct2d;",
        "mod icons;",
        "mod scene;",
        "WindowsRendererBackend",
    ] {
        assert!(
            windows_overlay.contains(token),
            "Windows overlay root should route through backend infrastructure token `{token}`"
        );
    }

    for token in [
        "RendererKind",
        "CompositionPlanned",
        "CompositionVisible",
        "Direct2dPerPixel",
        "GdiFallback",
        "SHUOHUA_WINDOWS_OVERLAY_COMPOSITION_PROBE",
        "SHUOHUA_WINDOWS_OVERLAY_COMPOSITION_VISIBLE",
        "probe_composition",
        "uses_per_pixel_surface",
        "disable_accelerated_backend",
        "composition_readiness",
        "update_reserved_scene",
    ] {
        assert!(
            backend.contains(token),
            "Windows backend selector should contain token `{token}`"
        );
    }

    for token in [
        "win32_composition_planned",
        "win32_direct2d_per_pixel",
        "CompositionReadiness",
        "CompositionGeometry",
        "Planned",
        "ProbeReady",
        "DCompositionCreateDevice",
        "CreateTargetForHwnd",
        "CreateVisual",
        "CompositionVisualTree",
        "AddVisual",
        "SetOffsetX2",
        "SetOffsetY2",
        "bind_animation_probes",
        "bind_icon_animation_for_state",
        "opacity_keyframe_animation",
        "SetRoot",
        "Commit",
        "IDCompositionSurface",
        "BeginDraw::<IDXGISurface>",
        "CreateDxgiSurfaceRenderTarget",
        "EndDraw",
        "draw_panel_probe",
        "draw_icon_probe",
        "draw_shadow_probe",
        "shadow_layer_alpha",
        "draw_scene_text",
        "DWriteCreateFactory",
        "CreateTextFormat",
        "D2D1_TEXT_ANTIALIAS_MODE_DEFAULT",
        "ensure_panel_surface",
        "ensure_icon_surface",
        "repeating_opacity_keyframe_animation",
        "AddRepeat",
        "IDCompositionRectangleClip",
        "CreateRectangleClip",
        "SetClip",
        "SetTopLeftRadiusX2",
        "SetBottomRightRadiusY2",
        "IDCompositionVisual3",
        "SetOpacity2",
        "SetOpacity",
        "DIRECT2D_SHADOW_OUTSET",
        "DirectComposition or Windows Composition visuals",
        "fallback: Direct2D per-pixel layered surface",
    ] {
        assert!(
            composition.contains(token),
            "Windows composition skeleton should record token `{token}`"
        );
    }

    for token in [
        "Segoe Fluent Icons",
        "Segoe MDL2 Assets",
        "StateIconPlan",
        "IconAnimation",
        "state_icon_plan",
        "Recording",
        "Pulse",
    ] {
        assert!(
            icons.contains(token),
            "Windows icon plan should use official icon font route token `{token}`"
        );
    }

    assert!(
        scene.contains("WindowsOverlayScene")
            && scene.contains("state_icon_plan")
            && direct2d.contains("icon_font_fallback_order")
            && direct2d.contains("WindowsOverlayScene")
            && direct2d.contains("CreateTextFormat"),
        "Direct2D fallback should render status icons through the shared Windows overlay scene and DirectWrite icon glyphs"
    );
    for forbidden in ["Ellipse(", "Polygon(", "DrawLine("] {
        assert!(
            !direct2d.contains(forbidden),
            "Direct2D status icon path should not keep hand-drawn primitive token `{forbidden}`"
        );
    }

    for token in [
        "Phase 10bg infrastructure status",
        "Segoe Fluent Icons",
        "Composition backend",
        "Direct2D/DirectWrite-on-composition-surface",
        "compositor-owned rounded clipping",
        "panel opacity binding",
        "shadow outset geometry",
        "composition shadow surface",
        "independent icon surface",
        "static animation binding",
        "looping state-driven opacity animation",
        "manual visible backend gate",
        "DirectWrite text",
        "Direct2D fallback now renders state icons through DirectWrite icon glyphs",
    ] {
        assert!(
            overlay_doc.contains(token),
            "overlay docs should record composition infrastructure token `{token}`"
        );
    }
}

#[test]
fn windows_overlay_records_per_pixel_layered_surface() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let backend = std::fs::read_to_string(root.join("src/overlay/windows.rs")).unwrap();
    let direct2d = std::fs::read_to_string(root.join("src/overlay/windows/direct2d.rs")).unwrap();
    let overlay_doc = std::fs::read_to_string(root.join("docs/cross-platform/overlay.md")).unwrap();
    let capability_doc =
        std::fs::read_to_string(root.join("docs/cross-platform/platform-capabilities.md")).unwrap();

    for token in [
        "CreateDIBSection",
        "CreateDCRenderTarget",
        "ID2D1DCRenderTarget",
        "BindDC",
        "UpdateLayeredWindow",
        "AC_SRC_ALPHA",
        "D2D1_ALPHA_MODE_PREMULTIPLIED",
        "SourceConstantAlpha: 255",
        "None,\n                Some(&size)",
    ] {
        assert!(
            direct2d.contains(token),
            "Windows per-pixel overlay surface should contain token `{token}`"
        );
    }

    assert!(
        backend.contains("!self.renderer.uses_per_pixel_surface()")
            && backend.contains("apply_window_alpha(self.hwnd, self.cfg.core.background_alpha)"),
        "Windows Direct2D path should avoid global SetLayeredWindowAttributes alpha"
    );

    for token in [
        "Windows Phase 10as Per-Pixel Layered Surface",
        "UpdateLayeredWindow",
        "solid 255-alpha text",
    ] {
        assert!(
            overlay_doc.contains(token),
            "overlay per-pixel surface policy should document `{token}`"
        );
    }

    for token in [
        "Phase 10as Windows Per-Pixel Layered Surface",
        "does not upgrade capability levels yet",
        "solid text alpha",
    ] {
        assert!(
            capability_doc.contains(token),
            "platform capability doc should document `{token}`"
        );
    }
}

#[test]
fn windows_overlay_records_dwm_backdrop_probe() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let backend = std::fs::read_to_string(root.join("src/overlay/windows.rs")).unwrap();
    let overlay_doc = std::fs::read_to_string(root.join("docs/cross-platform/overlay.md")).unwrap();
    let capability_doc =
        std::fs::read_to_string(root.join("docs/cross-platform/platform-capabilities.md")).unwrap();

    for forbidden in [
        "Win32_Graphics_Dwm",
        "DwmSetWindowAttribute",
        "DWMWA_SYSTEMBACKDROP_TYPE",
        "DWMSBT_TRANSIENTWINDOW",
    ] {
        assert!(
            !manifest.contains(forbidden) && !backend.contains(forbidden),
            "Windows DWM backdrop route should stay disabled in code: `{forbidden}`"
        );
    }

    for token in [
        "Windows Phase 10aw DWM Backdrop Probe Disabled",
        "unknown backdrop content outside the rounded overlay surface",
        "DirectComposition/Windows Composition",
    ] {
        assert!(
            overlay_doc.contains(token),
            "overlay DWM backdrop disable policy should document `{token}`"
        );
    }

    for token in [
        "Phase 10aw Windows DWM Backdrop Probe Disabled",
        "does not change Windows overlay capability levels",
        "overlay.material",
    ] {
        assert!(
            capability_doc.contains(token),
            "capability doc should preserve disabled DWM boundary `{token}`"
        );
    }
}

#[test]
fn windows_overlay_records_per_pixel_shadow_polish() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let backend = std::fs::read_to_string(root.join("src/overlay/windows.rs")).unwrap();
    let direct2d = std::fs::read_to_string(root.join("src/overlay/windows/direct2d.rs")).unwrap();
    let overlay_doc = std::fs::read_to_string(root.join("docs/cross-platform/overlay.md")).unwrap();
    let capability_doc =
        std::fs::read_to_string(root.join("docs/cross-platform/platform-capabilities.md")).unwrap();

    for token in [
        "DIRECT2D_SHADOW_OUTSET",
        "surface_outset",
        "clear_window_region",
        "!self.renderer.uses_per_pixel_surface()",
    ] {
        assert!(
            backend.contains(token),
            "Windows overlay shell should keep Direct2D shadow inset boundary token `{token}`"
        );
    }

    for token in [
        "AMBIENT_SHADOW_LAYERS",
        "AMBIENT_SHADOW_ALPHA",
        "KEY_SHADOW_LAYERS",
        "KEY_SHADOW_ALPHA",
        "shadow_layer_alpha",
        "draw_shadow",
        "inset_rect",
        "UpdateLayeredWindow",
    ] {
        assert!(
            direct2d.contains(token),
            "Windows Direct2D shadow polish should contain token `{token}`"
        );
    }

    for forbidden in ["DWMWA_SYSTEMBACKDROP_TYPE", "DwmSetWindowAttribute"] {
        assert!(
            !direct2d.contains(forbidden),
            "Direct2D shadow polish must not re-enable DWM backdrop token `{forbidden}`"
        );
    }

    for token in [
        "Windows Phase 10ay Direct2D Per-Pixel Shadow Polish",
        "renderer-owned shadow outset",
        "not blur or Liquid Glass parity",
    ] {
        assert!(
            overlay_doc.contains(token),
            "overlay shadow polish policy should document `{token}`"
        );
    }

    for token in [
        "Phase 10ay Windows Direct2D Per-Pixel Shadow Polish",
        "does not change Windows overlay capability levels",
        "overlay.material",
    ] {
        assert!(
            capability_doc.contains(token),
            "capability doc should preserve shadow polish boundary `{token}`"
        );
    }
}

#[test]
fn windows_overlay_records_foreground_monitor_work_area() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let backend = std::fs::read_to_string(root.join("src/overlay/windows.rs")).unwrap();
    let overlay_doc = std::fs::read_to_string(root.join("docs/cross-platform/overlay.md")).unwrap();
    let capability_doc =
        std::fs::read_to_string(root.join("docs/cross-platform/platform-capabilities.md")).unwrap();

    for token in [
        "GetForegroundWindow",
        "MonitorFromWindow",
        "GetMonitorInfoW",
        "MONITOR_DEFAULTTONEAREST",
        "anchor_window",
        "monitor_work_area_rect",
        "primary_work_area_rect",
    ] {
        assert!(
            backend.contains(token),
            "Windows overlay should select foreground monitor work area via `{token}`"
        );
    }

    for token in [
        "Windows Phase 10az Foreground Monitor Work Area",
        "foreground window's nearest monitor",
        "SPI_GETWORKAREA",
    ] {
        assert!(
            overlay_doc.contains(token),
            "overlay foreground-monitor policy should document `{token}`"
        );
    }

    for token in [
        "Phase 10az Windows Foreground Monitor Work Area",
        "does not change Windows overlay capability levels",
        "overlay.window_anchor",
    ] {
        assert!(
            capability_doc.contains(token),
            "capability doc should preserve monitor work-area boundary `{token}`"
        );
    }
}

#[test]
fn windows_active_app_identity_backend_lives_behind_desktop_facade() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let windows_app_context = root.join("src/platform/windows/app_context.rs");
    assert!(
        windows_app_context.exists(),
        "Windows active app backend should live at src/platform/windows/app_context.rs"
    );

    let platform_mod = std::fs::read_to_string(root.join("src/platform/mod.rs")).unwrap();
    for token in ["#[cfg(target_os = \"windows\")]", "pub(crate) mod windows;"] {
        assert!(
            platform_mod.contains(token),
            "src/platform/mod.rs should cfg-gate Windows backend token `{token}`"
        );
    }

    let desktop = std::fs::read_to_string(root.join("src/platform/desktop.rs")).unwrap();
    for token in [
        "#[cfg(target_os = \"windows\")]",
        "crate::platform::windows::app_context::frontmost_app()",
        "AppContext::default()",
    ] {
        assert!(
            desktop.contains(token),
            "desktop facade should route Windows active app lookup through token `{token}`"
        );
    }

    let backend = std::fs::read_to_string(windows_app_context).unwrap();
    for token in [
        "GetForegroundWindow",
        "GetWindowThreadProcessId",
        "OpenProcess",
        "QueryFullProcessImageNameW",
        "GetApplicationUserModelId",
        "PROCESS_QUERY_LIMITED_INFORMATION",
        "windows_app_user_model_id",
        "windows_exe_name",
        "app_name_from_exe_name",
        "foreground_self_window_runtime_smoke",
        "ProfileRouteCfg",
        "current_from_app_context",
        "CreateWindowExW",
        "SetForegroundWindow",
    ] {
        assert!(
            backend.contains(token),
            "Windows active app backend should contain token `{token}`"
        );
    }

    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for token in [
        "Win32_UI_WindowsAndMessaging",
        "Win32_Storage_Packaging_Appx",
    ] {
        assert!(
            manifest.contains(token),
            "Cargo.toml should enable Windows active app lookup feature `{token}`"
        );
    }
}

#[test]
fn windows_active_app_capability_reports_process_identity_partial() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let capability = std::fs::read_to_string(root.join("src/platform/capability.rs")).unwrap();
    let platform_doc =
        std::fs::read_to_string(root.join("docs/cross-platform/platform-capabilities.md")).unwrap();
    let windows_doc = std::fs::read_to_string(root.join("docs/cross-platform/windows.md")).unwrap();

    for token in [
        "CapabilityId::DesktopActiveApp",
        "foreground_window_process_identity",
        "foreground_profile_route_self_window_smoke",
        "Validate foreground app route matching across packaged Windows apps and broader real app targets",
    ] {
        assert!(
            capability.contains(token),
            "Windows desktop.active_app capability should report process identity token `{token}`"
        );
    }

    for token in [
        "Phase 10bk Windows AppUserModelID Active App Identity",
        "`partial`，backend `foreground_window_process_identity`，reason",
        "foreground_profile_route_self_window_smoke",
        "`GetApplicationUserModelId`",
        "AUMID 为空是正常降级",
    ] {
        assert!(
            platform_doc.contains(token),
            "platform capability docs should record Windows AUMID token `{token}`"
        );
    }

    for token in [
        "Phase 10bk extends the same process handle with best-effort `GetApplicationUserModelId`",
        "foreground_profile_route_self_window_smoke",
        "`windows_app_user_model_id`",
        "AUMID is optional",
    ] {
        assert!(
            windows_doc.contains(token),
            "Windows design doc should record active app identity token `{token}`"
        );
    }
}

#[test]
fn windows_clipboard_write_backend_uses_win32_unicode_clipboard() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let backend = std::fs::read_to_string(root.join("src/platform/windows/clipboard.rs")).unwrap();

    for token in [
        "OpenClipboard",
        "EmptyClipboard",
        "SetClipboardData",
        "CF_UNICODETEXT_FORMAT",
        "GlobalAlloc",
        "GlobalLock",
        "GlobalUnlock",
        "GlobalFree",
        "clipboard_utf16",
    ] {
        assert!(
            backend.contains(token),
            "Windows clipboard backend should contain token `{token}`"
        );
    }

    let platform_clipboard =
        std::fs::read_to_string(root.join("src/platform/clipboard.rs")).unwrap();
    assert!(
        platform_clipboard.contains("crate::platform::windows::clipboard::write_string(text)"),
        "shared clipboard facade should dispatch to the Windows backend"
    );

    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for feature in ["Win32_System_DataExchange", "Win32_System_Memory"] {
        assert!(
            manifest.contains(feature),
            "Cargo.toml should enable `{feature}` for Windows clipboard writes"
        );
    }
}

#[test]
fn windows_clipboard_capability_reports_write_only_partial() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let capability = std::fs::read_to_string(root.join("src/platform/capability.rs")).unwrap();

    for token in [
        "CapabilityId::DesktopClipboard",
        "win32_clipboard_unicode",
        "common_targets_record_paste_smoke",
        "Validate full record-to-clipboard behavior across UAC/elevation and Office/Teams-style apps",
    ] {
        assert!(
            capability.contains(token),
            "Windows desktop.clipboard capability should report write-only partial token `{token}`"
        );
    }

    let dispatch = std::fs::read_to_string(root.join("src/voice/dispatch.rs")).unwrap();
    for token in [
        "windows_dispatch_clipboard_runtime_smoke",
        "SHUOHUA_WINDOWS_DISPATCH_SMOKE_TEXT",
        "GetClipboardData",
    ] {
        assert!(
            dispatch.contains(token),
            "voice dispatch should keep Windows clipboard runtime smoke token `{token}`"
        );
    }
}

#[test]
fn windows_paste_backend_uses_sendinput_ctrl_v() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let backend = std::fs::read_to_string(root.join("src/platform/windows/autotype.rs")).unwrap();

    for token in [
        "SendInput",
        "INPUT_KEYBOARD",
        "KEYBDINPUT",
        "KEYEVENTF_KEYUP",
        "VK_CONTROL",
        "VK_V",
        "ctrl_v_inputs",
        "paste_runtime_smoke",
        "paste_into_win32_edit_runtime_smoke",
        "CreateWindowExW",
        "GetWindowTextW",
        "SHUOHUA_WINDOWS_PASTE_TARGET_SMOKE_TEXT",
    ] {
        assert!(
            backend.contains(token),
            "Windows paste backend should contain token `{token}`"
        );
    }

    let platform_autotype = std::fs::read_to_string(root.join("src/platform/autotype.rs")).unwrap();
    assert!(
        platform_autotype.contains("crate::platform::windows::autotype::paste()"),
        "shared autotype facade should dispatch to the Windows backend"
    );

    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(
        manifest.contains("Win32_UI_Input_KeyboardAndMouse"),
        "Cargo.toml should enable KeyboardAndMouse APIs for Windows paste injection"
    );
}

#[test]
fn windows_text_injection_capability_reports_sendinput_partial() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let capability = std::fs::read_to_string(root.join("src/platform/capability.rs")).unwrap();

    for token in [
        "CapabilityId::DesktopTextInjection",
        "sendinput_ctrl_v",
        "common_targets_record_paste_smoke",
        "Validate Ctrl+V injection across remote desktop, UAC/elevation, and Office/Teams-style apps",
    ] {
        assert!(
            capability.contains(token),
            "Windows desktop.text_injection capability should report SendInput partial token `{token}`"
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
fn tui_status_consumes_capability_snapshots_without_ipc_side_effects() {
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
        "\\\\.\\pipe\\shuohua-<sha256(user-sid + logon-sid) prefix>",
        "security descriptor/DACL",
        "Local\\shuohua-daemon-<sha256(user-sid + logon-sid) prefix>",
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
        "CLI, daemon, TUI, and packaged desktop app entries must share one product data model",
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
fn app_paths_facade_owns_config_and_state_roots() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let paths = std::fs::read_to_string(root.join("src/paths.rs")).unwrap();
    for token in [
        "pub struct AppPaths",
        "pub fn discover() -> Self",
        "pub fn config_root(&self)",
        "pub fn state_root(&self)",
        "pub fn main_config(&self)",
        "pub fn profile_dir(&self)",
        "pub fn asr_provider(&self",
        "pub fn post_dir(&self)",
        "pub fn cache(&self)",
        "StateDirs::from_app_paths",
        "FOLDERID_RoamingAppData",
        "FOLDERID_LocalAppData",
        "SHGetKnownFolderPath",
        "CoTaskMemFree",
    ] {
        assert!(
            paths.contains(token),
            "src/paths.rs should centralize product path token `{token}`"
        );
    }

    let config_paths = std::fs::read_to_string(root.join("src/config/paths.rs")).unwrap();
    assert!(
        config_paths.contains("crate::paths::AppPaths::discover()"),
        "config paths should resolve through AppPaths"
    );
    for forbidden in ["std::env::var", "XDG_CONFIG_HOME", "join(\".config\")"] {
        assert!(
            !config_paths.contains(forbidden),
            "config paths should not read `{forbidden}` directly after AppPaths facade"
        );
    }

    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for token in ["Win32_System_Com", "Win32_UI_Shell"] {
        assert!(
            manifest.contains(token),
            "Cargo.toml should enable Windows known-folder dependency feature `{token}`"
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
        "Named Pipe Busy Smoke",
        "Elevation Boundary Smoke",
        "Cross-User Smoke",
        "Service Dry-Run Status",
        "Explorer Open/Reveal",
        "Do not use this checklist to claim audio, overlay, hotkey, clipboard, paste",
        ".\\shuo.exe doctor",
        ".\\shuo.exe --daemon",
        ".\\shuo.exe service status",
        "$env:APPDATA\\Shuohua",
        "$env:LOCALAPPDATA\\Shuohua",
        ".\\scripts\\windows-ipc-smoke.ps1 -StopExisting",
        "verifies repeated start is idempotent",
        "This is a deferred manual gate",
        "access/scope/security",
    ] {
        assert!(
            doc.contains(token),
            "docs/cross-platform/windows-runtime-validation.md should record bottom-up smoke token `{token}`"
        );
    }
}

#[test]
fn windows_local_dev_replaces_ci_artifact_loop() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workflow = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    for forbidden in [
        "windows-artifact",
        "windows-latest",
        "upload-artifact",
        "shuo-windows-debug",
    ] {
        assert!(
            !workflow.contains(forbidden),
            ".github/workflows/ci.yml should not use slow Windows CI artifact token `{forbidden}`"
        );
    }

    let local_dev =
        std::fs::read_to_string(root.join("docs/cross-platform/windows-local-dev.md")).unwrap();
    for token in [
        "Windows Local Development",
        "git pull --ff-only",
        "rustup default stable-x86_64-pc-windows-msvc",
        "cargo test --target x86_64-pc-windows-msvc",
        "cargo build --target x86_64-pc-windows-msvc",
        "windows-runtime-validation.md",
    ] {
        assert!(
            local_dev.contains(token),
            "docs/cross-platform/windows-local-dev.md should document local dev token `{token}`"
        );
    }

    let doc = std::fs::read_to_string(root.join("docs/cross-platform/windows.md")).unwrap();
    for token in [
        "Windows: maintain a local development checkout",
        "Do not rely on GitHub Actions for Windows build artifacts",
        "windows-local-dev.md",
    ] {
        assert!(
            doc.contains(token),
            "docs/cross-platform/windows.md should document Windows local dev token `{token}`"
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
