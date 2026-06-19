use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::schema::{self, SchemaId};
use crate::config::spec::ConfigSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    Config,
    Asr,
    Profile,
    PostRule,
    PostLlm,
}

#[derive(Debug, Clone, Copy)]
pub struct Template {
    pub id: &'static str,
    pub kind: TemplateKind,
    pub path: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    schema: SchemaId,
    values: &'static [(&'static str, TemplateValue)],
}

impl Template {
    pub fn spec(&self) -> ConfigSpec {
        schema::spec_for(self.schema)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TemplateValue {
    String(&'static str),
    Integer(i64),
    Bool(bool),
    StringArray(&'static [&'static str]),
    InlineTable(&'static [(&'static str, TemplateValue)]),
    Table(&'static [(&'static str, TemplateValue)]),
}

pub fn registry() -> &'static [Template] {
    TEMPLATES
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn render(template: &Template) -> String {
    render_with_lang(template, crate::i18n::Lang::EnUS)
}

pub fn render_with_lang(template: &Template, lang: crate::i18n::Lang) -> String {
    let mut body = String::new();
    body.push_str(&format!("# {}\n", template.title));
    body.push_str(&format!("# {}\n\n", template.description));
    body.push_str(&render_from_spec(
        &template.spec(),
        template.values,
        Some(lang),
    ));
    body
}

#[derive(Debug, Clone, Copy)]
pub struct ThemePreset {
    pub id: &'static str,
    pub path: &'static str,
    body: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/embedded_themes.rs"));

pub fn theme_presets() -> &'static [ThemePreset] {
    THEME_PRESETS
}

pub fn theme_preset_body(name: &str) -> Option<&'static str> {
    let id = if name == crate::config::theme::DEFAULT_THEME_NAME
        || name == crate::config::theme::LEGACY_DEFAULT_THEME_NAME
    {
        crate::config::theme::DEFAULT_THEME_NAME
    } else {
        name
    };
    theme_presets()
        .iter()
        .find(|preset| preset.id == id)
        .map(|preset| preset.body)
}

pub fn render_theme_preset(preset: &ThemePreset) -> String {
    preset.body.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmComponentDraft {
    pub template_id: String,
    pub file_id: String,
    pub provider_name: String,
    pub format: String,
    pub base_url: String,
    pub model: String,
}

pub fn llm_templates() -> impl Iterator<Item = &'static Template> {
    registry()
        .iter()
        .filter(|template| template.kind == TemplateKind::PostLlm)
}

pub fn llm_draft_from_template(template_id: &str) -> Option<LlmComponentDraft> {
    let template = llm_templates().find(|template| template.id == template_id)?;
    let values = template_values(template);
    Some(LlmComponentDraft {
        template_id: template_id.to_string(),
        file_id: template
            .path
            .strip_prefix("post/llm/")
            .and_then(|path| path.strip_suffix(".toml"))
            .unwrap_or("llm")
            .to_string(),
        provider_name: string_value(values, "name")
            .unwrap_or("provider")
            .to_string(),
        format: string_value(values, "format")
            .unwrap_or("openai")
            .to_string(),
        base_url: string_value(values, "base_url")
            .unwrap_or_default()
            .to_string(),
        model: string_value(values, "model")
            .unwrap_or_default()
            .to_string(),
    })
}

pub fn render_llm_component(draft: &LlmComponentDraft) -> Result<String> {
    validate_component_id(&draft.file_id).context("invalid file id")?;
    validate_component_id(&draft.provider_name).context("invalid provider name")?;
    anyhow::ensure!(
        matches!(draft.format.as_str(), "openai" | "anthropic"),
        "format must be openai or anthropic"
    );
    anyhow::ensure!(!draft.model.trim().is_empty(), "model is required");

    let template = llm_templates()
        .find(|template| template.id == draft.template_id)
        .with_context(|| format!("unknown LLM template {}", draft.template_id))?;
    let body = render_llm_component_body(template, draft);
    toml::from_str::<toml::Value>(&body).context("rendered LLM template is invalid TOML")?;
    Ok(body)
}

