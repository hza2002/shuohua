use std::path::Path;

#[test]
fn shared_macos_adapters_live_under_platform_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in [
        "src/platform/mod.rs",
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
