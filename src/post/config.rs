use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::llm::{LlmCleanup, LlmCleanupConfig, ProviderFormat};
use super::{PostProcessor, RuleBasedFiller};

pub struct PostChain {
    pub name: String,
    pub processors: Vec<Box<dyn PostProcessor>>,
}

#[derive(Debug, Clone)]
pub struct PostDirs {
    pub rules: PathBuf,
    pub llm: PathBuf,
}

pub fn default_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("shuohua/post");
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/shuohua/post")
}

pub fn load_components(chain: &[String], dirs: &PostDirs) -> Result<PostChain> {
    let mut processors = Vec::with_capacity(chain.len());
    for id in chain {
        processors.push(load_component(id, dirs)?);
    }
    Ok(PostChain {
        name: chain.join(" → "),
        processors,
    })
}

fn load_component(id: &str, dirs: &PostDirs) -> Result<Box<dyn PostProcessor>> {
    let (kind, name) = id
        .split_once(':')
        .with_context(|| format!("post chain item {id:?} must be kind:name"))?;
    let path = match kind {
        "rule" => dirs.rules.join(format!("{name}.toml")),
        "llm" => dirs.llm.join(format!("{name}.toml")),
        other => anyhow::bail!("unknown post component kind {other:?} in {id:?}"),
    };
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("read post component {}", path.display()))?;
    let cfg: ProcessorCfg = toml::from_str(&body)
        .with_context(|| format!("parse post component {}", path.display()))?;
    cfg.build(id)
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
        thinking: Option<bool>,
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
                thinking,
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
                thinking: *thinking,
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
    fn parses_openai_and_anthropic_llm_processors() {
        let dir = temp_dir();
        let llm = dir.join("llm");
        fs::create_dir_all(&llm).unwrap();
        fs::write(
            llm.join("openai_cleanup.toml"),
            r#"
type = "llm"
format = "openai"
name = "deepseek"
base_url = "https://api.deepseek.com"
api_key = "sk-test"
model = "deepseek-chat"
system_prompt = "clean speech"
prompt = "app={{app_name}} text={{text}}"
"#,
        )
        .unwrap();
        fs::write(
            llm.join("anthropic_cleanup.toml"),
            r#"
type = "llm"
format = "anthropic"
name = "anthropic"
api_key = "sk-ant-test"
model = "claude-haiku-4-5"
prompt = "{{text}}"
"#,
        )
        .unwrap();
        let dirs = PostDirs {
            rules: dir.join("rules"),
            llm,
        };

        let chain = load_components(
            &[
                "llm:openai_cleanup".to_string(),
                "llm:anthropic_cleanup".to_string(),
            ],
            &dirs,
        )
        .unwrap();

        assert_eq!(chain.name, "llm:openai_cleanup → llm:anthropic_cleanup");
        assert_eq!(chain.processors.len(), 2);
        assert_eq!(chain.processors[0].name(), "llm:openai_cleanup");
        assert_eq!(chain.processors[1].name(), "llm:anthropic_cleanup");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn loads_rule_and_llm_components_from_post_dirs() {
        let dir = temp_dir();
        let rules = dir.join("rules");
        let llm = dir.join("llm");
        fs::create_dir_all(&rules).unwrap();
        fs::create_dir_all(&llm).unwrap();
        fs::write(
            rules.join("filler.toml"),
            r#"
type = "rule"
patterns = ["嗯", "啊"]
"#,
        )
        .unwrap();
        fs::write(
            llm.join("deepseek.toml"),
            r#"
type = "llm"
format = "openai"
name = "deepseek"
api_key = "sk-test"
model = "deepseek-chat"
prompt = "{{text}}"
"#,
        )
        .unwrap();
        let dirs = PostDirs { rules, llm };

        let chain = load_components(
            &["rule:filler".to_string(), "llm:deepseek".to_string()],
            &dirs,
        )
        .unwrap();

        assert_eq!(chain.name, "rule:filler → llm:deepseek");
        assert_eq!(chain.processors.len(), 2);
        assert_eq!(chain.processors[0].name(), "rule:filler");
        assert_eq!(chain.processors[1].name(), "llm:deepseek");
        let _ = fs::remove_dir_all(dir);
    }
}
