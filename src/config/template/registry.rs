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
    pub(super) values: &'static [(&'static str, TemplateValue)],
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

#[derive(Debug, Clone, Copy)]
pub struct ThemePreset {
    pub id: &'static str,
    pub path: &'static str,
    pub(super) body: &'static str,
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

const CONFIG_VALUES: &[(&str, TemplateValue)] = &[
    (
        "hotkey",
        TemplateValue::Table(&[
            ("trigger", TemplateValue::String("right_option:double")),
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
        TemplateValue::Table(&[("timeout_ms", TemplateValue::Integer(10_000))]),
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
