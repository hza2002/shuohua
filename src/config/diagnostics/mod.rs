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
    use std::path::PathBuf;

    use super::*;
    use crate::config::spec::Severity;

    fn temp_config_home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "shuohua-diagnostics-test-{}",
            ulid::Ulid::generate()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn local_diagnostics_scans_unreferenced_profile_asr_and_post_files() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("asr")).unwrap();
        fs::create_dir_all(root.join("post")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(root.join("profile/broken.toml"), "[asr\n").unwrap();
        fs::write(
            root.join("asr/apple.toml"),
            "type = \"apple\"\nlocal_vad = \"on\"\nunknown = 1\n",
        )
        .unwrap();
        fs::write(
            root.join("post/broken.toml"),
            "type = \"llm\"\napi_key = \"\"\n",
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.files_checked >= 4);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.source.ends_with("profile/broken.toml")));
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.source.ends_with("asr/apple.toml")));
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.source.ends_with("post/broken.toml")));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn local_diagnostics_reports_profile_reference_errors() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("post")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("profile/default.toml"),
            r#"
name = "default"
[asr]
instance = "doubao"

[post]
chain = ["missing", "deepseek"]
"#,
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.diagnostics.iter().any(|d| {
            d.scope == DiagnosticScope::Profile
                && d.source.ends_with("profile/default.toml")
                && d.path == "asr.instance"
                && d.message.contains("asr/doubao.toml")
        }));
        assert!(report.diagnostics.iter().any(|d| {
            d.scope == DiagnosticScope::Profile
                && d.source.ends_with("profile/default.toml")
                && d.path == "post.chain"
                && d.message.contains("post/missing.toml")
        }));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn local_diagnostics_rejects_dangling_and_non_llm_overrides() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("post")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("post/zh_filter.toml"),
            "type = \"rule\"\npatterns = []\n",
        )
        .unwrap();
        fs::write(
            root.join("profile/default.toml"),
            r#"
[asr]
instance = "apple"

[post]
chain = ["zh_filter"]

[post.overrides.stray]
model = "x"

[post.overrides.zh_filter]
model = "x"
"#,
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(
            report.diagnostics.iter().any(|d| {
                d.scope == DiagnosticScope::Profile
                    && d.source.ends_with("profile/default.toml")
                    && d.path == "post.overrides"
                    && d.severity == Severity::Error
                    && d.message.contains("stray")
                    && d.message.contains("not in the chain")
            }),
            "dangling override should error: {:?}",
            report.diagnostics
        );
        assert!(
            report.diagnostics.iter().any(|d| {
                d.scope == DiagnosticScope::Profile
                    && d.source.ends_with("profile/default.toml")
                    && d.path == "post.overrides"
                    && d.severity == Severity::Error
                    && d.message.contains("zh_filter")
                    && d.message.contains("non-llm")
            }),
            "rule-target override should error: {:?}",
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
    fn local_diagnostics_rejects_non_identifier_config_file_names() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("post")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("post/Zh Filter.toml"),
            "type = \"rule\"\npatterns = []\n",
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.diagnostics.iter().any(|d| {
            d.scope == DiagnosticScope::PostProcessor
                && d.source.ends_with("post/Zh Filter.toml")
                && d.severity == Severity::Error
                && d.message.contains("invalid file name")
        }));
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
        fs::create_dir_all(root.join("post")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("post/draft.toml"),
            "type = \"llm\"\nname = \"draft\"\napi_key = \"\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.diagnostics.iter().any(|d| {
            d.source.ends_with("post/draft.toml")
                && d.path == "api_key"
                && d.severity == Severity::Warning
        }));
        assert!(!report.diagnostics.iter().any(|d| {
            d.source.ends_with("post/draft.toml")
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
        fs::create_dir_all(root.join("post")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("profile/default.toml"),
            "name = \"default\"\n[asr]\ninstance = \"apple\"\n[post]\nchain = [\"draft\"]\n",
        )
        .unwrap();
        fs::write(
            root.join("post/draft.toml"),
            "type = \"llm\"\nname = \"draft\"\napi_key = \"\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.diagnostics.iter().any(|d| {
            d.source.ends_with("post/draft.toml")
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
        fs::create_dir_all(root.join("post")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("profile/default.toml"),
            r#"
name = "default"

[asr]
instance = "apple"

[post]
chain = ["deepseek"]

[post.overrides.deepseek]
modle = "typo"
"#,
        )
        .unwrap();
        fs::write(
            root.join("post/deepseek.toml"),
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
        fs::create_dir_all(root.join("asr")).unwrap();
        fs::create_dir_all(root.join("post")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("asr/apple.toml"),
            "type = \"apple\"\nlocal_vad = \"on\"\n",
        )
        .unwrap();
        fs::write(
            root.join("profile/default.toml"),
            r#"
name = "default"
[asr]
instance = "apple"
language = "zh-CN"
hotwords = ["Rust"]

[post]
chain = ["deepseek"]

[post.overrides.deepseek]
model = "deepseek-chat"
"#,
        )
        .unwrap();
        fs::write(
            root.join("post/deepseek.toml"),
            r#"
type = "llm"
format = "openai"
name = "deepseek"
base_url = "https://api.deepseek.com"
api_key = "sk-test"
model = "deepseek-chat"
prompt = "{{text}}"
"#,
        )
        .unwrap();

        let plan = runtime_check_plan_from_config_home(&home).unwrap();

        assert_eq!(plan.profiles.len(), 1);
        assert_eq!(plan.asr_targets()[0].instance.id, "apple");
        assert_eq!(plan.asr_targets()[0].hotwords, vec!["Rust"]);
        assert_eq!(plan.llm_targets()[0].id, "deepseek");
        assert_eq!(
            plan.llm_targets()[0]
                .overrides
                .get("model")
                .and_then(toml::Value::as_str),
            Some("deepseek-chat")
        );
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn asr_diagnostics_rejects_asr_file_missing_type_field() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("asr")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        // Valid identifier stem but missing `type` field — should report an error.
        fs::write(root.join("asr/whisper.toml"), "api_key = \"x\"\n").unwrap();

        let report = run_local_from_config_home(&home);

        assert!(
            report.diagnostics.iter().any(|d| {
                d.scope == DiagnosticScope::AsrProvider
                    && d.source.ends_with("asr/whisper.toml")
                    && d.severity == Severity::Error
            }),
            "expected an error for asr/whisper.toml, got: {:?}",
            report.diagnostics
        );
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn asr_diagnostics_accepts_custom_typed_instance_and_profile_referencing_it() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("asr")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("asr/team.toml"),
            "type = \"doubao\"\napp_key = \"k\"\naccess_key = \"s\"\n",
        )
        .unwrap();
        fs::write(
            root.join("profile/default.toml"),
            "name = \"default\"\n[asr]\ninstance = \"team\"\n",
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(
            !report.diagnostics.iter().any(|d| {
                d.source.ends_with("asr/team.toml") || d.source.ends_with("profile/default.toml")
            }),
            "expected no errors, got: {:?}",
            report.diagnostics
        );
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn profile_referencing_missing_custom_asr_instance_reports_error() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("asr")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("profile/default.toml"),
            "name = \"default\"\n[asr]\ninstance = \"team\"\n",
        )
        .unwrap();
        // No asr/team.toml — should report missing file.

        let report = run_local_from_config_home(&home);

        assert!(
            report.diagnostics.iter().any(|d| {
                d.scope == DiagnosticScope::Profile
                    && d.source.ends_with("profile/default.toml")
                    && d.path == "asr.instance"
                    && d.message.contains("asr/team.toml")
            }),
            "expected missing-instance error mentioning asr/team.toml, got: {:?}",
            report.diagnostics
        );
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn asr_instance_file_missing_type_field_reports_error() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("asr")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        // File exists but has no `type` field.
        fs::write(root.join("asr/team.toml"), "app_key = \"k\"\n").unwrap();

        let report = run_local_from_config_home(&home);

        assert!(
            report.diagnostics.iter().any(|d| {
                d.scope == DiagnosticScope::AsrProvider
                    && d.source.ends_with("asr/team.toml")
                    && d.severity == Severity::Error
                    && d.message.contains("type")
            }),
            "expected missing-type error for asr/team.toml, got: {:?}",
            report.diagnostics
        );
        let _ = fs::remove_dir_all(home);
    }
}
