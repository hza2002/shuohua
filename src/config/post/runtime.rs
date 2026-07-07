use std::fmt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Map as JsonMap, Value as JsonValue};
use toml::value::Table;

use crate::config::schema;
use crate::config::spec::validate_value;

#[derive(Debug, Clone)]
pub struct PostChainConfig {
    pub name: String,
    pub processors: Vec<ProcessorConfig>,
}

#[derive(Clone, PartialEq)]
pub enum ProcessorConfig {
    Rule {
        id: String,
        patterns: Vec<String>,
    },
    Llm {
        id: String,
        format: ProviderFormatCfg,
        provider_name: String,
        base_url: String,
        api_key: String,
        model: String,
        extra_body: JsonMap<String, JsonValue>,
        system_prompt: Option<String>,
        prompt: String,
    },
}

impl fmt::Debug for ProcessorConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rule { id, patterns } => f
                .debug_struct("Rule")
                .field("id", id)
                .field("patterns", patterns)
                .finish(),
            Self::Llm {
                id,
                format,
                provider_name,
                base_url,
                api_key: _,
                model,
                extra_body,
                system_prompt,
                prompt,
            } => f
                .debug_struct("Llm")
                .field("id", id)
                .field("format", format)
                .field("provider_name", provider_name)
                .field("base_url", base_url)
                .field("api_key", &"<redacted>")
                .field("model", model)
                .field("extra_body", extra_body)
                .field("system_prompt", system_prompt)
                .field("prompt", prompt)
                .finish(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PostDir {
    pub dir: PathBuf,
}

pub fn default_dir() -> PathBuf {
    crate::config::paths::post_dir()
}

pub fn load_components(
    chain: &[String],
    dir: &PostDir,
    llm_overrides: &Table,
) -> Result<PostChainConfig> {
    load_chain_config(chain, dir, llm_overrides)
}

pub fn load_chain_config(
    chain: &[String],
    dir: &PostDir,
    llm_overrides: &Table,
) -> Result<PostChainConfig> {
    let mut processors = Vec::with_capacity(chain.len());
    for id in chain {
        processors.push(load_component(id, dir, llm_overrides)?);
    }
    Ok(PostChainConfig {
        name: chain.join(" → "),
        processors,
    })
}

fn load_component(id: &str, dir: &PostDir, llm_overrides: &Table) -> Result<ProcessorConfig> {
    crate::config::inventory::validate_config_file_id(id)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("invalid post component id {id:?}"))?;
    let path = dir.dir.join(format!("{id}.toml"));
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("read post component {}", path.display()))?;
    let mut value: toml::Value = toml::from_str(&body)
        .with_context(|| format!("parse post component {}", path.display()))?;
    let kind = crate::config::post::kind_from_value(id, &path, &value)?;
    if kind == crate::config::post::PostKind::Llm {
        if let Some(override_value) = llm_overrides.get(id) {
            let override_table = override_value.as_table().with_context(|| {
                format!("post.overrides.{id} override for {id:?} must be a TOML table")
            })?;
            merge_table(&mut value, override_table)
                .with_context(|| format!("merge post.overrides.{id} override into {id:?}"))?;
        }
    }
    validate_component_value(kind, &value)
        .with_context(|| format!("validate post component {}", path.display()))?;
    let cfg: ProcessorCfg = value
        .try_into()
        .with_context(|| format!("parse post component {}", path.display()))?;
    cfg.into_config(id)
}

pub fn load_llm_config(id: &str, dir: &PostDir, llm_overrides: &Table) -> Result<ProcessorConfig> {
    match load_component(id, dir, llm_overrides)? {
        cfg @ ProcessorConfig::Llm { .. } => Ok(cfg),
        ProcessorConfig::Rule { .. } => {
            anyhow::bail!("post component {id:?} is not an llm component")
        }
    }
}

fn merge_table(value: &mut toml::Value, overrides: &Table) -> Result<()> {
    let table = value
        .as_table_mut()
        .context("expected top-level TOML table")?;
    for (key, value) in overrides {
        table.insert(key.clone(), value.clone());
    }
    Ok(())
}

