mod report;
mod runtime_plan;
mod scan;

#[allow(unused_imports)]
pub use report::{ConfigDiagnostic, ConfigDiagnosticReport, DiagnosticScope};
#[allow(unused_imports)]
pub use runtime_plan::{
    runtime_check_plan, runtime_check_plan_from_config_home, AsrRuntimeTarget, LlmRuntimeTarget,
    RuntimeCheckPlan, RuntimeProfileCheck,
};
#[allow(unused_imports)]
pub use scan::{run_local, run_local_from_config_home};

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::config::spec::Severity;

    fn temp_config_home() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("shuohua-diagnostics-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn source_ends_with(source: &Path, relative: &[&str]) -> bool {
        source.ends_with(relative.iter().collect::<PathBuf>())
    }

    #[test]
    fn local_diagnostics_scans_unreferenced_profile_asr_and_post_files() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("asr")).unwrap();
        fs::create_dir_all(root.join("post/llm")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(root.join("profile/broken.toml"), "[asr\n").unwrap();
        fs::write(
            root.join("asr/apple.toml"),
            "idle_pause = true\nunknown = 1\n",
        )
        .unwrap();
        fs::write(
            root.join("post/llm/broken.toml"),
            "type = \"llm\"\napi_key = \"\"\n",
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.files_checked >= 4);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| source_ends_with(&d.source, &["profile", "broken.toml"])));
        assert!(report
            .diagnostics
            .iter()
            .any(|d| source_ends_with(&d.source, &["asr", "apple.toml"])));
        assert!(report
            .diagnostics
            .iter()
            .any(|d| source_ends_with(&d.source, &["post", "llm", "broken.toml"])));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn local_diagnostics_reports_profile_reference_errors() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("post/rule")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("profile/default.toml"),
            r#"
name = "default"
[asr]
provider = "doubao"

[post]
chain = ["rule:missing", "llm:missing", "bad-item", "other:name"]
"#,
        )
        .unwrap();

        let report = run_local_from_config_home(&home);
        let missing_asr = root.join("asr").join("doubao.toml");
        let missing_rule = root.join("post").join("rule").join("missing.toml");

        assert!(
            report.diagnostics.iter().any(|d| {
                d.scope == DiagnosticScope::Profile
                    && source_ends_with(&d.source, &["profile", "default.toml"])
                    && d.path == "asr.provider"
                    && d.message.contains(&missing_asr.display().to_string())
            }),
            "{:?}",
            report.diagnostics
        );
        assert!(
            report.diagnostics.iter().any(|d| {
                d.scope == DiagnosticScope::Profile
                    && source_ends_with(&d.source, &["profile", "default.toml"])
                    && d.path == "post.chain"
                    && d.message.contains(&missing_rule.display().to_string())
            }),
            "{:?}",
            report.diagnostics
        );
        assert!(
            report.diagnostics.iter().any(|d| {
                d.scope == DiagnosticScope::Profile
                    && source_ends_with(&d.source, &["profile", "default.toml"])
                    && d.path == "post.chain"
                    && d.message.contains("post chain item")
            }),
            "{:?}",
            report.diagnostics
        );
        assert!(
            report.diagnostics.iter().any(|d| {
                d.scope == DiagnosticScope::Profile
                    && source_ends_with(&d.source, &["profile", "default.toml"])
                    && d.path == "post.chain"
                    && d.message.contains("unknown post component kind")
            }),
            "{:?}",
            report.diagnostics
        );
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn local_diagnostics_reports_only_user_config_sources() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report
            .diagnostics
            .iter()
            .all(|d| d.source.starts_with(&root)));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn theme_diagnostics_accepts_macos_overlay_fields() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("theme")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("theme/custom.toml"),
            r#"
