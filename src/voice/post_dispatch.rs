//! Post chain 执行 + 剪贴板 / Cmd+V dispatch。
//!
//! Post chain 的 `Error/Timeout` 步推 overlay notice 黄字；dispatch 失败 →
//! `HistoryStatus::Error` + 红字 error；空文本 → `Empty`，跳过整条链。

use std::time::Duration;

use crate::history::{HistoryError, HistoryStatus, PipelineStepHistory};
use crate::overlay::{OverlayCmd, OverlayHandle, OverlayState};
use crate::post::{self, PipelineStepStatus, PipelineText, PostChain};
use crate::voice::dispatch;
use crate::voice::CancelSignal;

/// 非阻断 warn 在 meta 行上显示多久。跟 overlay 的 ERROR_TTL_MS 对齐。
const NOTICE_TTL_MS: u32 = 3000;

pub(crate) struct DispatchOutcome {
    pub final_text: String,
    pub pipeline: Vec<PipelineStepHistory>,
    pub status: HistoryStatus,
    pub error: Option<HistoryError>,
}

pub(crate) struct DispatchContext<'a> {
    pub recording_id: &'a str,
    pub app_context: &'a post::AppContext,
    pub overlay: Option<&'a OverlayHandle>,
    pub cancel: CancelSignal<'a>,
}

pub(crate) async fn dispatch_with_post_chain(
    segment_texts: &[String],
    auto_paste: bool,
    post_chain: &PostChain,
    post_timeout_ms: u64,
    ctx: DispatchContext<'_>,
) -> DispatchOutcome {
    let raw_text: String = segment_texts.concat();
    if raw_text.is_empty() {
        return DispatchOutcome {
            final_text: String::new(),
            pipeline: Vec::new(),
            status: HistoryStatus::Empty,
            error: None,
        };
    }
    let mut current = PipelineText::new(raw_text.clone(), segment_texts.to_vec());
    let mut steps = Vec::with_capacity(post_chain.processors.len());
    let timeout = Duration::from_millis(post_timeout_ms);
    if let Some(o) = ctx.overlay {
        o.send(OverlayCmd::SetState {
            state: OverlayState::Thinking,
        });
    }

    for processor in &post_chain.processors {
        if ctx.cancel.is_cancelled() {
            return canceled_outcome(raw_text, steps);
        }

        let step_fut = post::run_step(
            ctx.recording_id,
            processor.as_ref(),
            current,
            ctx.app_context,
            timeout,
        );
        tokio::pin!(step_fut);
        let (step, next) = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => {
                return canceled_outcome(raw_text, steps);
            }
            out = &mut step_fut => out,
        };
        current = next;

        match step.status {
            PipelineStepStatus::Error | PipelineStepStatus::Timeout => {
                let text = post_step_notice_text(&step);
                if let Some(o) = ctx.overlay {
                    o.send(OverlayCmd::Notice {
                        text,
                        ttl_ms: NOTICE_TTL_MS,
                    });
                }
            }
            PipelineStepStatus::Ok | PipelineStepStatus::Skipped => {}
        }
        steps.push(step);
    }

    let dispatched = current.text.clone();
    let pipeline: Vec<PipelineStepHistory> =
        steps.into_iter().map(PipelineStepHistory::from).collect();
    if ctx.cancel.is_cancelled() {
        return DispatchOutcome {
            final_text: raw_text,
            pipeline,
            status: HistoryStatus::Canceled,
            error: None,
        };
    }
    if let Err(e) = dispatch::dispatch(ctx.recording_id, &current.text, auto_paste) {
        tracing::error!(
            recording_id = ctx.recording_id,
            auto_paste,
            pipeline_steps = pipeline.len(),
            error = ?e,
            "dispatch failed"
        );
        return DispatchOutcome {
            final_text: dispatched,
            pipeline,
            status: HistoryStatus::Error,
            error: Some(HistoryError {
                kind: "dispatch".to_string(),
                msg: format!("{e:#}"),
            }),
        };
    }
    DispatchOutcome {
        final_text: dispatched,
        pipeline,
        status: HistoryStatus::Submitted,
        error: None,
    }
}

