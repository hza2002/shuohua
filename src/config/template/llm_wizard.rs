use std::collections::BTreeSet;
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
