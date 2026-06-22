mod llm_wizard;
mod registry;
mod render;

#[allow(unused_imports)]
pub use llm_wizard::{
    create_llm_component, llm_draft_from_template, llm_templates, render_llm_component,
    LlmComponentDraft,
};
#[allow(unused_imports)]
pub use registry::{
    registry, theme_preset_body, theme_presets, Template, TemplateKind, TemplateValue, ThemePreset,
};
#[allow(unused_imports)]
pub use render::{render, render_theme_preset, render_with_lang};

#[cfg(test)]
mod tests {
    use crate::config::spec::ValueKind;

    use super::*;

    fn render_by_id(id: &str) -> Option<String> {
        registry()
            .iter()
            .find(|template| template.id == id)
            .map(render)
    }

    #[test]
    fn registry_renders_expected_llm_template() {
        let body = render_by_id("post/llm/deepseek").unwrap();

        assert!(body.contains("type = \"llm\""));
        assert!(body.contains("format = \"openai\""));
        assert!(body.contains("name = \"deepseek\""));
        assert!(body.contains("api_key = \"\""));
        assert!(body.contains("[extra_body]"));
        assert!(body.contains("thinking = { type = \"disabled\" }"));
    }

    #[test]
    fn rendered_registry_templates_are_valid_toml() {
        for template in registry() {
            assert!(
                render(template).starts_with("# "),
                "{} missing header comment",
                template.id
            );
            toml::from_str::<toml::Value>(&render(template))
                .unwrap_or_else(|e| panic!("{} renders invalid TOML: {e}", template.id));
        }
    }

    #[test]
    fn config_template_documents_record_audio_modes_in_one_comment() {
        let body = render_with_lang(
            registry()
                .iter()
                .find(|template| template.id == "config")
                .unwrap(),
            crate::i18n::Lang::ZhCN,
        );

        assert!(body.contains(
            "# off=不保存；lossless=FLAC 无损；compact=AAC 32 kbps，约比 FLAC 再省 75% 空间\nrecord_audio = \"off\""
        ));
    }

    #[test]
    fn rendered_theme_presets_are_valid_toml() {
        for preset in theme_presets() {
            let body = render_theme_preset(preset);
            assert!(
                body.starts_with("# "),
                "{} missing header comment",
                preset.id
            );
            let theme: crate::config::theme::ThemeFile = toml::from_str(&body)
                .unwrap_or_else(|e| panic!("{} renders invalid TOML: {e}", preset.id));
            crate::config::theme::validate_theme_file(&theme)
                .unwrap_or_else(|e| panic!("{} renders invalid theme: {e}", preset.id));
        }
    }

    #[test]
    fn config_template_uses_accessible_default_hotkey() {
        let body = render_by_id("config").unwrap();

        assert!(body.contains("# Hotkey that toggles recording."));
        assert!(body.contains("trigger = \"right_option:double\""));
        let config = crate::config::main::parse(&body).unwrap();
        crate::hotkey::Bindings::parse(&config.hotkey.trigger, &config.hotkey.cancel).unwrap();
    }

    #[test]
    fn rendered_templates_can_use_zh_cn_field_comments() {
        let template = registry()
            .iter()
            .find(|template| template.id == "config")
            .unwrap();
        let body = render_with_lang(template, crate::i18n::Lang::ZhCN);

        assert!(body.contains("# 用于开始或结束录音的快捷键。"));
        toml::from_str::<toml::Value>(&body).unwrap();
    }

    #[test]
    fn runtime_parsers_accept_core_templates_after_required_secrets_are_filled() {
        crate::config::main::parse(&render_by_id("config").unwrap()).unwrap();

        let profile: crate::config::profile::Profile =
            toml::from_str(&render_by_id("profile/default").unwrap()).unwrap();
        assert_eq!(profile.name, "default");

        let mut llm = render_by_id("post/llm/openai").unwrap();
        llm = llm.replace("api_key = \"\"", "api_key = \"sk-test\"");
        llm = llm.replace("model = \"gpt-4.1-mini\"", "model = \"test-model\"");
        let value: toml::Value = toml::from_str(&llm).unwrap();
        assert_eq!(value["type"].as_str(), Some("llm"));
    }

    #[test]
    fn required_secret_fields_render_empty_placeholder() {
        for template in registry()
            .iter()
            .filter(|template| template.kind == TemplateKind::PostLlm)
        {
            let spec = template.spec();
            let api_key = spec.field_for_path("api_key").unwrap();
            assert!(api_key.required_without_default());
            assert!(api_key.is_secret());
            assert!(render(template).contains("api_key = \"\""));
            assert_eq!(api_key.kind(), ValueKind::String);
        }
    }

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("shuohua-template-test-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn llm_draft_uses_template_defaults_and_renders_overrides() {
        let mut draft = llm_draft_from_template("post/llm/deepseek").unwrap();
        draft.file_id = "cleaner".to_string();
        draft.provider_name = "team-deepseek".to_string();
        draft.model = "deepseek-chat-v3".to_string();

        let body = render_llm_component(&draft).unwrap();

        assert!(body.contains("format = \"openai\""));
        assert!(body.contains("name = \"team-deepseek\""));
        assert!(body.contains("model = \"deepseek-chat-v3\""));
        assert!(body.contains("[extra_body]"));
        toml::from_str::<toml::Value>(&body).unwrap();
    }

    #[test]
    fn create_llm_component_refuses_duplicate_file_and_provider_name() {
        let dir = temp_dir();
        let post = dir.join("post");
        let llm = post.join("llm");
        std::fs::create_dir_all(&llm).unwrap();
        std::fs::write(
            llm.join("existing.toml"),
            "type = \"llm\"\nname = \"taken\"\napi_key = \"sk-test\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
        )
        .unwrap();

        let mut draft = llm_draft_from_template("post/llm/openai").unwrap();
        draft.file_id = "existing".to_string();
        draft.provider_name = "fresh".to_string();
        draft.model = "gpt-test".to_string();
        assert!(create_llm_component(&post, &draft)
            .unwrap_err()
            .to_string()
            .contains("already exists"));

        draft.file_id = "new_component".to_string();
        draft.provider_name = "taken".to_string();
        assert!(create_llm_component(&post, &draft)
            .unwrap_err()
            .to_string()
            .contains("provider name"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn create_llm_component_writes_valid_file() {
        let dir = temp_dir();
        let post = dir.join("post");
        let mut draft = llm_draft_from_template("post/llm/anthropic").unwrap();
        draft.file_id = "claude_cleanup".to_string();
        draft.provider_name = "anthropic-team".to_string();
        draft.model = "claude-test".to_string();

        let path = create_llm_component(&post, &draft).unwrap();

        assert_eq!(path, post.join("llm/claude_cleanup.toml"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("format = \"anthropic\""));
        assert!(body.contains("name = \"anthropic-team\""));
        toml::from_str::<toml::Value>(&body).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