fn post_step_notice_text(step: &post::PipelineStep) -> String {
    match step.status {
        PipelineStepStatus::Timeout => crate::t!("notice.step_timeout", name = step.name),
        PipelineStepStatus::Error => {
            // Only LLM steps carry a failure_reason (rule processors never Err);
            // show the provider-specific reason, else the generic step failure.
            if let Some(reason) = step.failure_reason {
                let reason = crate::i18n::tr(reason.i18n_key(), &[]);
                crate::i18n::tr("notice.llm_step_failed", &[("reason", reason)])
            } else {
                crate::t!("notice.step_failed", name = step.name)
            }
        }
        PipelineStepStatus::Ok | PipelineStepStatus::Skipped => String::new(),
    }
}

fn canceled_outcome(final_text: String, steps: Vec<post::PipelineStep>) -> DispatchOutcome {
    DispatchOutcome {
        final_text,
        pipeline: steps.into_iter().map(PipelineStepHistory::from).collect(),
        status: HistoryStatus::Canceled,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;

    use crate::post::{PostError, PostProcessor};
    struct BlockingProcessor {
        started: Arc<Notify>,
    }

    #[async_trait]
    impl PostProcessor for BlockingProcessor {
        fn name(&self) -> &str {
            "blocking"
        }

        async fn process(
            &self,
            _input: PipelineText,
            _ctx: &post::AppContext,
        ) -> Result<PipelineText, PostError> {
            self.started.notify_one();
            std::future::pending().await
        }
    }

    struct AppendProcessor {
        name: &'static str,
        suffix: &'static str,
    }

    #[async_trait]
    impl PostProcessor for AppendProcessor {
        fn name(&self) -> &str {
            self.name
        }

        async fn process(
            &self,
            mut input: PipelineText,
            _ctx: &post::AppContext,
        ) -> Result<PipelineText, PostError> {
            input.text.push_str(self.suffix);
            Ok(input)
        }
    }

    struct CancelOnReturnProcessor {
        cancel: CancellationToken,
    }

    #[async_trait]
    impl PostProcessor for CancelOnReturnProcessor {
        fn name(&self) -> &str {
            "cancel-on-return"
        }

        async fn process(
            &self,
            input: PipelineText,
            _ctx: &post::AppContext,
        ) -> Result<PipelineText, PostError> {
            self.cancel.cancel();
            Ok(PipelineText {
                text: String::new(),
                ..input
            })
        }
    }

    struct FailingProcessor;

    #[async_trait]
    impl PostProcessor for FailingProcessor {
        fn name(&self) -> &str {
            "broken"
        }

        async fn process(
            &self,
            _input: PipelineText,
            _ctx: &post::AppContext,
        ) -> Result<PipelineText, PostError> {
            Err(PostError::failed_with_reason(
                crate::post::PostFailureReason::ModelNotFound,
                "broken (custom, openai) http error 404; error code=model_not_found message=verbose provider details",
            ))
        }
    }

    #[tokio::test]
    async fn cancel_during_post_prevents_dispatch() {
        let started = Arc::new(Notify::new());
        let post_chain = PostChain {
            name: "test".into(),
            processors: vec![Box::new(BlockingProcessor {
                started: Arc::clone(&started),
            })],
        };
        let cancel = CancellationToken::new();
        let segment_texts = vec!["hello".into()];
        let app_context = post::AppContext::default();
        let future = dispatch_with_post_chain(
            &segment_texts,
            false,
            &post_chain,
            60_000,
            DispatchContext {
                recording_id: "test-recording",
                app_context: &app_context,
                overlay: None,
                cancel: CancelSignal::new(&cancel),
            },
        );
        tokio::pin!(future);

        tokio::select! {
            _ = started.notified() => {}
            outcome = &mut future => panic!("post completed before cancel: {:?}", outcome.status),
        }
        cancel.cancel();

        let outcome = tokio::time::timeout(Duration::from_millis(100), future)
            .await
            .expect("cancel should interrupt post-processing");
        assert_eq!(outcome.status, HistoryStatus::Canceled);
        assert_eq!(outcome.final_text, "hello");
        assert!(outcome.pipeline.is_empty());
        assert!(outcome.error.is_none());
    }

    #[tokio::test]
    async fn cancel_during_later_post_preserves_completed_steps() {
        let started = Arc::new(Notify::new());
        let post_chain = PostChain {
            name: "test".into(),
            processors: vec![
                Box::new(AppendProcessor {
                    name: "first",
                    suffix: " world",
                }),
                Box::new(BlockingProcessor {
                    started: Arc::clone(&started),
                }),
            ],
        };
        let cancel = CancellationToken::new();
        let segment_texts = vec!["hello".into()];
        let app_context = post::AppContext::default();
        let future = dispatch_with_post_chain(
            &segment_texts,
            false,
            &post_chain,
            60_000,
            DispatchContext {
                recording_id: "test-recording",
                app_context: &app_context,
                overlay: None,
                cancel: CancelSignal::new(&cancel),
            },
        );
        tokio::pin!(future);

        tokio::select! {
            _ = started.notified() => {}
            outcome = &mut future => panic!("post completed before cancel: {:?}", outcome.status),
        }
        cancel.cancel();

        let outcome = tokio::time::timeout(Duration::from_millis(100), future)
            .await
            .expect("cancel should interrupt post-processing");
        assert_eq!(outcome.status, HistoryStatus::Canceled);
        assert_eq!(outcome.final_text, "hello");
        assert_eq!(outcome.pipeline.len(), 1);
        assert_eq!(outcome.pipeline[0].name, "first");
        assert_eq!(
            outcome.pipeline[0].status,
            crate::history::PipelineStepStatus::Ok
        );
        assert_eq!(outcome.pipeline[0].text.as_deref(), Some("hello world"));
        assert!(outcome.error.is_none());
    }

    #[tokio::test]
    async fn cancel_after_post_completion_is_checked_before_dispatch() {
        let cancel = CancellationToken::new();
        let post_chain = PostChain {
            name: "test".into(),
            processors: vec![Box::new(CancelOnReturnProcessor {
                cancel: cancel.clone(),
            })],
        };
        let segment_texts = vec!["hello".into()];
        let app_context = post::AppContext::default();
        let outcome = dispatch_with_post_chain(
            &segment_texts,
            false,
            &post_chain,
            1_000,
            DispatchContext {
                recording_id: "test-recording",
                app_context: &app_context,
                overlay: None,
                cancel: CancelSignal::new(&cancel),
            },
        )
        .await;

        assert_eq!(outcome.status, HistoryStatus::Canceled);
        assert_eq!(outcome.final_text, "hello");
        // Canceled pipeline steps are observation data only; top-level text
        // remains raw ASR and is not derived from the completed post step.
        assert_eq!(outcome.pipeline.len(), 1);
        assert_eq!(outcome.pipeline[0].name, "cancel-on-return");
        assert_eq!(
            outcome.pipeline[0].status,
            crate::history::PipelineStepStatus::Ok
        );
        assert_eq!(outcome.pipeline[0].text.as_deref(), Some(""));
        assert!(outcome.error.is_none());
    }

    #[tokio::test]
    async fn empty_asr_text_is_not_user_cancel() {
        let cancel = CancellationToken::new();
        let post_chain = PostChain {
            name: "test".into(),
            processors: Vec::new(),
        };
        let app_context = post::AppContext::default();

        let outcome = dispatch_with_post_chain(
            &[],
            false,
            &post_chain,
            1_000,
            DispatchContext {
                recording_id: "test-recording",
                app_context: &app_context,
                overlay: None,
                cancel: CancelSignal::new(&cancel),
            },
        )
        .await;

        assert_eq!(outcome.status, HistoryStatus::Empty);
        assert_eq!(outcome.final_text, "");
        assert!(outcome.pipeline.is_empty());
        assert!(outcome.error.is_none());
    }

    #[tokio::test]
    async fn post_failure_notice_uses_short_reason_not_provider_message() {
        crate::i18n::init("en-US");
        let cancel = CancellationToken::new();
        let post_chain = PostChain {
            name: "test".into(),
            processors: vec![Box::new(FailingProcessor)],
        };
        let (overlay, mut overlay_rx) = OverlayHandle::channel();
        let app_context = post::AppContext::default();

        let outcome = dispatch_with_post_chain(
            &["hello".to_string()],
            false,
            &post_chain,
            1_000,
            DispatchContext {
                recording_id: "test-recording",
                app_context: &app_context,
                overlay: Some(&overlay),
                cancel: CancelSignal::new(&cancel),
            },
        )
        .await;

        assert_eq!(outcome.status, HistoryStatus::Submitted);
        assert_eq!(outcome.final_text, "hello");
        let mut notices = Vec::new();
        while let Ok(cmd) = overlay_rx.try_recv() {
            if let OverlayCmd::Notice { text, .. } = cmd {
                notices.push(text);
            }
        }
        assert_eq!(notices, ["LLM skipped: model not found"]);
    }
}
