pub const DEFAULT_MACOS_DEPLOYMENT_TARGET: &str = "15.0";

pub fn should_build_macos_helper(target_os: &str) -> bool {
    target_os == "macos"
}

pub fn swift_target_triple(target_arch: &str, deployment_target: &str) -> Result<String, String> {
    let arch = match target_arch {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => return Err(format!("unsupported macOS helper target arch {other:?}")),
    };
    if deployment_target.trim().is_empty() {
        return Err("MACOSX_DEPLOYMENT_TARGET must not be empty".to_string());
    }
    Ok(format!("{arch}-apple-macosx{deployment_target}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_macos_targets_skip_swift_helper() {
        assert!(!should_build_macos_helper("linux"));
        assert!(should_build_macos_helper("macos"));
    }

    #[test]
    fn swift_target_triple_maps_cargo_arches() {
        assert_eq!(
            swift_target_triple("aarch64", "15.0").unwrap(),
            "arm64-apple-macosx15.0"
        );
        assert_eq!(
            swift_target_triple("x86_64", "15.0").unwrap(),
            "x86_64-apple-macosx15.0"
        );
    }

    #[test]
    fn swift_target_triple_rejects_unknown_arch_and_empty_target() {
        assert!(swift_target_triple("armv7", "15.0").is_err());
        assert!(swift_target_triple("aarch64", "").is_err());
    }
}
