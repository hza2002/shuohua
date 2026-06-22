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
        "src/platform/permissions.rs",
        "src/platform/daemon.rs",
        "src/post/app_context.rs",
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
            "src/hotkey/mod.rs" | "src/hotkey/provider_darwin.rs" | "src/post/app_context.rs"
        )
}
