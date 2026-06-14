use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::llm::{LlmCleanup, LlmCleanupConfig, ProviderFormat};
use super::{AppContext, PostProcessor, RuleBasedFiller};

pub struct PostChain {
    pub name: String,
    pub processors: Vec<Box<dyn PostProcessor>>,
}

pub fn default_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("shuohua/post");
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/shuohua/post")
}

pub fn load_for_app(dir: &Path, ctx: &AppContext, _timeout: Duration) -> Result<PostChain> {
    let Some(path) = chain_path(dir, ctx) else {
        return Ok(PostChain::builtin_filler());
    };
    load_from(&path)
}

fn chain_path(dir: &Path, ctx: &AppContext) -> Option<PathBuf> {
    if let Some(bundle_id) = ctx.bundle_id.as_deref() {
        let app_path = dir.join(format!("{bundle_id}.toml"));
        if app_path.exists() {
            return Some(app_path);
        }
    }
    let default_path = dir.join("default.toml");
    default_path.exists().then_some(default_path)
}

impl PostChain {
    pub fn builtin_filler() -> Self {
        Self {
            name: "filler".to_string(),
            processors: vec![Box::new(RuleBasedFiller::default_patterns())],
        }
    }
}

pub fn load_from(path: &Path) -> Result<PostChain> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("read post chain {}", path.display()))?;
    let cfg: ChainFile =
        toml::from_str(&body).with_context(|| format!("parse post chain {}", path.display()))?;
    cfg.into_chain()
}

#[derive(Debug, Deserialize)]
struct ChainFile {
    #[serde(default = "default_chain_name")]
    name: String,
    #[serde(default)]
    chain: Vec<String>,
    #[serde(default)]
    processors: std::collections::BTreeMap<String, ProcessorCfg>,
}

fn default_chain_name() -> String {
    "post".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ProcessorCfg {
    Rule {
        patterns: Vec<String>,
    },
    Llm {
        #[serde(default = "default_format")]
        format: ProviderFormatCfg,
        name: String,
        #[serde(default)]
        base_url: Option<String>,
        api_key: String,
        model: String,
        #[serde(default)]
        system_prompt: Option<String>,
        prompt: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProviderFormatCfg {
    Openai,
    Anthropic,
}

fn default_format() -> ProviderFormatCfg {
    ProviderFormatCfg::Openai
}

impl ChainFile {
    fn into_chain(self) -> Result<PostChain> {
        let mut processors: Vec<Box<dyn PostProcessor>> = Vec::with_capacity(self.chain.len());
        for id in &self.chain {
            let cfg = self
                .processors
                .get(id)
                .with_context(|| format!("post processor {id:?} missing config"))?;
            processors.push(cfg.build(id)?);
        }
        Ok(PostChain {
            name: self.name,
            processors,
        })
    }
}

impl ProcessorCfg {
    fn build(&self, id: &str) -> Result<Box<dyn PostProcessor>> {
        match self {
            ProcessorCfg::Rule { patterns } => {
                let borrowed = patterns.iter().map(String::as_str).collect::<Vec<_>>();
                Ok(Box::new(RuleBasedFiller::with_name(id, &borrowed)))
            }
            ProcessorCfg::Llm {
                format,
                name,
                base_url,
                api_key,
                model,
                system_prompt,
                prompt,
            } => Ok(Box::new(LlmCleanup::new(LlmCleanupConfig {
                name: id.to_string(),
                format: match format {
                    ProviderFormatCfg::Openai => ProviderFormat::OpenAi,
                    ProviderFormatCfg::Anthropic => ProviderFormat::Anthropic,
                },
                provider_name: name.clone(),
                base_url: base_url
                    .clone()
                    .unwrap_or_else(|| default_base_url(*format, name)),
                api_key: api_key.clone(),
                model: model.clone(),
                system_prompt: system_prompt.clone(),
                prompt: prompt.clone(),
            }))),
        }
    }
}

fn default_base_url(format: ProviderFormatCfg, name: &str) -> String {
    match format {
        ProviderFormatCfg::Anthropic => "https://api.anthropic.com".to_string(),
        ProviderFormatCfg::Openai => match name {
            "openai" => "https://api.openai.com/v1".to_string(),
            "deepseek" => "https://api.deepseek.com".to_string(),
            "openrouter" => "https://openrouter.ai/api/v1".to_string(),
            _ => "https://api.openai.com/v1".to_string(),
        },
    }
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
    fn missing_per_app_and_default_falls_back_to_builtin_filler() {
        let dir = temp_dir();
        let ctx = AppContext {
            bundle_id: Some("com.example.App".to_string()),
            app_name: Some("Example".to_string()),
        };

        let chain = load_for_app(&dir, &ctx, Duration::from_secs(2)).unwrap();

        assert_eq!(chain.name, "filler");
        assert_eq!(chain.processors.len(), 1);
        assert_eq!(chain.processors[0].name(), "filler");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn per_app_file_wins_over_default_file() {
        let dir = temp_dir();
        fs::write(
            dir.join("default.toml"),
            r#"
name = "default"
chain = ["filler"]

[processors.filler]
type = "rule"
patterns = ["default"]
"#,
        )
        .unwrap();
        fs::write(
            dir.join("com.example.App.toml"),
            r#"
name = "app"
chain = ["custom"]

[processors.custom]
type = "rule"
patterns = ["app"]
"#,
        )
        .unwrap();
        let ctx = AppContext {
            bundle_id: Some("com.example.App".to_string()),
            app_name: None,
        };

        let chain = load_for_app(&dir, &ctx, Duration::from_secs(2)).unwrap();

        assert_eq!(chain.name, "app");
        assert_eq!(chain.processors[0].name(), "custom");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parses_openai_and_anthropic_llm_processors() {
        let dir = temp_dir();
        let path = dir.join("default.toml");
        fs::write(
            &path,
            r#"
name = "llm chain"
chain = ["openai_cleanup", "anthropic_cleanup"]

[processors.openai_cleanup]
type = "llm"
format = "openai"
name = "deepseek"
base_url = "https://api.deepseek.com"
api_key = "sk-test"
model = "deepseek-chat"
system_prompt = "clean speech"
prompt = "app={{app_name}} text={{text}}"

[processors.anthropic_cleanup]
type = "llm"
format = "anthropic"
name = "anthropic"
api_key = "sk-ant-test"
model = "claude-haiku-4-5"
prompt = "{{text}}"
"#,
        )
        .unwrap();

        let chain = load_from(&path).unwrap();

        assert_eq!(chain.name, "llm chain");
        assert_eq!(chain.processors.len(), 2);
        assert_eq!(chain.processors[0].name(), "openai_cleanup");
        assert_eq!(chain.processors[1].name(), "anthropic_cleanup");
        let _ = fs::remove_dir_all(dir);
    }
}
