use std::path::Path;

#[test]
fn cli_runtime_is_owned_only_by_dispatcher() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cli_dir = root.join("src/cli");
    let mut owners = Vec::new();

    for entry in std::fs::read_dir(&cli_dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let body = std::fs::read_to_string(&path).unwrap();
        if body.contains("tokio::runtime") {
            owners.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    owners.sort();

    assert_eq!(
        owners,
        ["mod.rs"],
        "CLI subcommands must use the runtime owned by cli::run_command"
    );
}
