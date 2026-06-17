#![cfg_attr(not(test), allow(dead_code))]

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
}
