pub const DEFAULT_MACOS_DEPLOYMENT_TARGET: &str = "15.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacosHelperTarget {
    pub output_name: &'static str,
    pub source_path: &'static str,
}

pub fn macos_helper_targets() -> &'static [MacosHelperTarget] {
    &[
        MacosHelperTarget {
            output_name: "apple_helper",
            source_path: "src/asr/providers/apple_helper.swift",
        },
        MacosHelperTarget {
            output_name: "apple_capture_helper",
            source_path: "src/voice/apple_capture_helper.swift",
        },
    ]
}

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
    fn helper_targets_include_asr_and_capture_helpers() {
        let targets = macos_helper_targets();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].output_name, "apple_helper");
        assert_eq!(
            targets[0].source_path,
            "src/asr/providers/apple_helper.swift"
        );
        assert_eq!(targets[1].output_name, "apple_capture_helper");
        assert_eq!(
            targets[1].source_path,
            "src/voice/apple_capture_helper.swift"
        );
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
