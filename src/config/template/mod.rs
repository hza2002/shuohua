mod asr_wizard;
mod llm_wizard;
mod registry;
mod render;

#[allow(unused_imports)]
pub use asr_wizard::{
    asr_apple_from_template, asr_doubao_from_template, asr_templates, asr_tencent_from_template,
    create_asr_apple, create_asr_doubao, create_asr_tencent, render_asr_apple, render_asr_doubao,
    render_asr_tencent, AsrAppleDraft, AsrDoubaoDraft, AsrTencentDraft,
};
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
        let body = render_by_id("post/deepseek").unwrap();

        assert!(body.contains("type = \"llm\""));
        assert!(body.contains("format = \"openai\""));
        assert!(body.contains("name = \"deepseek\""));
        assert!(body.contains("api_key = \"\""));
        assert!(body.contains("[extra_body]"));
        assert!(body.contains("thinking = { type = \"disabled\" }"));
    }

    #[test]
    fn doubao_template_uses_auto_local_vad() {
        let body = render_by_id("asr/doubao").unwrap();
        let value: toml::Value = toml::from_str(&body).unwrap();

        assert_eq!(value["local_vad"].as_str(), Some("auto"));
    }

    #[test]
    fn asr_templates_do_not_include_speechmatics() {
        assert!(render_by_id("asr/speechmatics").is_none());
        assert!(!asr_templates().any(|template| template.id == "asr/speechmatics"));
    }

    #[test]
    fn asr_templates_use_timeout_fields() {
        for id in ["asr/apple", "asr/doubao", "asr/tencent"] {
            let body = render_by_id(id).unwrap();
            assert!(body.contains("open_timeout_ms"));
            assert!(body.contains("finalize_timeout_ms"));
            assert!(!body.contains("session_open_timeout_ms"));
            assert!(!body.contains("session_finalize_timeout_ms"));
        }
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
        assert!(body.contains("resume = \"shift+right_option:double\""));
        let config = crate::config::main::parse(&body).unwrap();
        crate::hotkey::Bindings::parse(
            &config.hotkey.trigger,
            &config.hotkey.cancel,
            &config.hotkey.resume,
        )
        .unwrap();
    }

    #[test]
    fn config_template_exports_production_defaults_and_routes() {
        let body = render_by_id("config").unwrap();
        let config = crate::config::main::parse(&body).unwrap();

        assert_eq!(
            config.voice.preprocess.backend,
            crate::config::VoicePreprocessBackend::WebRtc
        );
        assert_eq!(
            config.voice.vad.backend,
            crate::config::VoiceVadBackend::Silero
        );
        assert_eq!(config.post.timeout_ms, 30_000);
        assert_eq!(config.profile.default, "default");
        assert!(body.contains("threshold = 0.5"));
        assert!(body.contains("pause_silence_ms = 1500"));
        assert!(body.contains("pre_roll_ms = 300"));
        assert!(body.contains("max_overlap_ms = 200"));
        assert!(body.contains("min_start_voiced_frames = 2"));
        assert!(body.contains("[overlay]"));
        assert!(body.contains("[dev]"));
        assert!(config.profile.routes.contains_key("chat"));
        assert!(config.profile.routes.contains_key("agent"));
        assert!(config
            .profile
            .routes
            .get("chat")
            .unwrap()
            .contains(&"com.openai.chat".to_string()));
        assert!(config
            .profile
            .routes
            .get("agent")
            .unwrap()
            .contains(&"com.microsoft.VSCode".to_string()));
    }

    #[test]
    fn config_template_documents_preprocess_options_without_unimplemented_backends() {
        let body = render_with_lang(
            registry()
                .iter()
                .find(|template| template.id == "config")
                .unwrap(),
            crate::i18n::Lang::ZhCN,
        );

        assert!(body.contains("[voice.preprocess]"));
        assert!(body.contains("backend = \"webrtc\""));
        assert!(body.contains("apple 用 macOS 原生语音处理"));
        assert!(body.contains("webrtc 用 WebRTC Audio Processing"));
        assert!(body.contains("off 用原始采集"));
        assert!(!body.contains("预留"));
        assert!(!body.contains("reserved"));
    }

    #[test]
    fn config_template_routes_are_disjoint_and_conservative() {
        let body = render_by_id("config").unwrap();
        let config = crate::config::main::parse(&body).unwrap();
        let chat = config.profile.routes.get("chat").unwrap();
        let agent = config.profile.routes.get("agent").unwrap();

        for bundle_id in chat {
            assert!(
                !agent.contains(bundle_id),
                "{bundle_id} appears in both chat and agent routes"
            );
        }
        for broad_app in [
            "com.google.Chrome",
            "com.apple.Safari",
            "org.mozilla.firefox",
            "com.apple.mail",
            "com.microsoft.Outlook",
            "us.zoom.xos",
        ] {
            assert!(!chat.contains(&broad_app.to_string()));
            assert!(!agent.contains(&broad_app.to_string()));
        }
    }

    #[test]
    fn registry_exports_chat_and_agent_profiles_with_llm_prompts() {
        for (id, name) in [("profile/chat", "chat"), ("profile/agent", "agent")] {
            let body = render_by_id(id).unwrap();
            let profile = crate::config::profile::parse(&body).unwrap();

            assert_eq!(profile.name.as_deref(), Some(name));
            assert_eq!(profile.asr.instance, "doubao");
            assert!(profile.asr.hotwords.is_empty());
            assert_eq!(
                profile.post.chain,
                vec!["zh_filter".to_string(), "deepseek".to_string()]
            );
            let llm = profile
                .post
                .overrides
                .get("deepseek")
                .and_then(toml::Value::as_table)
                .unwrap();
            assert!(llm
                .get("system_prompt")
                .and_then(toml::Value::as_str)
                .is_some_and(|prompt| prompt.contains("ASR 文本整理器")));
            assert!(llm
                .get("prompt")
                .and_then(toml::Value::as_str)
                .is_some_and(|prompt| prompt.contains("{{app_name}}")));
        }
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
        assert_eq!(profile.name.as_deref(), Some("default"));

        let mut llm = render_by_id("post/openai").unwrap();
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
        let mut draft = llm_draft_from_template("post/deepseek").unwrap();
        draft.file_id = "cleaner".to_string();
        draft.provider_name = "team-deepseek".to_string();
        draft.model = "deepseek-chat-v3".to_string();
        draft.system_prompt = "system text".to_string();
        draft.prompt = "{{text}}\nclean it".to_string();

        let body = render_llm_component(&draft).unwrap();

        assert!(body.contains("format = \"openai\""));
        assert!(body.contains("name = \"team-deepseek\""));
        assert!(body.contains("model = \"deepseek-chat-v3\""));
        assert!(body.contains("system_prompt = \"system text\""));
        assert!(body.contains("clean it"));
        assert!(body.contains("[extra_body]"));
        toml::from_str::<toml::Value>(&body).unwrap();
    }

    #[test]
    fn create_llm_component_refuses_duplicate_file_but_allows_duplicate_name() {
        let dir = temp_dir();
        let post = dir.join("post");
        std::fs::create_dir_all(&post).unwrap();
        std::fs::write(
            post.join("existing.toml"),
            "type = \"llm\"\nname = \"taken\"\nbase_url = \"https://a\"\napi_key = \"sk-test\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
        )
        .unwrap();

        let mut draft = llm_draft_from_template("post/openai").unwrap();
        draft.file_id = "existing".to_string();
        draft.provider_name = "fresh".to_string();
        draft.model = "gpt-test".to_string();
        assert!(create_llm_component(&post, &draft)
            .unwrap_err()
            .to_string()
            .contains("already exists"));

        // Duplicate name with a different file_id is now allowed: name is display-only.
        draft.file_id = "new_component".to_string();
        draft.provider_name = "taken".to_string();
        let path = create_llm_component(&post, &draft).unwrap();
        assert_eq!(path, post.join("new_component.toml"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn create_llm_component_rejects_invalid_file_id() {
        let dir = temp_dir();
        let post = dir.join("post");
        let mut draft = llm_draft_from_template("post/openai").unwrap();
        draft.file_id = "BadName".to_string();
        draft.provider_name = "fresh".to_string();
        draft.model = "gpt-test".to_string();

        let error = create_llm_component(&post, &draft).unwrap_err();
        let error = format!("{error:#}");

        assert!(error.contains("invalid file name"), "{error}");
        assert!(error.contains("lowercase letter first"), "{error}");
        assert!(!post.join("BadName.toml").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn create_llm_component_writes_valid_file() {
        let dir = temp_dir();
        let post = dir.join("post");
        let mut draft = llm_draft_from_template("post/anthropic").unwrap();
        draft.file_id = "claude_cleanup".to_string();
        draft.provider_name = "anthropic-team".to_string();
        draft.model = "claude-test".to_string();

        let path = create_llm_component(&post, &draft).unwrap();

        assert_eq!(path, post.join("claude_cleanup.toml"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("format = \"anthropic\""));
        assert!(body.contains("name = \"anthropic-team\""));
        toml::from_str::<toml::Value>(&body).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn asr_doubao_draft_uses_template_defaults_and_renders_overrides() {
        let mut draft = asr_doubao_from_template("asr/doubao").unwrap();
        draft.app_key = "app-test".to_string();
        draft.access_key = "access-test".to_string();
        draft.language = "zh-CN".to_string();
        draft.enable_punc = false;
        draft.stream_mode = 2;

        let body = render_asr_doubao(&draft).unwrap();

        assert!(body.contains("app_key = \"app-test\""));
        assert!(body.contains("access_key = \"access-test\""));
        assert!(body.contains("resource_id = \"volc.bigasr.sauc.duration\""));
        assert!(body.contains("language = \"zh-CN\""));
        assert!(body.contains("enable_punc = false"));
        assert!(body.contains("stream_mode = 2"));
        toml::from_str::<crate::config::asr::doubao::DoubaoConfig>(&body).unwrap();
    }

    #[test]
    fn asr_tencent_draft_defaults_to_free_chinese_engine_and_allows_other_engines() {
        let mut draft = asr_tencent_from_template("asr/tencent").unwrap();
        assert_eq!(draft.engine_model_type, "16k_zh");
        assert!(!draft.need_vad);
        assert_eq!(draft.filter_modal, 1);
        assert_eq!(draft.convert_num_mode, 1);
        assert_eq!(draft.sentence_strategy, 0);

        draft.app_id = "1250000000".to_string();
        draft.secret_id = "sid-test".to_string();
        draft.secret_key = "key-test".to_string();
        draft.engine_model_type = "16k_multi_lang".to_string();

        let body = render_asr_tencent(&draft).unwrap();

        assert!(body.contains("type = \"tencent\""));
        assert!(body.contains("engine_model_type = \"16k_multi_lang\""));
        assert!(body.contains("need_vad = false"));
        assert!(body.contains("filter_modal = 1"));
        assert!(!body.contains("filter_empty_result"));
        assert!(!body.contains("word_info"));
        assert!(!body.contains("emotion_recognition"));
        let parsed: crate::config::asr::tencent::TencentConfig = toml::from_str(&body).unwrap();
        assert_eq!(parsed.engine_model_type, "16k_multi_lang");
        assert_eq!(parsed.vad_silence_time, 1000);
        assert_eq!(parsed.max_speak_time, 60_000);
    }

    #[test]
    fn create_asr_tencent_writes_fixed_provider_file() {
        let dir = temp_dir();
        let asr = dir.join("asr");
        let mut draft = asr_tencent_from_template("asr/tencent").unwrap();
        draft.app_id = "1250000000".to_string();
        draft.secret_id = "sid-test".to_string();
        draft.secret_key = "key-test".to_string();

        let path = create_asr_tencent(&asr, &draft).unwrap();

        assert_eq!(path, asr.join("tencent.toml"));
        toml::from_str::<crate::config::asr::tencent::TencentConfig>(
            &std::fs::read_to_string(&path).unwrap(),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn create_asr_doubao_writes_fixed_provider_file_and_refuses_duplicate() {
        let dir = temp_dir();
        let asr = dir.join("asr");
        let mut draft = asr_doubao_from_template("asr/doubao").unwrap();
        draft.app_key = "app-test".to_string();
        draft.access_key = "access-test".to_string();

        let path = create_asr_doubao(&asr, &draft).unwrap();

        assert_eq!(path, asr.join("doubao.toml"));
        toml::from_str::<crate::config::asr::doubao::DoubaoConfig>(
            &std::fs::read_to_string(&path).unwrap(),
        )
        .unwrap();
        assert!(create_asr_doubao(&asr, &draft)
            .unwrap_err()
            .to_string()
            .contains("already exists"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn create_asr_apple_writes_fixed_provider_file() {
        let dir = temp_dir();
        let asr = dir.join("asr");
        let draft = asr_apple_from_template("asr/apple").unwrap();

        let path = create_asr_apple(&asr, &draft).unwrap();

        assert_eq!(path, asr.join("apple.toml"));
        let body = std::fs::read_to_string(&path).unwrap();
        let parsed: crate::config::asr::apple::AppleConfig = toml::from_str(&body).unwrap();
        assert_eq!(parsed.language.as_deref(), Some("zh-CN"));
        assert!(parsed.install_assets);
        let _ = std::fs::remove_dir_all(dir);
    }
}