[overlay.macos]
glass_variant = 11
glass_style = "clear"
subdued = 0
background_blur_radius = 3
"#,
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(!report.diagnostics.iter().any(|d| {
            d.source.ends_with("theme/custom.toml")
                && d.severity == Severity::Warning
                && d.path.starts_with("overlay.macos")
                && d.message.contains("unknown")
        }));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn theme_diagnostics_rejects_legacy_overlay_fields() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("theme")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("theme/legacy.toml"),
            r#"
[overlay.glass]
variant = 11
style = "clear"
subdued = 0

[overlay.surface]
background_blur_radius = 3
"#,
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.diagnostics.iter().any(|d| {
            d.source.ends_with("theme/legacy.toml")
                && d.severity == Severity::Warning
                && d.path == "overlay.glass"
                && d.message.contains("unknown")
        }));
        assert!(report.diagnostics.iter().any(|d| {
            d.source.ends_with("theme/legacy.toml")
                && d.severity == Severity::Warning
                && d.path == "overlay.surface.background_blur_radius"
                && d.message.contains("unknown")
        }));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn unreferenced_llm_draft_empty_api_key_is_warning() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("post/llm")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("post/llm/draft.toml"),
            "type = \"llm\"\nname = \"draft\"\napi_key = \"\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.diagnostics.iter().any(|d| {
            d.source.ends_with("post/llm/draft.toml")
                && d.path == "api_key"
                && d.severity == Severity::Warning
        }));
        assert!(!report.diagnostics.iter().any(|d| {
            d.source.ends_with("post/llm/draft.toml")
                && d.path == "api_key"
                && d.severity == Severity::Error
        }));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn referenced_llm_empty_api_key_is_error() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("post/llm")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("profile/default.toml"),
            "name = \"default\"\n[asr]\nprovider = \"apple\"\n[post]\nchain = [\"llm:draft\"]\n",
        )
        .unwrap();
        fs::write(
            root.join("post/llm/draft.toml"),
            "type = \"llm\"\nname = \"draft\"\napi_key = \"\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.diagnostics.iter().any(|d| {
            d.source.ends_with("post/llm/draft.toml")
                && d.path == "api_key"
                && d.severity == Severity::Error
        }));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn profile_llm_override_typos_are_reported_locally() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("post/llm")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("profile/default.toml"),
            r#"
name = "default"

[asr]
provider = "apple"

[post]
chain = ["llm:deepseek"]

[post.llm.deepseek]
modle = "typo"
"#,
        )
        .unwrap();
        fs::write(
            root.join("post/llm/deepseek.toml"),
            r#"
type = "llm"
name = "deepseek"
api_key = "sk-test"
model = "deepseek-chat"
prompt = "{{text}}"
"#,
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.diagnostics.iter().any(|d| {
            d.source.ends_with("profile/default.toml")
                && d.path == "post"
                && d.message.contains("modle")
                && d.message.contains("unknown field")
        }));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn runtime_check_plan_lists_configured_asr_and_referenced_llm_targets() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("post/llm")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("profile/default.toml"),
            r#"
name = "default"
[asr]
provider = "apple"
language = "zh-CN"
hotwords = ["Rust"]

[post]
chain = ["llm:deepseek"]

[post.llm.deepseek]
model = "deepseek-chat"
"#,
        )
        .unwrap();
        fs::write(
            root.join("post/llm/deepseek.toml"),
            r#"
type = "llm"
format = "openai"
name = "deepseek"
api_key = "sk-test"
model = "deepseek-chat"
prompt = "{{text}}"
"#,
        )
        .unwrap();

        let plan = runtime_check_plan_from_config_home(&home).unwrap();

        assert_eq!(plan.profiles.len(), 1);
        assert_eq!(plan.asr_targets()[0].provider, "apple");
        assert_eq!(plan.asr_targets()[0].hotwords, vec!["Rust"]);
        assert_eq!(plan.llm_targets()[0].id, "llm:deepseek");
        assert_eq!(
            plan.llm_targets()[0]
                .overrides
                .get("model")
                .and_then(toml::Value::as_str),
            Some("deepseek-chat")
        );
        let _ = fs::remove_dir_all(home);
    }
}