fn validate_component_value(
    kind: crate::config::post::PostKind,
    value: &toml::Value,
) -> Result<()> {
    let spec = schema::spec_for(kind.schema_id());
    crate::config::main::reject_schema_diagnostics(validate_value(&spec, value))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ProcessorCfg {
    Rule {
        #[serde(default)]
        #[serde(rename = "name")]
        _name: Option<String>,
        patterns: Vec<String>,
    },
    Llm {
        #[serde(default = "default_format")]
        format: ProviderFormatCfg,
        #[serde(default)]
        name: Option<String>,
        base_url: String,
        api_key: String,
        model: String,
        #[serde(default)]
        extra_body: JsonMap<String, JsonValue>,
        #[serde(default)]
        system_prompt: Option<String>,
        prompt: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFormatCfg {
    Openai,
    Anthropic,
}

fn default_format() -> ProviderFormatCfg {
    ProviderFormatCfg::Openai
}

impl ProcessorCfg {
    fn into_config(self, id: &str) -> Result<ProcessorConfig> {
        match self {
            ProcessorCfg::Rule { _name, patterns } => Ok(ProcessorConfig::Rule {
                id: id.to_string(),
                patterns,
            }),
            ProcessorCfg::Llm {
                format,
                name,
                base_url,
                api_key,
                model,
                extra_body,
                system_prompt,
                prompt,
            } => Ok(ProcessorConfig::Llm {
                id: id.to_string(),
                format,
                provider_name: name.unwrap_or_else(|| id.to_string()),
                base_url,
                api_key,
                model,
                extra_body,
                system_prompt,
                prompt,
            }),
        }
    }
}

#[cfg(test)]
fn load_llm_config_for_test(
    id: &str,
    dir: &PostDir,
    llm_overrides: &Table,
) -> Result<ProcessorConfig> {
    load_llm_config(id, dir, llm_overrides)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shuohua-post-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_openai_and_anthropic_llm_processor_configs() {
        let dir = temp_dir();
        let post = dir.join("post");
        fs::create_dir_all(&post).unwrap();
        fs::write(
            post.join("openai_cleanup.toml"),
            r#"
type = "llm"
format = "openai"
name = "deepseek"
base_url = "https://api.deepseek.com"
api_key = "sk-test"
model = "deepseek-chat"
system_prompt = "clean speech"
prompt = "app={{app_name}} text={{text}}"
[extra_body]
thinking = { type = "disabled" }
"#,
        )
        .unwrap();
        fs::write(
            post.join("anthropic_cleanup.toml"),
            r#"
type = "llm"
format = "anthropic"
name = "anthropic"
base_url = "https://api.anthropic.com"
api_key = "sk-ant-test"
model = "claude-haiku-4-5"
prompt = "{{text}}"
"#,
        )
        .unwrap();
        let dir = PostDir { dir: post };

        let chain = load_components(
            &[
                "openai_cleanup".to_string(),
                "anthropic_cleanup".to_string(),
            ],
            &dir,
            &Table::new(),
        )
        .unwrap();

        assert_eq!(chain.name, "openai_cleanup → anthropic_cleanup");
        assert_eq!(chain.processors.len(), 2);
        assert!(matches!(
            &chain.processors[0],
            ProcessorConfig::Llm {
                id,
                format: ProviderFormatCfg::Openai,
                provider_name,
                base_url,
                ..
            } if id == "openai_cleanup"
                && provider_name == "deepseek"
                && base_url == "https://api.deepseek.com"
        ));
        assert!(matches!(
            &chain.processors[1],
            ProcessorConfig::Llm {
                id,
                format: ProviderFormatCfg::Anthropic,
                provider_name,
                base_url,
                ..
            } if id == "anthropic_cleanup"
                && provider_name == "anthropic"
                && base_url == "https://api.anthropic.com"
        ));
        let _ = fs::remove_dir_all(dir.dir);
    }

    #[test]
    fn loads_rule_and_llm_components_from_post_dir() {
        let root = temp_dir();
        let post = root.join("post");
        fs::create_dir_all(&post).unwrap();
        fs::write(
            post.join("zh_filter.toml"),
            r#"
type = "rule"
patterns = ["嗯", "啊"]
"#,
        )
        .unwrap();
        fs::write(
            post.join("deepseek.toml"),
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
        let dir = PostDir { dir: post };

        let chain = load_components(
            &["zh_filter".to_string(), "deepseek".to_string()],
            &dir,
            &Table::new(),
        )
        .unwrap();

        assert_eq!(chain.name, "zh_filter → deepseek");
        assert_eq!(chain.processors.len(), 2);
        assert!(matches!(
            &chain.processors[0],
            ProcessorConfig::Rule { id, patterns }
                if id == "zh_filter" && patterns == &vec!["嗯".to_string(), "啊".to_string()]
        ));
        assert!(matches!(
            &chain.processors[1],
            ProcessorConfig::Llm {
                id,
                provider_name,
                base_url,
                ..
            } if id == "deepseek"
                && provider_name == "deepseek"
                && base_url == "https://api.deepseek.com"
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn loads_named_rule_component() {
        let root = temp_dir();
        let post = root.join("post");
        fs::create_dir_all(&post).unwrap();
        fs::write(
            post.join("zh_filter.toml"),
            r#"
type = "rule"
patterns = ["嗯", "呃", "啊"]
"#,
        )
        .unwrap();
        let dir = PostDir { dir: post };

        let chain = load_components(&["zh_filter".to_string()], &dir, &Table::new()).unwrap();

        assert_eq!(chain.name, "zh_filter");
        assert_eq!(chain.processors.len(), 1);
        assert!(matches!(
            &chain.processors[0],
            ProcessorConfig::Rule { id, patterns }
                if id == "zh_filter"
                    && patterns == &vec!["嗯".to_string(), "呃".to_string(), "啊".to_string()]
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nameless_llm_component_falls_back_to_id_for_provider_name() {
        let root = temp_dir();
        let post = root.join("post");
        fs::create_dir_all(&post).unwrap();
        fs::write(
            post.join("cleaner.toml"),
            r#"
type = "llm"
base_url = "https://api.deepseek.com"
api_key = "sk-test"
model = "deepseek-chat"
prompt = "{{text}}"
"#,
        )
        .unwrap();
        let dir = PostDir { dir: post };

        let cfg = load_llm_config_for_test("cleaner", &dir, &Table::new()).unwrap();

        assert!(matches!(
            &cfg,
            ProcessorConfig::Llm {
                id,
                provider_name,
                base_url,
                ..
            } if id == "cleaner"
                && provider_name == "cleaner"
                && base_url == "https://api.deepseek.com"
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn applies_llm_component_overrides() {
        let root = temp_dir();
        let post = root.join("post");
        fs::create_dir_all(&post).unwrap();
        fs::write(
            post.join("deepseek.toml"),
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
        let mut overrides = Table::new();
        overrides.insert(
            "deepseek".to_string(),
            toml::Value::Table(toml::toml! {
                model = "deepseek-v4-flash"
                system_prompt = "terminal"
                extra_body = { thinking = { type = "disabled" } }
            }),
        );
        let dir = PostDir { dir: post };

        let cfg = load_llm_config_for_test("deepseek", &dir, &overrides).unwrap();
        let chain = load_components(&["deepseek".to_string()], &dir, &overrides).unwrap();

        assert!(matches!(
            &cfg,
            ProcessorConfig::Llm {
                model,
                system_prompt,
                extra_body,
                ..
            } if model == "deepseek-v4-flash"
                && system_prompt.as_deref() == Some("terminal")
                && extra_body.get("thinking") == Some(&serde_json::json!({ "type": "disabled" }))
        ));
        assert_eq!(chain.name, "deepseek");
        assert!(matches!(
            &chain.processors[0],
            ProcessorConfig::Llm {
                id,
                model,
                system_prompt,
                extra_body,
                ..
            } if id == "deepseek"
                && model == "deepseek-v4-flash"
                && system_prompt.as_deref() == Some("terminal")
                && extra_body.get("thinking") == Some(&serde_json::json!({ "type": "disabled" }))
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_unknown_llm_override_fields() {
        let root = temp_dir();
        let post = root.join("post");
        fs::create_dir_all(&post).unwrap();
        fs::write(
            post.join("deepseek.toml"),
            r#"
type = "llm"
name = "deepseek"
base_url = "https://api.deepseek.com"
api_key = "sk-test"
model = "deepseek-chat"
prompt = "{{text}}"
"#,
        )
        .unwrap();
        let mut overrides = Table::new();
        overrides.insert(
            "deepseek".to_string(),
            toml::Value::Table(
                [("modle".to_string(), toml::Value::String("typo".to_string()))]
                    .into_iter()
                    .collect(),
            ),
        );
        let dir = PostDir { dir: post };

        let error = load_llm_config_for_test("deepseek", &dir, &overrides).unwrap_err();
        let error = format!("{error:#}");

        assert!(error.contains("modle"), "{error}");
        assert!(error.contains("unknown field"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_invalid_post_component_file_id_before_reading_file() {
        let root = temp_dir();
        let dir = PostDir {
            dir: root.join("post"),
        };

        let error = load_components(&["BadName".to_string()], &dir, &Table::new()).unwrap_err();
        let error = format!("{error:#}");

        assert!(error.contains("invalid post component id"), "{error}");
        assert!(error.contains("lowercase letter first"), "{error}");
        assert!(!error.contains("read post component"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn processor_config_debug_redacts_api_key() {
        let config = ProcessorConfig::Llm {
            id: "deepseek".to_string(),
            format: ProviderFormatCfg::Openai,
            provider_name: "deepseek".to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            api_key: "sk-secret".to_string(),
            model: "deepseek-chat".to_string(),
            extra_body: JsonMap::new(),
            system_prompt: None,
            prompt: "{{text}}".to_string(),
        };

        let debug = format!("{config:?}");

        assert!(!debug.contains("sk-secret"), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
    }
}
