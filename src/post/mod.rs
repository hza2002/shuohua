//! 文本后处理流水线（DESIGN §2.10）。
//!
//! 数据契约：
//! - [`PipelineText`] 流过整条链。`raw` 永远是原始 ASR 文本；`text` 是当前
//!   in-flight 版本；`segments` 是本次 recording 的 ASR session 文本列表。
//! - run_chain "链不阻塞"：单步 processor 失败/超时**跳过**，下一个继续用上一步
//!   的 text。最差产出 == raw（不会丢内容）。

pub mod app_context;
pub mod llm;
pub mod zh_filter;

use anyhow::Result;
use async_trait::async_trait;
use std::time::{Duration, Instant};
use thiserror::Error;

use crate::config::post::{PostChainConfig, ProcessorConfig, ProviderFormatCfg};
use crate::post::llm::{LlmCleanup, LlmCleanupConfig, ProviderFormat};
pub use zh_filter::ZhFilter;

pub struct PostChain {
    pub name: String,
    pub processors: Vec<Box<dyn PostProcessor>>,
}

pub fn build_chain(config: PostChainConfig) -> Result<PostChain> {
    let mut processors = Vec::with_capacity(config.processors.len());
    for processor in config.processors {
        processors.push(build_processor(processor)?);
    }
    Ok(PostChain {
        name: config.name,
        processors,
    })
}

pub fn build_llm_cleanup_config(config: ProcessorConfig) -> Result<LlmCleanupConfig> {
    match config {
        ProcessorConfig::Llm {
            id,
            format,
            provider_name,
            base_url,
            api_key,
            model,
            extra_body,
            system_prompt,
            prompt,
        } => Ok(LlmCleanupConfig {
            name: id,
            format: match format {
                ProviderFormatCfg::Openai => ProviderFormat::OpenAi,
                ProviderFormatCfg::Anthropic => ProviderFormat::Anthropic,
            },
            provider_name,
            base_url,
            api_key,
            model,
            extra_body,
            system_prompt,
            prompt,
        }),
        ProcessorConfig::Rule { .. } => anyhow::bail!("expected llm config"),
    }
}

fn build_processor(config: ProcessorConfig) -> Result<Box<dyn PostProcessor>> {
    match config {
        ProcessorConfig::Rule { id, patterns } => {
            let borrowed = patterns.iter().map(String::as_str).collect::<Vec<_>>();
            Ok(Box::new(ZhFilter::with_name(&id, &borrowed)))
        }
        llm @ ProcessorConfig::Llm { .. } => {
            Ok(Box::new(LlmCleanup::new(build_llm_cleanup_config(llm)?)))
        }
    }
}

#[derive(Debug, Clone)]
pub struct PipelineText {
    /// 原始 ASR 全文，整条链不变。
    #[allow(dead_code)]
    pub raw: String,
    /// 本次 recording 的 ASR session 文本列表。
    #[allow(dead_code)]
    pub segments: Vec<String>,
    /// 当前 in-flight 版本。run_chain 跑完即最终上屏文本。
    pub text: String,
}

impl PipelineText {
    /// 从 raw + 多段构造初始 PipelineText（text == raw）。
    pub fn new(raw: String, segments: Vec<String>) -> Self {
        Self {
            text: raw.clone(),
            raw,
            segments,
        }
    }
}

/// 前台 App 上下文。daemon 在 toggle OFF 时取一次，整条 pipeline 共享。
#[derive(Debug, Default, Clone)]
pub struct AppContext {
    pub bundle_id: Option<String>,
    pub app_name: Option<String>,
}

#[derive(Error, Debug)]
pub enum PostError {
    #[error("{0}")]
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PipelineStep {
    pub name: String,
    pub status: PipelineStepStatus,
    pub duration_ms: f64,
    pub text: Option<String>,
    pub error: Option<String>,
}

impl PipelineStep {
    fn ok(name: &str, elapsed: Duration, text: String) -> Self {
        Self {
            name: name.to_string(),
            status: PipelineStepStatus::Ok,
            duration_ms: elapsed.as_secs_f64() * 1000.0,
            text: Some(text),
            error: None,
        }
    }

