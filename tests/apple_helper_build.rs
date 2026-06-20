#[path = "../build_support/apple_helper.rs"]
mod apple_helper;

#[test]
fn swift_target_triple_maps_release_arches_to_macos15() {
    assert_eq!(
        apple_helper::swift_target_triple("aarch64", apple_helper::DEFAULT_MACOS_DEPLOYMENT_TARGET)
            .unwrap(),
        "arm64-apple-macosx15.0"
    );
    assert_eq!(
        apple_helper::swift_target_triple("x86_64", "15.0").unwrap(),
        "x86_64-apple-macosx15.0"
    );
}

#[test]
fn swift_target_triple_rejects_invalid_inputs() {
    assert!(apple_helper::swift_target_triple("armv7", "15.0").is_err());
    assert!(apple_helper::swift_target_triple("aarch64", "").is_err());
}
