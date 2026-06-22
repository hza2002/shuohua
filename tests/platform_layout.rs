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
            "src/hotkey/mod.rs"
                | "src/hotkey/provider_darwin.rs"
                | "src/cli/service/mod.rs"
                | "src/cli/service/macos.rs"
                | "src/post/app_context.rs"
        )
}
