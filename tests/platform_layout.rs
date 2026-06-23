use std::path::{Path, PathBuf};

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
fn overlay_renderer_capabilities_are_consumed_by_doctor_only() {
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
            "src/overlay/mod.rs" | "src/overlay/renderer.rs" | "src/cli/doctor.rs"
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
        "overlay renderer capability snapshot should only feed doctor until GUI/TUI consumption is designed:\n{}",
        offenders.join("\n")
    );
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
fn gui_library_boundary_does_not_create_tauri_workspace_or_runtime() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    for file in [
        "src-tauri/tauri.conf.json",
        "src-tauri/Cargo.toml",
        "gui/src-tauri/tauri.conf.json",
        "gui/src-tauri/Cargo.toml",
    ] {
        assert!(
            !root.join(file).exists(),
            "Tauri workspace/app file {file} should not exist before library boundary review"
        );
    }

    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(
        cargo.contains("[[bin]]") && cargo.contains("name = \"shuo\""),
        "package should still expose the existing shuo binary target"
    );

    for token in ["tauri", "wry", "webview", "WebView", "tao"] {
        assert!(
            !cargo.contains(token),
            "GUI library split must happen before adding GUI runtime dependency token `{token}`"
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
