//! 文本后处理流水线（DESIGN §2.10）。
//!
//! M2.5 只装一个内置 processor [`RuleBasedFiller`]（去口语词）。M7 再加 LLM 清洗
//! 和 per-app 链路配置。
//!
//! 数据契约：
//! - [`PipelineText`] 流过整条链。`raw` 永远是原始 ASR 文本；`text` 是当前
//!   in-flight 版本；`segments` 是多 ASR session 的原始段（M2.5.d2 引入），M3
//!   history 直接消费。
//! - run_chain "链不阻塞"：单步 processor 失败/超时**跳过**，下一个继续用上一步
//!   的 text。最差产出 == raw（不会丢内容）。
//! - M2.5 不接 toast / step_tx；M3 加 overlay 时再扩参数。失败/超时走诊断日志。

pub mod app_context;
pub mod config;
pub mod filler;
pub mod llm;

use async_trait::async_trait;
use std::time::{Duration, Instant};
use thiserror::Error;

pub use filler::RuleBasedFiller;

#[derive(Debug, Clone)]
pub struct PipelineText {
    /// 原始 ASR 全文，整条链不变。
    pub raw: String,
    /// 多段 ASR session 的原始文本列表（M2.5.d2 之后才会出现 >1 项）。
    /// M3 history.asr.sessions 直接消费这个 Vec。
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
}
