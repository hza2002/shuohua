#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::spec::{ConfigSpec, FieldSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    Main,
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
    spec: fn() -> ConfigSpec,
    values: &'static [(&'static str, TemplateValue)],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

pub fn manifest() -> String {
    let mut body = String::new();
    for template in registry() {
        body.push_str("[[templates]]\n");
        body.push_str(&format!("id = {:?}\n", template.id));
        body.push_str(&format!("kind = {:?}\n", kind_name(template.kind)));
        body.push_str(&format!("path = {:?}\n", template.path));
        body.push_str(&format!("title = {:?}\n", template.title));
        body.push_str(&format!("description = {:?}\n\n", template.description));
    }
    body
}

pub fn render(template: &Template) -> String {
    render_from_spec(&(template.spec)(), template.values)
}

pub fn render_by_id(id: &str) -> Option<String> {
    registry()
        .iter()
        .find(|template| template.id == id)
        .map(render)
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

fn render_from_spec(spec: &ConfigSpec, values: &[(&str, TemplateValue)]) -> String {
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
        body.push_str(&format!("{} = {}\n", field.name(), render_value(value)));
    }

    if !body.is_empty() && !table_values.is_empty() {
        body.push('\n');
    }

    for (idx, (name, value)) in table_values.iter().enumerate() {
        if idx > 0 {
            body.push('\n');
        }
        body.push_str(&format!("[{name}]\n"));
        let TemplateValue::Table(entries) = value else {
            continue;
        };
        for (key, value) in *entries {
            body.push_str(&format!("{key} = {}\n", render_value(value)));
        }
    }

    body
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

fn kind_name(kind: TemplateKind) -> &'static str {
    match kind {
        TemplateKind::Main => "main",
        TemplateKind::Profile => "profile",
        TemplateKind::PostRule => "post_rule",
        TemplateKind::PostLlm => "post_llm",
    }
}

fn main_spec() -> ConfigSpec {
    ConfigSpec::new("main")
        .field(FieldSpec::table("hotkey").required())
        .field(FieldSpec::table("voice").optional())
        .field(FieldSpec::table("voice.vad").optional())
        .field(FieldSpec::table("post").optional())
        .field(FieldSpec::table("profile").optional().free_table())
        .field(FieldSpec::table("ui").optional())
        .field(FieldSpec::table("overlay").optional())
}

fn profile_spec() -> ConfigSpec {
    ConfigSpec::new("profile")
        .field(FieldSpec::string("name").required())
        .field(FieldSpec::table("asr").required())
        .field(FieldSpec::table("post").optional())
}

fn post_rule_spec() -> ConfigSpec {
    ConfigSpec::new("post.rule")
        .field(
            FieldSpec::string("type")
                .required()
                .allowed_values(["rule"]),
        )
        .field(FieldSpec::array("patterns").required())
}

fn post_llm_spec() -> ConfigSpec {
    ConfigSpec::new("post.llm")
        .field(FieldSpec::string("type").required().allowed_values(["llm"]))
        .field(
            FieldSpec::string("format")
                .default("openai")
                .allowed_values(["openai", "anthropic"]),
        )
        .field(FieldSpec::string("name").required())
        .field(FieldSpec::string("base_url").optional())
        .field(FieldSpec::string("api_key").required().secret())
        .field(FieldSpec::string("model").required())
        .field(FieldSpec::string("system_prompt").optional())
        .field(FieldSpec::string("prompt").required())
        .field(FieldSpec::table("extra_body").optional().free_table())
}

const MAIN_VALUES: &[(&str, TemplateValue)] = &[
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
            ("record_audio", TemplateValue::Bool(false)),
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
        TemplateValue::Table(&[("language", TemplateValue::String("auto"))]),
    ),
];

