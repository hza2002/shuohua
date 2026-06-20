//! Post chain 执行 + 剪贴板 / Cmd+V dispatch。
//!
//! Post chain 的 `Error/Timeout` 步推 overlay notice 黄字；dispatch 失败 →
//! `HistoryStatus::Error` + 红字 error；空文本 → `Empty`，跳过整条链。

use std::time::Duration;

use crate::overlay::{OverlayCmd, OverlayHandle, OverlayState};
use crate::post::{self, PipelineStepStatus, PipelineText, PostChain};
use crate::state::history::{HistoryError, HistoryStatus, PipelineStepHistory};
use crate::voice::{dispatch, SessionControl};
use tokio::sync::watch;

/// 非阻断 warn 在 meta 行上显示多久。跟 overlay 的 ERROR_TTL_MS 对齐。
const NOTICE_TTL_MS: u32 = 3000;

pub(crate) struct DispatchOutcome {
    pub final_text: String,
    pub pipeline: Vec<PipelineStepHistory>,
    pub status: HistoryStatus,
    pub error: Option<HistoryError>,
}

pub(crate) async fn dispatch_with_post_chain(
    segment_texts: &[String],
    auto_paste: bool,
    app_context: &post::AppContext,
    post_chain: &PostChain,
    post_timeout_ms: u64,
    overlay: Option<&OverlayHandle>,
    control_rx: &mut watch::Receiver<SessionControl>,
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
    let initial = PipelineText::new(raw_text.clone(), segment_texts.to_vec());
    if let Some(o) = overlay {
        o.send(OverlayCmd::SetState {
            state: OverlayState::Thinking,
        });
    }
    let post = post::run_chain(
        &post_chain.processors,
        initial,
        app_context,
        Duration::from_millis(post_timeout_ms),
    );
    tokio::pin!(post);
    let (out, steps) = tokio::select! {
        biased;
        result = &mut post => result,
        canceled = wait_for_cancel(control_rx) => {
            if canceled {
                return canceled_outcome(raw_text);
            }
            post.await
        }
    };
    for step in &steps {
        match step.status {
            PipelineStepStatus::Error | PipelineStepStatus::Timeout => {
                let text = match step.status {
                    PipelineStepStatus::Timeout => {
                        crate::t!("notice.step_timeout", name = step.name)
                    }
                    _ => crate::t!("notice.step_failed", name = step.name),
                };
                if let Some(o) = overlay {
                    o.send(OverlayCmd::Notice {
                        text,
                        ttl_ms: NOTICE_TTL_MS,
                    });
                }
            }
            PipelineStepStatus::Ok | PipelineStepStatus::Skipped => {}
        }
    }
    let dispatched = out.text.clone();
    let pipeline: Vec<PipelineStepHistory> =
        steps.into_iter().map(PipelineStepHistory::from).collect();
    if matches!(*control_rx.borrow(), SessionControl::Cancel) {
        return canceled_outcome(raw_text);
    }
    if let Err(e) = dispatch::dispatch(&out.text, auto_paste) {
        tracing::error!(error = ?e, "dispatch failed");
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

fn canceled_outcome(final_text: String) -> DispatchOutcome {
    DispatchOutcome {
        final_text,
        pipeline: Vec::new(),
        status: HistoryStatus::Canceled,
        error: None,
    }
}

async fn wait_for_cancel(control_rx: &mut watch::Receiver<SessionControl>) -> bool {
    loop {
        if matches!(*control_rx.borrow_and_update(), SessionControl::Cancel) {
            return true;
        }
        if control_rx.changed().await.is_err() {
            return false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::sync::{watch, Notify};

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

    struct CancelOnReturnProcessor {
        control_tx: watch::Sender<SessionControl>,
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
            self.control_tx.send(SessionControl::Cancel).unwrap();
            Ok(PipelineText {
                text: String::new(),
                ..input
            })
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
        let (control_tx, mut control_rx) = watch::channel(SessionControl::Idle);
        let segment_texts = vec!["hello".into()];
        let app_context = post::AppContext::default();
        let future = dispatch_with_post_chain(
            &segment_texts,
            false,
            &app_context,
            &post_chain,
            60_000,
            None,
            &mut control_rx,
        );
        tokio::pin!(future);

        tokio::select! {
            _ = started.notified() => {}
            outcome = &mut future => panic!("post completed before cancel: {:?}", outcome.status),
        }
        control_tx.send(SessionControl::Cancel).unwrap();

        let outcome = tokio::time::timeout(Duration::from_millis(100), future)
            .await
            .expect("cancel should interrupt post-processing");
        assert_eq!(outcome.status, HistoryStatus::Canceled);
        assert_eq!(outcome.final_text, "hello");
        assert!(outcome.pipeline.is_empty());
        assert!(outcome.error.is_none());
    }

    #[tokio::test]
    async fn cancel_after_post_completion_is_checked_before_dispatch() {
        let (control_tx, mut control_rx) = watch::channel(SessionControl::Idle);
        let post_chain = PostChain {
            name: "test".into(),
            processors: vec![Box::new(CancelOnReturnProcessor { control_tx })],
        };
        let segment_texts = vec!["hello".into()];
        let outcome = dispatch_with_post_chain(
            &segment_texts,
            false,
            &post::AppContext::default(),
            &post_chain,
            1_000,
            None,
            &mut control_rx,
        )
        .await;

        assert_eq!(outcome.status, HistoryStatus::Canceled);
        assert_eq!(outcome.final_text, "hello");
        assert!(outcome.pipeline.is_empty());
        assert!(outcome.error.is_none());
    }

    #[tokio::test]
    async fn empty_asr_text_is_not_user_cancel() {
        let (_control_tx, mut control_rx) = watch::channel(SessionControl::Idle);
        let post_chain = PostChain {
            name: "test".into(),
            processors: Vec::new(),
        };

        let outcome = dispatch_with_post_chain(
            &[],
            false,
            &post::AppContext::default(),
            &post_chain,
            1_000,
            None,
            &mut control_rx,
        )
        .await;

        assert_eq!(outcome.status, HistoryStatus::Empty);
        assert_eq!(outcome.final_text, "");
        assert!(outcome.pipeline.is_empty());
        assert!(outcome.error.is_none());
    }
}