pub fn create_llm_component(post_dir: &Path, draft: &LlmComponentDraft) -> Result<PathBuf> {
    let llm_dir = post_dir.join("llm");
    let path = llm_dir.join(format!("{}.toml", draft.file_id));
    anyhow::ensure!(
        !path.exists(),
        "post/llm/{}.toml already exists",
        draft.file_id
    );
    ensure_provider_name_available(&llm_dir, &draft.provider_name)?;
    let body = render_llm_component(draft)?;
    std::fs::create_dir_all(&llm_dir).with_context(|| format!("create {}", llm_dir.display()))?;
    std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn ensure_provider_name_available(llm_dir: &Path, provider_name: &str) -> Result<()> {
    let mut names = BTreeSet::new();
    for path in toml_files(llm_dir) {
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("read existing LLM component {}", path.display()))?;
        let value: toml::Value = toml::from_str(&body)
            .with_context(|| format!("parse existing LLM component {}", path.display()))?;
        if let Some(name) = value.get("name").and_then(toml::Value::as_str) {
            names.insert(name.to_string());
        }
    }
    anyhow::ensure!(
        !names.contains(provider_name),
        "provider name {provider_name:?} already exists"
    );
    Ok(())
}

fn validate_component_id(value: &str) -> Result<()> {
    anyhow::ensure!(!value.trim().is_empty(), "value is required");
    anyhow::ensure!(
        value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'),
        "use only ASCII letters, digits, '-' and '_'"
    );
    Ok(())
}

fn toml_files(dir: &Path) -> Vec<PathBuf> {
    let mut paths = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    paths.sort();
    paths
}

fn template_values(template: &Template) -> &[(&'static str, TemplateValue)] {
    template.values
}

fn string_value<'a>(values: &'a [(&str, TemplateValue)], key: &str) -> Option<&'a str> {
    values.iter().find_map(|(name, value)| match (name, value) {
        (name, TemplateValue::String(value)) if *name == key => Some(*value),
        _ => None,
    })
}

fn render_llm_component_body(template: &Template, draft: &LlmComponentDraft) -> String {
    let mut body = String::new();
    body.push_str("type = \"llm\"\n");
    body.push_str(&format!("format = {:?}\n", draft.format));
    body.push_str(&format!("name = {:?}\n", draft.provider_name));
    body.push_str(&format!("base_url = {:?}\n", draft.base_url));
    body.push_str("api_key = \"\"\n");
    body.push_str(&format!("model = {:?}\n", draft.model));
    body.push_str("prompt = \"{{text}}\"\n");

    if template
        .values
        .iter()
        .any(|(name, _)| *name == "extra_body")
    {
        body.push_str("\n[extra_body]\n");
        body.push_str("thinking = { type = \"disabled\" }\n");
    }
    body
}

fn render_from_spec(
    spec: &ConfigSpec,
    values: &[(&str, TemplateValue)],
    lang: Option<crate::i18n::Lang>,
) -> String {
    let mut body = String::new();
    let mut table_values = Vec::new();

    for field in spec.fields() {
        let Some((_, value)) = values.iter().find(|(name, _)| *name == field.name()) else {
            continue;
        };
        if matches!(value, TemplateValue::Table(_)) {
            table_values.push((field.name(), *value));
            continue;
        }
        push_field_comment(&mut body, field, lang);
        body.push_str(&format!("{} = {}\n", field.name(), render_value(value)));
    }

    if !body.is_empty() && !table_values.is_empty() {
        body.push('\n');
    }

    for (idx, (name, value)) in table_values.iter().enumerate() {
        if idx > 0 {
            body.push('\n');
        }
        if let Some(field) = spec.field_for_path(name) {
            push_field_comment(&mut body, field, lang);
        }
        body.push_str(&format!("[{name}]\n"));
        let TemplateValue::Table(entries) = value else {
            continue;
        };
        for (key, value) in *entries {
            let field_path = format!("{name}.{key}");
            if let Some(field) = spec.field_for_path(&field_path) {
                push_field_comment(&mut body, field, lang);
            }
            body.push_str(&format!("{key} = {}\n", render_value(value)));
        }
    }

    body
}