const DEFAULT_PROFILE_VALUES: &[(&str, TemplateValue)] = &[
    ("name", TemplateValue::String("default")),
    (
        "asr",
        TemplateValue::Table(&[
            ("provider", TemplateValue::String("apple")),
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

const CUSTOM_OPENAI_VALUES: &[(&str, TemplateValue)] = &[
    ("type", TemplateValue::String("llm")),
    ("format", TemplateValue::String("openai")),
    ("name", TemplateValue::String("custom-openai")),
    (
        "base_url",
        TemplateValue::String("https://api.openai.com/v1"),
    ),
    ("api_key", TemplateValue::String("")),
    ("model", TemplateValue::String("")),
    ("prompt", TemplateValue::String("{{text}}")),
];

const CUSTOM_ANTHROPIC_VALUES: &[(&str, TemplateValue)] = &[
    ("type", TemplateValue::String("llm")),
    ("format", TemplateValue::String("anthropic")),
    ("name", TemplateValue::String("custom-anthropic")),
    (
        "base_url",
        TemplateValue::String("https://api.anthropic.com"),
    ),
    ("api_key", TemplateValue::String("")),
    ("model", TemplateValue::String("")),
    ("prompt", TemplateValue::String("{{text}}")),
];

const TEMPLATES: &[Template] = &[
    Template {
        id: "main",
        kind: TemplateKind::Main,
        path: "main.toml",
        title: "Main config",
        description: "Top-level shuohua config.toml.",
        spec: main_spec,
        values: MAIN_VALUES,
    },
    Template {
        id: "profile/default",
        kind: TemplateKind::Profile,
        path: "profile/default.toml",
        title: "Default profile",
        description: "Default profile using Apple ASR and the zh_filter rule.",
        spec: profile_spec,
        values: DEFAULT_PROFILE_VALUES,
    },
    Template {
        id: "post/rule/zh_filter",
        kind: TemplateKind::PostRule,
        path: "post/rule/zh_filter.toml",
        title: "Chinese speech cleanup rule",
        description: "Rule processor for common Chinese filler words.",
        spec: post_rule_spec,
        values: ZH_FILTER_VALUES,
    },
    Template {
        id: "post/llm/deepseek",
        kind: TemplateKind::PostLlm,
        path: "post/llm/deepseek.toml",
        title: "DeepSeek",
        description: "OpenAI-compatible DeepSeek post-processing preset.",
        spec: post_llm_spec,
        values: DEEPSEEK_VALUES,
    },
    Template {
        id: "post/llm/openai",
        kind: TemplateKind::PostLlm,
        path: "post/llm/openai.toml",
        title: "OpenAI",
        description: "OpenAI post-processing preset.",
        spec: post_llm_spec,
        values: OPENAI_VALUES,
    },
    Template {
        id: "post/llm/anthropic",
        kind: TemplateKind::PostLlm,
        path: "post/llm/anthropic.toml",
        title: "Anthropic",
        description: "Anthropic post-processing preset.",
        spec: post_llm_spec,
        values: ANTHROPIC_VALUES,
    },
    Template {
        id: "post/llm/custom-openai",
        kind: TemplateKind::PostLlm,
        path: "post/llm/custom-openai.toml",
        title: "Custom OpenAI-compatible",
        description: "Custom OpenAI-compatible provider template.",
        spec: post_llm_spec,
        values: CUSTOM_OPENAI_VALUES,
    },
    Template {
        id: "post/llm/custom-anthropic",
        kind: TemplateKind::PostLlm,
        path: "post/llm/custom-anthropic.toml",
        title: "Custom Anthropic",
        description: "Custom Anthropic-compatible provider template.",
        spec: post_llm_spec,
        values: CUSTOM_ANTHROPIC_VALUES,
    },
];

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::config::spec::ValueKind;

    use super::*;

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
    fn all_assets_match_rendered_registry_templates() {
        for template in registry() {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/config")
                .join(template.path);
            let asset = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            assert_eq!(asset, render(template), "{}", template.id);
        }
    }

    #[test]
    fn manifest_asset_matches_registry() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/config/manifest.toml");
        let asset = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

        assert_eq!(asset, manifest());
        let parsed: toml::Value = toml::from_str(&asset).unwrap();
        assert_eq!(
            parsed
                .get("templates")
                .and_then(toml::Value::as_array)
                .unwrap()
                .len(),
            registry().len()
        );
    }

    #[test]
    fn runtime_parsers_accept_core_templates_after_required_secrets_are_filled() {
        crate::config::parse(&render_by_id("main").unwrap()).unwrap();

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
            let spec = (template.spec)();
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
