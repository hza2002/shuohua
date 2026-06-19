//! Post chain 执行 + 剪贴板 / Cmd+V dispatch。
//!
//! Post chain 的 `Error/Timeout` 步推 overlay notice 黄字；dispatch 失败 →
//! `HistoryStatus::Error` + 红字 error；空文本 → `Canceled`，跳过整条链。

use std::time::Duration;

use crate::overlay::{OverlayCmd, OverlayHandle, OverlayState};
use crate::post::{self, PipelineStepStatus, PipelineText, PostChain};
use crate::state::history::{HistoryError, HistoryStatus, PipelineStepHistory};
use crate::voice::dispatch;

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
) -> DispatchOutcome {
    let raw_text: String = segment_texts.concat();
    if raw_text.is_empty() {
        return DispatchOutcome {
            final_text: String::new(),
            pipeline: Vec::new(),
            status: HistoryStatus::Canceled,
            error: None,
        };
    }
    let initial = PipelineText::new(raw_text, segment_texts.to_vec());
    if let Some(o) = overlay {
        o.send(OverlayCmd::SetState {
            state: OverlayState::Thinking,
        });
    }
    let (out, steps) = post::run_chain(
        &post_chain.processors,
        initial,
        app_context,
        Duration::from_millis(post_timeout_ms),
    )
    .await;
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
