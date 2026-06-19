//! 一次录音的完整生命周期：运行录音引擎，再统一完成 post、dispatch 和 history。

use crate::asr::types::AsrProvider;
use crate::overlay::{OverlayCmd, OverlayHandle, TextKind};
use crate::state::history::{HistoryRecord, HistoryStatus};
use crate::state::StateStore;
use crate::voice::engine::{self, EngineOutcome};
use crate::voice::history_build::{append_history, HistoryInput};
use crate::voice::observer::observe_finish;
use crate::voice::post_dispatch::dispatch_with_post_chain;
use crate::voice::SessionControl;
use tokio::sync::watch;

pub use crate::voice::engine::SessionParams;

const NOTICE_TTL_MS: u32 = 3_000;

pub async fn run_recording(
    provider: &dyn AsrProvider,
    params: SessionParams,
    control_rx: watch::Receiver<SessionControl>,
) {
    let Some(outcome) = engine::run(provider, params, control_rx).await else {
        return;
    };
    complete_recording(outcome).await;
}

async fn complete_recording(outcome: EngineOutcome) {
    let EngineOutcome {
        params,
        recording_id,
        recording_started_at,
        recording_started_instant,
        app_context,
        sessions,
        cancel_requested,
        terminal_error,
        total_audio_samples,
        mut trace,
        provider_name,
    } = outcome;
    let session_texts = crate::voice::capture::session_texts(&sessions);
    let raw_text = session_texts.concat();

    if cancel_requested {
        tracing::info!(recording_id, "recording canceled");
        params.state.set_idle();
        let history_result = append_history(HistoryInput {
            id: recording_id.clone(),
            provider: provider_name,
            started_at: recording_started_at,
            ended_at: time::OffsetDateTime::now_utc(),
            started_instant: recording_started_instant,
            asr_text: raw_text.clone(),
            final_text: raw_text,
            sessions,
            pipeline: Vec::new(),
            app: app_context.bundle_id,
            status: HistoryStatus::Canceled,
            error: None,
        });
        publish_history_result(
            &params.state,
            params.overlay.as_ref(),
            &recording_id,
            history_result,
        );
        engine::overlay_send(&params, OverlayCmd::Hide);
        observe_finish(&mut trace, "canceled", total_audio_samples);
        return;
    }

    let (final_text, pipeline, status, dispatch_error) = if terminal_error.is_some() {
        (raw_text.clone(), Vec::new(), HistoryStatus::Error, None)
    } else {
        let outcome = dispatch_with_post_chain(
            &session_texts,
            params.auto_paste,
            &app_context,
            &params.post_chain,
            params.post_timeout_ms,
            params.overlay.as_ref(),
        )
        .await;
        (
            outcome.final_text,
            outcome.pipeline,
            outcome.status,
            outcome.error,
        )
    };
    for step in &pipeline {
        params
            .state
            .pipeline_step(recording_id.clone(), step.clone());
    }

    if terminal_error.is_some() || dispatch_error.is_some() {
        params.state.set_error(Some(recording_id.clone()));
        engine::send_error_overlay(
            &params,
            if terminal_error.is_some() {
                crate::t!("error.asr_runtime")
            } else {
                crate::t!("error.dispatch")
            },
        );
    } else {
        if let Some(text) = pipeline.iter().rev().find_map(|step| step.text.clone()) {
            engine::overlay_send(
                &params,
                OverlayCmd::SetText {
                    text,
                    kind: TextKind::Final,
                },
            );
        }
        params.state.set_idle();
    }

    let history_status = terminal_error
        .as_ref()
        .map(|_| HistoryStatus::Error)
        .unwrap_or(status);
    let trace_status = if terminal_error.is_some() || dispatch_error.is_some() {
        "error"
    } else {
        match status {
            HistoryStatus::Submitted => "submitted",
            HistoryStatus::Canceled => "canceled",
            HistoryStatus::Error => "error",
            HistoryStatus::Timeout => "timeout",
        }
    };
    let should_hide = terminal_error.is_none() && dispatch_error.is_none();
    let history_result = append_history(HistoryInput {
        id: recording_id.clone(),
        provider: provider_name,
        started_at: recording_started_at,
        ended_at: time::OffsetDateTime::now_utc(),
        started_instant: recording_started_instant,
        asr_text: raw_text,
        final_text,
        sessions,
        pipeline,
        app: app_context.bundle_id,
        status: history_status,
        error: terminal_error.or(dispatch_error),
    });
    publish_history_result(
        &params.state,
        params.overlay.as_ref(),
        &recording_id,
        history_result,
    );
    if should_hide {
        engine::overlay_send(&params, OverlayCmd::Hide);
    }
    observe_finish(&mut trace, trace_status, total_audio_samples);
}

fn publish_history_result(
    state: &StateStore,
    overlay: Option<&OverlayHandle>,
    recording_id: &str,
    result: anyhow::Result<HistoryRecord>,
) {
    match result {
        Ok(record) => state.history_appended(record),
        Err(error) => {
            tracing::error!(recording_id, error = ?error, "history append failed");
            state.error(
                Some(recording_id.to_string()),
                "history_append",
                format!("{error:#}"),
            );
            if let Some(overlay) = overlay {
                overlay.send(OverlayCmd::Notice {
                    text: crate::t!("notice.history_save_failed"),
                    ttl_ms: NOTICE_TTL_MS,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_append_failure_emits_error_and_notice() {
        let state = StateStore::new();
        let (_, mut state_rx) = state.subscribe_with_snapshot();
        let (overlay, mut overlay_rx) = OverlayHandle::channel();
        publish_history_result(
            &state,
            Some(&overlay),
            "01HXYZ",
            Err(anyhow::anyhow!("disk full")),
        );
        match state_rx.try_recv().unwrap() {
            crate::state::StateEvent::Error {
                recording_id,
                kind,
                msg,
            } => {
                assert_eq!(recording_id.as_deref(), Some("01HXYZ"));
                assert_eq!(kind, "history_append");
                assert!(msg.contains("disk full"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
        match overlay_rx.try_recv().unwrap() {
            OverlayCmd::Notice { text, ttl_ms } => {
                assert_eq!(text, crate::t!("notice.history_save_failed"));
                assert_eq!(ttl_ms, NOTICE_TTL_MS);
            }
            other => panic!("unexpected overlay command: {other:?}"),
        }
    }
}
