//! 一次录音的完整生命周期：运行录音引擎，再统一完成 post、dispatch 和 history。

use crate::asr::types::AsrProvider;
use crate::history::{HistoryRecord, HistoryService, HistoryStatus};
use crate::overlay::{OverlayCmd, OverlayHandle};
use crate::state::StateStore;
use crate::voice::capture::{SegmentCapture, SessionCapture};
use crate::voice::engine::{self, EngineOutcome};
use crate::voice::history_build::{build_record, HistoryInput};
use crate::voice::observer::observe_finish;
use crate::voice::post_dispatch::{dispatch_with_post_chain, DispatchContext};
use crate::voice::SessionControl;

pub use crate::voice::engine::SessionParams;

const NOTICE_TTL_MS: u32 = 3_000;

pub async fn run_recording(
    provider: &dyn AsrProvider,
    params: SessionParams,
    control: SessionControl,
    history: HistoryService,
) {
    let completion_control = control.clone();
    let Some(outcome) = engine::run(provider, params, control).await else {
        return;
    };
    complete_recording(outcome, &completion_control, history).await;
}

async fn complete_recording(
    outcome: EngineOutcome,
    control: &SessionControl,
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
    let real_session_texts = crate::voice::capture::session_texts(&sessions);
    let has_new_asr_text = !real_session_texts.is_empty();
    // resume 录音只有音频没新文本时不算有内容：不写记录，避免盖掉想续写的可恢复
    // 记录（见 voice.md）。engine 的 retained audio 判定同源，保持一致。
    let has_real_content =
        crate::voice::capture::has_archivable_content_for(&sessions, params.start.is_seed());
    // seed 只在本次 recording 真有新 ASR 文本时才参与 history/post（见 voice.md）。
    let seed_text = params
        .start
        .seed()
        .and_then(|seed| seed.non_empty_text())
        .filter(|_| has_new_asr_text);
    let mut effective_session_texts = Vec::new();
    if let Some(text) = seed_text {
        effective_session_texts.push(text.to_string());
    }
    effective_session_texts.extend(real_session_texts);
    let effective_raw_text = effective_session_texts.concat();

    if cancel_requested {
        tracing::info!(recording_id, "recording canceled");
        params.state.set_idle();
        if has_real_content {
            let history_sessions =
                sessions_with_resume_seed(seed_text.map(str::to_string), sessions);
            let history_result = append_history(
                history.clone(),
                HistoryInput {
                    id: recording_id.clone(),
                    provider: provider_name,
                    started_at: recording_started_at,
                    ended_at: time::OffsetDateTime::now_utc(),
                    started_instant: recording_started_instant,
                    asr_text: effective_raw_text.clone(),
                    final_text: effective_raw_text,
                    sessions: history_sessions,
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

    if !has_real_content {
        tracing::info!(
            recording_id,
            "recording had no content; skipping post and history"
        );
        let trace_status = if terminal_error.as_ref().is_some() {
            params.state.set_error(Some(recording_id.clone()));
            engine::send_error_overlay(&params, crate::t!("error.asr_runtime"));
            "empty_error"
        } else {
            params.state.set_idle();
            engine::overlay_send(&params, OverlayCmd::Hide);
            "empty"
        };
        observe_finish(&mut trace, trace_status, total_audio_samples);
        return;
    }

    let (final_text, pipeline, status, dispatch_error) = if terminal_error.is_some() {
        (
            effective_raw_text.clone(),
            Vec::new(),
            HistoryStatus::Error,
            None,
        )
    } else {
        let outcome = dispatch_with_post_chain(
            &effective_session_texts,
            params.auto_paste,
            &params.post_chain,
            params.post_timeout_ms,
            DispatchContext {
                recording_id: &recording_id,
                app_context: &app_context,
                overlay: params.overlay.as_ref(),
                cancel: control.cancel_signal(),
            },
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
        // 不把 LLM/post 输出再推回 overlay：overlay 只镜像 ASR 原文（已在交给 LLM 那一刻
        // 显示完整），post 结果直接上屏（粘贴），无需二次展示。成功后直接 Hide。
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
    let history_sessions = sessions_with_resume_seed(seed_text.map(str::to_string), sessions);
    let history_result = append_history(
        history,
        HistoryInput {
            id: recording_id.clone(),
            provider: provider_name,
            started_at: recording_started_at,
            ended_at: time::OffsetDateTime::now_utc(),
            started_instant: recording_started_instant,
            asr_text: effective_raw_text,
            final_text,
            sessions: history_sessions,
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

fn sessions_with_resume_seed(
    seed_text: Option<String>,
    sessions: Vec<SessionCapture>,
) -> Vec<SessionCapture> {
    let Some(seed_text) = seed_text.filter(|text| !text.trim().is_empty()) else {
        return sessions;
    };
    let Some(seed_instant) = sessions.first().map(|session| session.started_at) else {
        return sessions;
    };
    let mut merged = Vec::with_capacity(sessions.len() + 1);
    merged.push(SessionCapture {
        started_at: seed_instant,
        ended_at: seed_instant,
        audio_samples: 0,
        segments: vec![SegmentCapture {
            text: seed_text,
            started_at: seed_instant,
            ended_at: seed_instant,
        }],
        final_text: None,
        partial_text: None,
    });
    merged.extend(sessions);
    merged
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
    use crate::post::{PipelineText, PostError};
    use crate::voice::capture::SegmentCapture;
    use crate::voice::capture::SessionCapture;
    use crate::voice::observer::{RecordingObserver, TraceStart};
    use crate::voice::resume::ResumeSeed;
    use async_trait::async_trait;
    use std::time::{Duration, Instant};

    fn test_outcome(
        sessions: Vec<SessionCapture>,
        state: StateStore,
        overlay: OverlayHandle,
        cancel_requested: bool,
    ) -> EngineOutcome {
        let started_instant = Instant::now();
        let started_at = time::OffsetDateTime::now_utc();
        EngineOutcome {
            params: SessionParams {
                auto_paste: false,
                record_audio: crate::config::RecordAudioMode::Off,
                preprocess: crate::config::VoicePreprocessCfg::default(),
                vad_trace: false,
                apple_backend_trace: false,
                idle_pause: false,
                open_timeout_ms: 100,
                finalize_timeout_ms: 100,
                vad: crate::config::VoiceVadCfg::default(),
                stop_delay_ms: 0,
                hotwords: vec![],
                start_app_context: crate::post::AppContext::default(),
                profile_name: "test".into(),
                profile_choices: vec![crate::overlay::ProfileChoice::test("test")],
                post_chain: crate::post::PostChain {
                    name: "test".into(),
                    processors: vec![],
                },
                post_timeout_ms: 100,
                start: crate::voice::resume::RecordingStart::Fresh,
                overlay: Some(overlay),
                state,
            },
            recording_id: "01TESTCANCEL".into(),
            recording_started_at: started_at,
            recording_started_instant: started_instant,
            app_context: crate::post::AppContext::default(),
            sessions,
            cancel_requested,
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

    fn cancel_outcome(
        sessions: Vec<SessionCapture>,
        state: StateStore,
        overlay: OverlayHandle,
    ) -> EngineOutcome {
        test_outcome(sessions, state, overlay, true)
    }

    fn success_outcome(
        sessions: Vec<SessionCapture>,
        state: StateStore,
        overlay: OverlayHandle,
    ) -> EngineOutcome {
        test_outcome(sessions, state, overlay, false)
    }

    fn error_outcome(
        sessions: Vec<SessionCapture>,
        state: StateStore,
        overlay: OverlayHandle,
    ) -> EngineOutcome {
        let mut outcome = test_outcome(sessions, state, overlay, false);
        outcome.terminal_error = Some(HistoryError {
            kind: "capture".to_string(),
            msg: "scripted capture failure".to_string(),
        });
        outcome
    }

    struct PanicProcessor;

    #[async_trait]
    impl crate::post::PostProcessor for PanicProcessor {
        fn name(&self) -> &str {
            "panic"
        }

        async fn process(
            &self,
            _input: PipelineText,
            _ctx: &crate::post::AppContext,
        ) -> std::result::Result<PipelineText, PostError> {
            panic!("seed-only resume must not run post processors");
        }
    }

    struct AppendProcessor {
        suffix: &'static str,
    }

    #[async_trait]
    impl crate::post::PostProcessor for AppendProcessor {
        fn name(&self) -> &str {
            "append"
        }

        async fn process(
            &self,
            mut input: PipelineText,
            _ctx: &crate::post::AppContext,
        ) -> std::result::Result<PipelineText, PostError> {
            input.text.push_str(self.suffix);
            Ok(input)
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
        let control = SessionControl::new();
        let history = HistoryService::with_dir(
            std::env::temp_dir().join(format!("shuohua-voice-history-{}", ulid::Ulid::new())),
        );
        let mut history_rx = history.subscribe();

        complete_recording(outcome, &control, history).await;

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

    /// Contentless success：没有可归档内容时 complete_recording 不写 history
    /// 或发送 history changed 事件，并回 Idle/Hide overlay。
    #[tokio::test]
    async fn contentless_success_skips_history_append_and_event() {
        let state = StateStore::new();
        let (_snapshot, mut state_rx) = state.subscribe_with_snapshot();
        let (overlay, mut overlay_rx) = OverlayHandle::channel();
        let outcome = success_outcome(Vec::new(), state.clone(), overlay);
        let control = SessionControl::new();
        let history = HistoryService::with_dir(
            std::env::temp_dir().join(format!("shuohua-voice-history-{}", ulid::Ulid::new())),
        );
        let mut history_rx = history.subscribe();

        complete_recording(outcome, &control, history).await;

        let mut events = Vec::new();
        while let Ok(event) = state_rx.try_recv() {
            events.push(event);
        }
        assert!(history_rx.try_recv().is_err());
        assert!(
            events
                .iter()
                .any(|e| matches!(e, crate::state::StateEvent::StateChanged { .. })),
            "contentless success must still return daemon to idle"
        );

        let mut overlay_cmds = Vec::new();
        while let Ok(cmd) = overlay_rx.try_recv() {
            overlay_cmds.push(cmd);
        }
        assert!(
            overlay_cmds.iter().any(|c| matches!(c, OverlayCmd::Hide)),
            "contentless success must hide the overlay: {overlay_cmds:?}"
        );
    }

    #[tokio::test]
    async fn resume_seed_without_new_recording_content_does_not_run_post_or_append() {
        let (overlay, _rx) = OverlayHandle::channel();
        let state = StateStore::new();
        let history = HistoryService::with_dir(
            std::env::temp_dir().join(format!("shuohua-test-{}", ulid::Ulid::new())),
        );
        let mut outcome = success_outcome(Vec::new(), state, overlay);
        outcome.params.post_chain.processors = vec![Box::new(PanicProcessor)];
        outcome.params.start = crate::voice::resume::RecordingStart::Seed(ResumeSeed {
            text: "old text".to_string(),
        });

        complete_recording(outcome, &SessionControl::new(), history.clone()).await;

        let page = history
            .page(crate::history::HistoryQuery {
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert!(page.records.is_empty());
    }

    #[tokio::test]
    async fn resume_seed_cancel_without_new_recording_content_does_not_append() {
        let (overlay, _rx) = OverlayHandle::channel();
        let state = StateStore::new();
        let history = HistoryService::with_dir(
            std::env::temp_dir().join(format!("shuohua-test-{}", ulid::Ulid::new())),
        );
        let mut outcome = cancel_outcome(Vec::new(), state, overlay);
        outcome.params.start = crate::voice::resume::RecordingStart::Seed(ResumeSeed {
            text: "old text".to_string(),
        });

        complete_recording(outcome, &SessionControl::new(), history.clone()).await;

        let page = history
            .page(crate::history::HistoryQuery {
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert!(page.records.is_empty());
    }

    /// Resume 抓到音频但 ASR 没识别出新文本（如环境噪音）：不写 history，避免盖
    /// 掉它想续写的那条可恢复记录，也不复用 seed 跑 post。
    #[tokio::test]
    async fn resume_audio_without_new_asr_text_skips_history_to_preserve_recoverable() {
        let (overlay, _rx) = OverlayHandle::channel();
        let state = StateStore::new();
        let history = HistoryService::with_dir(
            std::env::temp_dir().join(format!("shuohua-test-{}", ulid::Ulid::new())),
        );
        let base = Instant::now();
        let sessions = vec![SessionCapture {
            started_at: base + Duration::from_millis(500),
            ended_at: base + Duration::from_millis(1_500),
            audio_samples: 16_000,
            segments: vec![],
            final_text: None,
            partial_text: None,
        }];
        let mut outcome = success_outcome(sessions, state, overlay);
        outcome.recording_started_instant = base;
        outcome.params.post_chain.processors = vec![Box::new(PanicProcessor)];
        outcome.params.start = crate::voice::resume::RecordingStart::Seed(ResumeSeed {
            text: "old text".to_string(),
        });

        complete_recording(outcome, &SessionControl::new(), history.clone()).await;

        let page = history
            .page(crate::history::HistoryQuery {
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert!(
            page.records.is_empty(),
            "resume with audio-but-no-new-text must not append a record"
        );
    }

    #[tokio::test]
    async fn resume_success_after_new_content_appends_seed_plus_new_asr() {
        let (overlay, _rx) = OverlayHandle::channel();
        let state = StateStore::new();
        let history = HistoryService::with_dir(
            std::env::temp_dir().join(format!("shuohua-test-{}", ulid::Ulid::new())),
        );
        let base = Instant::now();
        let sessions = vec![SessionCapture {
            started_at: base + Duration::from_millis(500),
            ended_at: base + Duration::from_millis(1_500),
            audio_samples: 16_000,
            segments: vec![SegmentCapture {
                text: "new text".to_string(),
                started_at: base + Duration::from_millis(500),
                ended_at: base + Duration::from_millis(1_500),
            }],
            final_text: None,
            partial_text: None,
        }];
        let mut outcome = success_outcome(sessions, state, overlay);
        outcome.recording_started_instant = base;
        outcome.params.post_chain.processors = vec![Box::new(AppendProcessor { suffix: "." })];
        outcome.params.start = crate::voice::resume::RecordingStart::Seed(ResumeSeed {
            text: "old text ".to_string(),
        });

        complete_recording(outcome, &SessionControl::new(), history.clone()).await;

        let page = history
            .page(crate::history::HistoryQuery {
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.records.len(), 1);
        let record = &page.records[0];
        assert_eq!(record.status, HistoryStatus::Submitted);
        assert_eq!(record.asr.text, "old text new text");
        assert_eq!(record.text, "old text new text.");
        assert_eq!(record.pipeline.len(), 1);
        assert_eq!(
            record.pipeline[0].text.as_deref(),
            Some("old text new text.")
        );
        assert_eq!(record.asr.sessions.len(), 2);
        assert_eq!(record.asr.sessions[0].audio_ms, 0);
        assert_eq!(record.asr.audio_ms, 1_000);
    }

    #[tokio::test]
    async fn resume_cancel_after_new_content_appends_canceled_seed_plus_new_asr() {
        let (overlay, _rx) = OverlayHandle::channel();
        let state = StateStore::new();
        let history = HistoryService::with_dir(
            std::env::temp_dir().join(format!("shuohua-test-{}", ulid::Ulid::new())),
        );
        let base = Instant::now();
        let sessions = vec![SessionCapture {
            started_at: base + Duration::from_millis(500),
            ended_at: base + Duration::from_millis(1_500),
            audio_samples: 16_000,
            segments: vec![SegmentCapture {
                text: "new text".to_string(),
                started_at: base + Duration::from_millis(500),
                ended_at: base + Duration::from_millis(1_500),
            }],
            final_text: None,
            partial_text: None,
        }];
        let mut outcome = cancel_outcome(sessions, state, overlay);
        outcome.recording_started_instant = base;
        outcome.params.start = crate::voice::resume::RecordingStart::Seed(ResumeSeed {
            text: "old text ".to_string(),
        });

        complete_recording(outcome, &SessionControl::new(), history.clone()).await;

        let page = history
            .page(crate::history::HistoryQuery {
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.records.len(), 1);
        let record = &page.records[0];
        assert_eq!(record.status, HistoryStatus::Canceled);
        assert_eq!(record.asr.text, "old text new text");
        assert_eq!(record.text, "old text new text");
        assert_eq!(record.asr.sessions.len(), 2);
        assert_eq!(record.asr.sessions[0].audio_ms, 0);
        assert_eq!(record.asr.audio_ms, 1_000);
    }

    /// Contentless terminal error：没有 provider audio/text 时仍不写 history，
    /// 避免产生没有实际录音内容的错误记录。
    #[tokio::test]
    async fn contentless_terminal_error_skips_history_append_and_event() {
        let state = StateStore::new();
        let (_snapshot, mut state_rx) = state.subscribe_with_snapshot();
        let (overlay, mut overlay_rx) = OverlayHandle::channel();
        let outcome = error_outcome(Vec::new(), state.clone(), overlay);
        let control = SessionControl::new();
        let history = HistoryService::with_dir(
            std::env::temp_dir().join(format!("shuohua-voice-history-{}", ulid::Ulid::new())),
        );
        let mut history_rx = history.subscribe();

        complete_recording(outcome, &control, history).await;

        while state_rx.try_recv().is_ok() {}
        assert!(history_rx.try_recv().is_err());
        assert!(
            matches!(state.snapshot().state, crate::state::DaemonState::Error),
            "contentless terminal error must leave daemon in error state"
        );

        let mut overlay_cmds = Vec::new();
        while let Ok(cmd) = overlay_rx.try_recv() {
            overlay_cmds.push(cmd);
        }
        assert!(
            overlay_cmds.iter().any(|c| matches!(
                c,
                OverlayCmd::SetState {
                    state: crate::overlay::OverlayState::Error
                }
            )),
            "contentless terminal error must show error overlay: {overlay_cmds:?}"
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