    fn error(name: &str, elapsed: Duration, err: String) -> Self {
        Self {
            name: name.to_string(),
            status: PipelineStepStatus::Error,
            duration_ms: elapsed.as_secs_f64() * 1000.0,
            text: None,
            error: Some(err),
        }
    }

    fn timeout(name: &str, elapsed: Duration) -> Self {
        Self {
            name: name.to_string(),
            status: PipelineStepStatus::Timeout,
            duration_ms: elapsed.as_secs_f64() * 1000.0,
            text: None,
            error: Some("timeout".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineStepStatus {
    Ok,
    Error,
    Timeout,
    #[allow(dead_code)]
    Skipped,
}

#[async_trait]
pub trait PostProcessor: Send + Sync {
    fn name(&self) -> &str;
    async fn process(
        &self,
        input: PipelineText,
        ctx: &AppContext,
    ) -> Result<PipelineText, PostError>;
}

/// 跑整条链。失败/超时跳过该步，链路继续用上一步的 text；最差产出 == raw。
pub async fn run_chain(
    chain: &[Box<dyn PostProcessor>],
    initial: PipelineText,
    ctx: &AppContext,
    timeout: Duration,
) -> (PipelineText, Vec<PipelineStep>) {
    let mut current = initial;
    let mut steps = Vec::with_capacity(chain.len());
    for p in chain {
        let started = Instant::now();
        match tokio::time::timeout(timeout, p.process(current.clone(), ctx)).await {
            Ok(Ok(out)) => {
                let step = PipelineStep::ok(p.name(), started.elapsed(), out.text.clone());
                tracing::debug!(
                    step = %p.name(),
                    duration_ms = step.duration_ms,
                    "post step succeeded"
                );
                current = out;
                steps.push(step);
            }
            Ok(Err(e)) => {
                tracing::warn!(step = %p.name(), error = %e, "post step failed; skipped");
                steps.push(PipelineStep::error(
                    p.name(),
                    started.elapsed(),
                    e.to_string(),
                ));
            }
            Err(_) => {
                tracing::warn!(
                    step = %p.name(),
                    timeout_ms = timeout.as_millis(),
                    "post step timed out; skipped"
                );
                steps.push(PipelineStep::timeout(p.name(), started.elapsed()));
            }
        }
    }
    (current, steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct UpcaseProcessor;
    #[async_trait]
    impl PostProcessor for UpcaseProcessor {
        fn name(&self) -> &str {
            "upcase"
        }
        async fn process(
            &self,
            input: PipelineText,
            _ctx: &AppContext,
        ) -> Result<PipelineText, PostError> {
            Ok(PipelineText {
                text: input.text.to_uppercase(),
                ..input
            })
        }
    }

    struct FailProcessor;
    #[async_trait]
    impl PostProcessor for FailProcessor {
        fn name(&self) -> &str {
            "fail"
        }
        async fn process(
            &self,
            _input: PipelineText,
            _ctx: &AppContext,
        ) -> Result<PipelineText, PostError> {
            Err(PostError::Failed("intentional".into()))
        }
    }

    struct StallProcessor;
    #[async_trait]
    impl PostProcessor for StallProcessor {
        fn name(&self) -> &str {
            "stall"
        }
        async fn process(
            &self,
            input: PipelineText,
            _ctx: &AppContext,
        ) -> Result<PipelineText, PostError> {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(input)
        }
    }

    #[tokio::test]
    async fn empty_chain_returns_initial_unchanged() {
        let initial = PipelineText::new("hello".into(), vec!["hello".into()]);
        let (out, steps) = run_chain(
            &[],
            initial.clone(),
            &AppContext::default(),
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(out.text, "hello");
        assert_eq!(out.raw, "hello");
        assert!(steps.is_empty());
    }

    #[tokio::test]
    async fn single_processor_transforms_text() {
        let chain: Vec<Box<dyn PostProcessor>> = vec![Box::new(UpcaseProcessor)];
        let (out, steps) = run_chain(
            &chain,
            PipelineText::new("hi".into(), vec!["hi".into()]),
            &AppContext::default(),
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(out.text, "HI");
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].name, "upcase");
        assert_eq!(steps[0].status, PipelineStepStatus::Ok);
        assert_eq!(steps[0].text.as_deref(), Some("HI"));
        // raw 永远不变
        assert_eq!(out.raw, "hi");
    }

    #[tokio::test]
    async fn failed_processor_is_skipped_chain_continues() {
        let chain: Vec<Box<dyn PostProcessor>> =
            vec![Box::new(FailProcessor), Box::new(UpcaseProcessor)];
        let (out, steps) = run_chain(
            &chain,
            PipelineText::new("hi".into(), vec!["hi".into()]),
            &AppContext::default(),
            Duration::from_secs(1),
        )
        .await;
        // FailProcessor 失败 → text 留作 upstream "hi"；UpcaseProcessor 接它 → "HI"
        assert_eq!(out.text, "HI");
        assert_eq!(steps[0].status, PipelineStepStatus::Error);
        assert_eq!(steps[1].status, PipelineStepStatus::Ok);
    }

    #[tokio::test]
    async fn timed_out_processor_is_skipped() {
        let chain: Vec<Box<dyn PostProcessor>> =
            vec![Box::new(StallProcessor), Box::new(UpcaseProcessor)];
        // 真实 50ms timeout（StallProcessor 60s sleep > 50ms）→ skip → Upcase 接 "hi"
        let (out, steps) = run_chain(
            &chain,
            PipelineText::new("hi".into(), vec!["hi".into()]),
            &AppContext::default(),
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(out.text, "HI");
        assert_eq!(steps[0].status, PipelineStepStatus::Timeout);
    }

    #[tokio::test]
    async fn all_processors_failing_returns_raw() {
        let chain: Vec<Box<dyn PostProcessor>> =
            vec![Box::new(FailProcessor), Box::new(FailProcessor)];
        let (out, steps) = run_chain(
            &chain,
            PipelineText::new("raw text".into(), vec!["raw text".into()]),
            &AppContext::default(),
            Duration::from_secs(1),
        )
        .await;
        // 全失败 → text 留作 raw
        assert_eq!(out.text, "raw text");
        assert_eq!(out.raw, "raw text");
        assert_eq!(steps.len(), 2);
    }

    #[test]
    fn builds_runtime_chain_from_config_processors() {
        let chain = build_chain(crate::config::post::PostChainConfig {
            name: "rule:zh_filter → llm:deepseek".to_string(),
            processors: vec![
                crate::config::post::ProcessorConfig::Rule {
                    id: "rule:zh_filter".to_string(),
                    patterns: vec!["嗯".to_string(), "啊".to_string()],
                },
                crate::config::post::ProcessorConfig::Llm {
                    id: "llm:deepseek".to_string(),
                    format: crate::config::post::ProviderFormatCfg::Openai,
                    provider_name: "deepseek".to_string(),
                    base_url: "https://api.deepseek.com".to_string(),
                    api_key: "sk-test".to_string(),
                    model: "deepseek-chat".to_string(),
                    extra_body: serde_json::Map::new(),
                    system_prompt: None,
                    prompt: "{{text}}".to_string(),
                },
            ],
        })
        .unwrap();

        assert_eq!(chain.name, "rule:zh_filter → llm:deepseek");
        assert_eq!(chain.processors.len(), 2);
        assert_eq!(chain.processors[0].name(), "rule:zh_filter");
        assert_eq!(chain.processors[1].name(), "llm:deepseek");
    }
}
