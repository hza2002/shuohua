use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{registry, Template, TemplateKind, TemplateValue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmComponentDraft {
    pub template_id: String,
    pub file_id: String,
    pub provider_name: String,
    pub format: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub system_prompt: String,
    pub prompt: String,
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
            .strip_prefix("post/")
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
        api_key: string_value(values, "api_key")
            .unwrap_or_default()
            .to_string(),
        model: string_value(values, "model")
            .unwrap_or_default()
            .to_string(),
        system_prompt: string_value(values, "system_prompt")
            .unwrap_or("你是 ASR 文本整理器。保留用户原意，只清理口误、重复、标点和明确的识别错误。只输出整理后的文本。")
            .to_string(),
        prompt: string_value(values, "prompt").unwrap_or("{{text}}").to_string(),
    })
}

pub fn render_llm_component(draft: &LlmComponentDraft) -> Result<String> {
    crate::config::inventory::validate_config_file_id(&draft.file_id)
        .map_err(anyhow::Error::msg)
        .context("invalid file name")?;
    anyhow::ensure!(
        matches!(draft.format.as_str(), "openai" | "anthropic"),
        "format must be openai or anthropic"
    );
    anyhow::ensure!(
        draft.base_url.starts_with("http://") || draft.base_url.starts_with("https://"),
        "base_url is required and must start with http:// or https://"
    );
    anyhow::ensure!(!draft.model.trim().is_empty(), "model is required");
    anyhow::ensure!(!draft.prompt.trim().is_empty(), "prompt is required");

    let template = llm_templates()
        .find(|template| template.id == draft.template_id)
        .with_context(|| format!("unknown LLM template {}", draft.template_id))?;
    let body = render_llm_component_body(template, draft);
    toml::from_str::<toml::Value>(&body).context("rendered LLM template is invalid TOML")?;
    Ok(body)
}

pub fn create_llm_component(post_dir: &Path, draft: &LlmComponentDraft) -> Result<PathBuf> {
    let path = post_dir.join(format!("{}.toml", draft.file_id));
    anyhow::ensure!(
        !path.exists(),
        "file name {:?} already exists; pick a different file name",
        draft.file_id
    );
    let body = render_llm_component(draft)?;
    std::fs::create_dir_all(post_dir).with_context(|| format!("create {}", post_dir.display()))?;
    std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn template_values(template: &Template) -> &[(&'static str, TemplateValue)] {
    template.values
}

fn string_value<'a>(values: &'a [(&str, TemplateValue)], key: &str) -> Option<&'a str> {
    values.iter().find_map(|(name, value)| match (name, value) {
        (name, TemplateValue::String(value)) if *name == key => Some(*value),
        (name, TemplateValue::MultilineString(value)) if *name == key => Some(*value),
        _ => None,
    })
}

fn render_llm_component_body(template: &Template, draft: &LlmComponentDraft) -> String {
    let mut body = String::new();
    body.push_str("type = \"llm\"\n");
    body.push_str(&format!("format = {:?}\n", draft.format));
    body.push_str(&format!("name = {:?}\n", draft.provider_name));
    body.push_str(&format!("base_url = {:?}\n", draft.base_url));
    body.push_str(&format!("api_key = {:?}\n", draft.api_key));
    body.push_str(&format!("model = {:?}\n", draft.model));
    if !draft.system_prompt.trim().is_empty() {
        body.push_str(&format!("system_prompt = {:?}\n", draft.system_prompt));
    }
    body.push_str(&format!("prompt = {:?}\n", draft.prompt));

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