fn push_field_comment(
    body: &mut String,
    field: &crate::config::spec::FieldSpec,
    lang: Option<crate::i18n::Lang>,
) {
    let Some(lang) = lang else {
        return;
    };
    let Some(key) = field.description_key_value() else {
        return;
    };
    let text = crate::i18n::tr_lang(lang, key, &[]);
    for line in text.lines() {
        body.push_str("# ");
        body.push_str(line);
        body.push('\n');
    }
}

fn render_value(value: &TemplateValue) -> String {
    match value {
        TemplateValue::String(value) => format!("{value:?}"),
        TemplateValue::Integer(value) => value.to_string(),
        TemplateValue::Bool(value) => value.to_string(),
        TemplateValue::StringArray(values) => {
            let values = values
                .iter()
                .map(|value| format!("{value:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{values}]")
        }
        TemplateValue::InlineTable(entries) => {
            let entries = entries
                .iter()
                .map(|(key, value)| format!("{key} = {}", render_value(value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{ {entries} }}")
        }
        TemplateValue::Table(_) => unreachable!("tables are rendered by section"),
    }
}

const CONFIG_VALUES: &[(&str, TemplateValue)] = &[
    (
        "hotkey",
        TemplateValue::Table(&[
            ("trigger", TemplateValue::String("f16")),
            ("cancel", TemplateValue::String("escape")),
        ]),
    ),
    (
        "voice",
        TemplateValue::Table(&[
            ("stop_delay_ms", TemplateValue::Integer(800)),
            ("record_audio", TemplateValue::String("off")),
            ("auto_paste", TemplateValue::Bool(true)),
        ]),
    ),
    (
        "voice.vad",
        TemplateValue::Table(&[("backend", TemplateValue::String("off"))]),
    ),
    (
        "post",
        TemplateValue::Table(&[("timeout_ms", TemplateValue::Integer(2000))]),
    ),
    (
        "profile",
        TemplateValue::Table(&[("default", TemplateValue::String("default"))]),
    ),
    (
        "ui",
        TemplateValue::Table(&[
            ("language", TemplateValue::String("auto")),
            ("theme", TemplateValue::String("gruvbox-dark")),
            ("theme_tui", TemplateValue::String("")),
            ("theme_overlay", TemplateValue::String("")),
        ]),
    ),
];

const ASR_APPLE_VALUES: &[(&str, TemplateValue)] = &[
    ("language", TemplateValue::String("zh-CN")),
    ("install_assets", TemplateValue::Bool(true)),
    ("idle_pause", TemplateValue::Bool(false)),
    ("finalize_timeout_ms", TemplateValue::Integer(5000)),
];

const ASR_DOUBAO_VALUES: &[(&str, TemplateValue)] = &[
    ("app_key", TemplateValue::String("")),
    ("access_key", TemplateValue::String("")),
    (
        "resource_id",
        TemplateValue::String("volc.bigasr.sauc.duration"),
    ),
    ("language", TemplateValue::String("")),
    ("enable_itn", TemplateValue::Bool(true)),
    ("enable_punc", TemplateValue::Bool(true)),
    ("enable_ddc", TemplateValue::Bool(true)),
    ("stream_mode", TemplateValue::Integer(2)),
    ("ai_vad", TemplateValue::Bool(false)),
    ("idle_pause", TemplateValue::Bool(false)),
    ("finalize_timeout_ms", TemplateValue::Integer(12_000)),
];

const DEFAULT_PROFILE_VALUES: &[(&str, TemplateValue)] = &[
    ("name", TemplateValue::String("default")),
    (
        "asr",
        TemplateValue::Table(&[
            ("provider", TemplateValue::String("doubao")),
            ("hotwords", TemplateValue::StringArray(&[])),
        ]),
    ),
    (
        "post",
        TemplateValue::Table(&[("chain", TemplateValue::StringArray(&["rule:zh_filter"]))]),
    ),
];

const ZH_FILTER_VALUES: &[(&str, TemplateValue)] = &[
    ("type", TemplateValue::String("rule")),
    (
        "patterns",
        TemplateValue::StringArray(&["嗯", "呃", "啊", "就是"]),
    ),
];

const DEEPSEEK_VALUES: &[(&str, TemplateValue)] = &[
    ("type", TemplateValue::String("llm")),
    ("format", TemplateValue::String("openai")),
    ("name", TemplateValue::String("deepseek")),
    (
        "base_url",
        TemplateValue::String("https://api.deepseek.com"),
    ),
    ("api_key", TemplateValue::String("")),
    ("model", TemplateValue::String("deepseek-chat")),
    ("prompt", TemplateValue::String("{{text}}")),
    (
        "extra_body",
        TemplateValue::Table(&[(
            "thinking",
            TemplateValue::InlineTable(&[("type", TemplateValue::String("disabled"))]),
        )]),
    ),
];

const OPENAI_VALUES: &[(&str, TemplateValue)] = &[
    ("type", TemplateValue::String("llm")),
    ("format", TemplateValue::String("openai")),
    ("name", TemplateValue::String("openai")),
    (
        "base_url",
        TemplateValue::String("https://api.openai.com/v1"),
    ),
    ("api_key", TemplateValue::String("")),
    ("model", TemplateValue::String("gpt-4.1-mini")),
    ("prompt", TemplateValue::String("{{text}}")),
];

const ANTHROPIC_VALUES: &[(&str, TemplateValue)] = &[
    ("type", TemplateValue::String("llm")),
    ("format", TemplateValue::String("anthropic")),
    ("name", TemplateValue::String("anthropic")),
    (
        "base_url",
        TemplateValue::String("https://api.anthropic.com"),
    ),
    ("api_key", TemplateValue::String("")),
    ("model", TemplateValue::String("claude-haiku-4-5")),
    ("prompt", TemplateValue::String("{{text}}")),
];

const TEMPLATES: &[Template] = &[
    Template {
        id: "config",
        kind: TemplateKind::Config,
        path: "config.toml",
        title: "Config",
        description: "Top-level shuohua config.toml.",
        schema: SchemaId::Main,
        values: CONFIG_VALUES,
    },
    Template {
        id: "asr/apple",
        kind: TemplateKind::Asr,
        path: "asr/apple.toml",
        title: "Apple ASR",
        description: "Starter config for the local Apple SpeechAnalyzer provider.",
        schema: SchemaId::AsrApple,
        values: ASR_APPLE_VALUES,
    },
    Template {
        id: "asr/doubao",
        kind: TemplateKind::Asr,
        path: "asr/doubao.toml",
        title: "Doubao ASR",
        description: "Starter config for the Doubao provider.",
        schema: SchemaId::AsrDoubao,
        values: ASR_DOUBAO_VALUES,
    },
    Template {
        id: "profile/default",
        kind: TemplateKind::Profile,
        path: "profile/default.toml",
        title: "Default profile",
        description: "Default profile using Doubao ASR and the zh_filter rule.",
        schema: SchemaId::Profile,
        values: DEFAULT_PROFILE_VALUES,
    },
    Template {
        id: "post/rule/zh_filter",
        kind: TemplateKind::PostRule,
        path: "post/rule/zh_filter.toml",
        title: "Chinese speech cleanup rule",
        description: "Rule processor for common Chinese filler words.",
        schema: SchemaId::PostRule,
        values: ZH_FILTER_VALUES,
    },
    Template {
        id: "post/llm/deepseek",
        kind: TemplateKind::PostLlm,
        path: "post/llm/deepseek.toml",
        title: "DeepSeek",
        description: "OpenAI-compatible DeepSeek post-processing preset.",
        schema: SchemaId::PostLlm,
        values: DEEPSEEK_VALUES,
    },
    Template {
        id: "post/llm/openai",
        kind: TemplateKind::PostLlm,
        path: "post/llm/openai.toml",
        title: "OpenAI",
        description: "OpenAI post-processing preset.",
        schema: SchemaId::PostLlm,
        values: OPENAI_VALUES,
    },
    Template {
        id: "post/llm/anthropic",
        kind: TemplateKind::PostLlm,
        path: "post/llm/anthropic.toml",
        title: "Anthropic",
        description: "Anthropic post-processing preset.",
        schema: SchemaId::PostLlm,
        values: ANTHROPIC_VALUES,
    },
];

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
    fn rendered_templates_include_field_comments_from_schema() {
        let body = render_by_id("config").unwrap();

        assert!(body.contains("# Hotkey that toggles recording."));
        assert!(body.contains("trigger = \"f16\""));
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
        crate::config::parse(&render_by_id("config").unwrap()).unwrap();

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
