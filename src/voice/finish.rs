//! 一次录音的完整生命周期：运行录音引擎，再统一完成 post、dispatch 和 history。

use crate::asr::types::AsrProvider;
use crate::history::{HistoryRecord, HistoryService, HistoryStatus};
use crate::overlay::{OverlayCmd, OverlayHandle, TextKind};
use crate::state::StateStore;
use crate::voice::engine::{self, EngineOutcome};
use crate::voice::history_build::{build_record, HistoryInput};
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
    history: HistoryService,
) {
    let mut completion_control_rx = control_rx.clone();
    let Some(outcome) = engine::run(provider, params, control_rx).await else {
        return;
    };
    complete_recording(outcome, &mut completion_control_rx, history).await;
}

async fn complete_recording(
    outcome: EngineOutcome,
    control_rx: &mut watch::Receiver<SessionControl>,
    history: HistoryService,
) {
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
        if crate::voice::capture::has_archivable_content(&sessions) {
            let history_result = append_history(
                history.clone(),
                HistoryInput {
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
                },
            );
            let history_result = history_result.await;
            publish_history_result(
                &params.state,
                params.overlay.as_ref(),
                &recording_id,
                history_result,
            );
        } else {
            tracing::info!(
                recording_id,
                "canceled recording had no content; skipping history"
            );
        }
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
            control_rx,
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

    let history_status = history_status_for_completion(terminal_error.as_ref(), status);
    let trace_status = match history_status {
        HistoryStatus::Submitted => "submitted",
        HistoryStatus::Canceled => "canceled",
        HistoryStatus::Empty => "empty",
        HistoryStatus::Error => "error",
        HistoryStatus::Timeout => "timeout",
    };
    let should_hide = terminal_error.is_none() && dispatch_error.is_none();
    let history_result = append_history(
        history,
        HistoryInput {
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
        },
    );
    let history_result = history_result.await;
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

async fn append_history(
    history: HistoryService,
    input: HistoryInput,
) -> anyhow::Result<HistoryRecord> {
    let record = build_record(input);
    let record_for_append = record.clone();
    tokio::task::spawn_blocking(move || history.append(record_for_append))
        .await
        .map_err(|error| anyhow::anyhow!("join history append task: {error}"))??;
    tracing::info!(
        recording_id = %record.id,
        status = ?record.status,
        provider = %record.asr.provider,
        audio_ms = record.asr.audio_ms,
        session_count = record.asr.sessions.len(),
        pipeline_steps = record.pipeline.len(),
        "recording ended"
    );
    Ok(record)
}

fn publish_history_result(
    state: &StateStore,
    overlay: Option<&OverlayHandle>,
    recording_id: &str,
    result: anyhow::Result<HistoryRecord>,
) {
    match result {
        Ok(_record) => {}
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

fn history_status_for_completion(
    terminal_error: Option<&crate::history::HistoryError>,
    status: HistoryStatus,
) -> HistoryStatus {
    match terminal_error {
        Some(error) if error.kind == "asr_timeout" => HistoryStatus::Timeout,
        Some(_) => HistoryStatus::Error,
        None => status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::HistoryError;
    use crate::voice::capture::SessionCapture;
    use crate::voice::observer::{RecordingObserver, TraceStart};
    use std::time::Instant;

    fn cancel_outcome(
        sessions: Vec<SessionCapture>,
        state: StateStore,
        overlay: OverlayHandle,
    ) -> EngineOutcome {
        let started_instant = Instant::now();
        let started_at = time::OffsetDateTime::now_utc();
        EngineOutcome {
            params: SessionParams {
                auto_paste: false,
                record_audio: crate::config::RecordAudioMode::Off,
                vad_trace: false,
                idle_pause: false,
                finalize_timeout_ms: 100,
                vad: crate::config::VoiceVadCfg::default(),
                stop_delay_ms: 0,
                hotwords: vec![],
                start_app_context: crate::post::AppContext::default(),
                post_chain: crate::post::PostChain {
                    name: "test".into(),
                    processors: vec![],
                },
                post_timeout_ms: 100,
                overlay: Some(overlay),
                state,
            },
            recording_id: "01TESTCANCEL".into(),
            recording_started_at: started_at,
            recording_started_instant: started_instant,
            app_context: crate::post::AppContext::default(),
            sessions,
            cancel_requested: true,
            terminal_error: None,
            total_audio_samples: 0,
            trace: RecordingObserver::start(TraceStart {
                enabled: false,
                recording_id: "01TESTCANCEL".into(),
                provider: "test".into(),
                started_at: started_at.to_string(),
                started_instant,
            }),
            provider_name: "test".into(),
        }
    }

    /// Contentless cancel：complete_recording 必须跳过 history append（不发
    /// history changed 事件），但仍回 Idle 并 Hide overlay。
    #[tokio::test]
    async fn contentless_cancel_skips_history_append_and_event() {
        let state = StateStore::new();
        let (_snapshot, mut state_rx) = state.subscribe_with_snapshot();
        let (overlay, mut overlay_rx) = OverlayHandle::channel();
        let outcome = cancel_outcome(Vec::new(), state.clone(), overlay);
        let (_control_tx, mut control_rx) = watch::channel(SessionControl::Idle);
        let history = HistoryService::with_dir(
            std::env::temp_dir().join(format!("shuohua-voice-history-{}", ulid::Ulid::new())),
        );
        let mut history_rx = history.subscribe();

        complete_recording(outcome, &mut control_rx, history).await;

        let mut events = Vec::new();
        while let Ok(event) = state_rx.try_recv() {
            events.push(event);
        }
        assert!(history_rx.try_recv().is_err());
        assert!(
            events
                .iter()
                .any(|e| matches!(e, crate::state::StateEvent::StateChanged { .. })),
            "cancel must still return daemon to idle"
        );

        let mut overlay_cmds = Vec::new();
        while let Ok(cmd) = overlay_rx.try_recv() {
            overlay_cmds.push(cmd);
        }
        assert!(
            overlay_cmds.iter().any(|c| matches!(c, OverlayCmd::Hide)),
            "cancel must hide the overlay: {overlay_cmds:?}"
        );
    }

    #[test]
    fn asr_finalize_timeout_is_recording_timeout() {
        let error = HistoryError {
            kind: "asr_timeout".to_string(),
            msg: "timeout waiting final".to_string(),
        };

        assert_eq!(
            history_status_for_completion(Some(&error), HistoryStatus::Submitted),
            HistoryStatus::Timeout
        );
    }

    #[test]
    fn non_timeout_terminal_error_is_recording_error() {
        let error = HistoryError {
            kind: "asr_runtime".to_string(),
            msg: "stream closed".to_string(),
        };

        assert_eq!(
            history_status_for_completion(Some(&error), HistoryStatus::Submitted),
            HistoryStatus::Error
        );
    }

    #[test]
    fn completion_without_terminal_error_preserves_status() {
        assert_eq!(
            history_status_for_completion(None, HistoryStatus::Submitted),
            HistoryStatus::Submitted
        );
        assert_eq!(
            history_status_for_completion(None, HistoryStatus::Error),
            HistoryStatus::Error
        );
    }

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
