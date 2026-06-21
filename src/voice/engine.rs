//! 录音期间的统一 Active / Idle 引擎。
//!
//! `Continuous` 和 `VadPause` 只在 PCM 路由与 session 切换上不同。本模块负责
//! recorder/provider 初始化、ASR event、stop drain、provider finalize、
//! Active/Idle、错误/取消和 retained audio，完成后返回 [`EngineOutcome`]。

use std::time::{Duration, Instant};

use crate::asr::types::{AsrEvent, AsrProvider, AsrSession, LanguageMode, SessionCtx};
use crate::overlay::{OverlayCmd, OverlayHandle, OverlayState, TextKind};
use crate::post;
use crate::state::history::HistoryError;
use crate::state::{SessionMeta, SessionPhase as UiSessionPhase, StateStore};
use crate::voice::capture::{samples_to_ms, SegmentCapture, SessionCapture};
use crate::voice::finalize::{finalize_provider_session, FinalizeOutcome};
use crate::voice::meter::MeterCollector;
use crate::voice::observer::{
    instant_elapsed_ms, observe_asr_error, observe_asr_event, observe_finish, observe_finish_ms,
    observe_pcm, observe_provider_opened, observe_session, RecordingObserver, SessionPhase,
    TraceStart,
};
use crate::voice::{audio, recorder, SessionControl};
use tokio::sync::{mpsc, watch};
use tokio::time::{sleep_until, Instant as TokioInstant};

const FIRST_AUDIO_TIMEOUT_MS: u64 = 1000;
const MIN_NONZERO_AMPLITUDE: i16 = 8;
const NOTICE_TTL_MS: u32 = 3_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordingMode {
    Continuous,
    VadPause,
}

impl RecordingMode {
    pub(crate) fn select(idle_pause: bool, vad: &crate::config::VoiceVadCfg) -> Self {
        if idle_pause && matches!(vad.backend, crate::config::VoiceVadBackend::Silero) {
            Self::VadPause
        } else {
            Self::Continuous
        }
    }
}

type OpenedSession = (Box<dyn AsrSession>, mpsc::Receiver<AsrEvent>);

struct SessionOpener<'a> {
    provider: &'a dyn AsrProvider,
    context: SessionCtx,
}

impl<'a> SessionOpener<'a> {
    fn new(provider: &'a dyn AsrProvider, context: SessionCtx) -> Self {
        Self { provider, context }
    }

    async fn open_initial(&self) -> Result<OpenedSession, crate::asr::types::AsrError> {
        self.provider.open(self.context.clone()).await
    }

    async fn open_resume(
        &self,
        mode: RecordingMode,
    ) -> Result<Option<OpenedSession>, crate::asr::types::AsrError> {
        if mode == RecordingMode::Continuous {
            return Ok(None);
        }
        self.provider.open(self.context.clone()).await.map(Some)
    }
}

struct VadPauseState {
    silero: crate::voice::silero::SileroVad,
    controller: crate::voice::vad::VadController,
    timeline: crate::voice::timeline::PcmTimeline,
    pre_roll_samples: u64,
    max_overlap_samples: u64,
}

impl VadPauseState {
    fn new(config: &crate::config::VoiceVadCfg) -> anyhow::Result<Self> {
        use crate::voice::silero::{SileroConfig, SileroVad};
        use crate::voice::timeline::{ms_to_samples, PcmTimeline};
        use crate::voice::vad::{VadController, VadPolicy};

        let silero = SileroVad::new(SileroConfig {
            threshold: config.threshold,
        })?;
        let controller = VadController::new(VadPolicy {
            min_start_voiced_frames: config.min_start_voiced_frames,
            pause_silence_ms: config.pause_silence_ms,
            frame_ms: SileroConfig::frame_ms(),
        });
        let retention_ms = config.pre_roll_ms + config.max_overlap_ms + 100;
        Ok(Self {
            silero,
            controller,
            timeline: PcmTimeline::new(retention_ms),
            pre_roll_samples: ms_to_samples(config.pre_roll_ms),
            max_overlap_samples: ms_to_samples(config.max_overlap_ms),
        })
    }
}

struct CurrentSessionCapture {
    start_sample: u64,
    audio_samples: u64,
    segments: Vec<SegmentCapture>,
    final_text: Option<String>,
    pending_overlay_segments: usize,
}

impl CurrentSessionCapture {
    fn new(start_sample: u64) -> Self {
        Self {
            start_sample,
            audio_samples: 0,
            segments: Vec::new(),
            final_text: None,
            pending_overlay_segments: 0,
        }
    }

    fn record_sent_samples(&mut self, samples: u64) {
        self.audio_samples += samples;
    }

    fn into_session(self, recording_started: Instant) -> Option<SessionCapture> {
        if self.audio_samples == 0
            && self.segments.is_empty()
            && self.final_text.as_deref().unwrap_or("").is_empty()
        {
            return None;
        }
        let start_ms = samples_to_ms(self.start_sample);
        let end_ms = samples_to_ms(self.start_sample + self.audio_samples);
        Some(SessionCapture {
            started_at: recording_started + Duration::from_millis(start_ms),
            ended_at: recording_started + Duration::from_millis(end_ms),
            audio_samples: self.audio_samples,
            segments: self.segments,
            final_text: self.final_text,
        })
    }
}

pub struct SessionParams {
    pub auto_paste: bool,
    pub record_audio: crate::config::RecordAudioMode,
    pub vad_trace: bool,
    pub idle_pause: bool,
    pub finalize_timeout_ms: u64,
    pub vad: crate::config::VoiceVadCfg,
    pub stop_delay_ms: u32,
    pub hotwords: Vec<String>,
    pub start_app_context: post::AppContext,
    pub post_chain: crate::post::PostChain,
    pub post_timeout_ms: u64,
    pub overlay: Option<OverlayHandle>,
    pub state: StateStore,
}

pub(crate) struct EngineOutcome {
    pub params: SessionParams,
    pub recording_id: String,
    pub recording_started_at: time::OffsetDateTime,
    pub recording_started_instant: Instant,
    pub app_context: post::AppContext,
    pub sessions: Vec<SessionCapture>,
    pub cancel_requested: bool,
    pub terminal_error: Option<HistoryError>,
    pub total_audio_samples: u64,
    pub trace: RecordingObserver,
    pub provider_name: String,
}

pub(crate) async fn run(
    provider: &dyn AsrProvider,
    params: SessionParams,
    control_rx: watch::Receiver<SessionControl>,
) -> Option<EngineOutcome> {
    let mode = RecordingMode::select(params.idle_pause, &params.vad);
    let recording_id = ulid::Ulid::new().to_string();
    let recording_started_at = time::OffsetDateTime::now_utc();
    let recording_started_instant = Instant::now();
    tracing::info!(
        recording_id = %recording_id,
        provider = %provider.name(),
        app = ?params.start_app_context.bundle_id,
        mode = ?mode,
        "recording started"
    );

    let mut trace = RecordingObserver::start(TraceStart {
        enabled: params.vad_trace,
        recording_id: recording_id.clone(),
        provider: provider.name().to_string(),
        started_at: recording_started_at.to_string(),
        started_instant: recording_started_instant,
    });
    if params.vad_trace {
        tracing::info!(recording_id = %recording_id, "dev voice trace enabled");
    }

    let audio_output = prepare_audio_output(&params, &recording_id);
    let rec = match recorder::start(audio_output) {
        Ok(rec) => rec,
        Err(error) => {
            tracing::error!(recording_id = %recording_id, error = ?error, "recorder start failed");
            observe_finish_ms(&mut trace, "recorder_start_error", 0);
            params.state.set_error(Some(recording_id));
            send_error_overlay(&params, crate::t!("error.recorder_start"));
            return None;
        }
    };

    run_with_recorder(
        provider,
        params,
        control_rx,
        rec,
        recording_id,
        recording_started_at,
        recording_started_instant,
        mode,
        trace,
    )
    .await
}

/// 录音引擎主循环。测试可直接调用并注入 [`recorder::RecordingStream::for_test`]
/// 构造的 fake recorder。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_with_recorder(
    provider: &dyn AsrProvider,
    params: SessionParams,
    mut control_rx: watch::Receiver<SessionControl>,
    mut rec: recorder::RecordingStream,
    recording_id: String,
    recording_started_at: time::OffsetDateTime,
    recording_started_instant: Instant,
    mode: RecordingMode,
    mut trace: RecordingObserver,
) -> Option<EngineOutcome> {
    let mut app_context = params.start_app_context.clone();
    params
        .state
        .set_recording(recording_id.clone(), recording_started_at);
    emit_session_meta(&params.state, &recording_id, provider, &params);
    params
        .state
        .session_phase(recording_id.clone(), UiSessionPhase::Active);
    params
        .state
        .app(app_context.bundle_id.clone(), app_context.app_name.clone());
    overlay_send(
        &params,
        OverlayCmd::SetState {
            state: OverlayState::Connecting,
        },
    );
    overlay_send(
        &params,
        OverlayCmd::SetApp {
            bundle_id: app_context.bundle_id.clone(),
            app_name: app_context.app_name.clone(),
            chain_summary: params.post_chain.name.clone(),
        },
    );

    let mut vad_pause = if mode == RecordingMode::VadPause {
        match VadPauseState::new(&params.vad) {
            Ok(state) => Some(state),
            Err(error) => {
                tracing::error!(recording_id = %recording_id, error = ?error, "Silero VAD init failed");
                discard_retained_audio(&recording_id, &mut rec).await;
                observe_finish_ms(&mut trace, "vad_init_error", 0);
                params.state.set_error(Some(recording_id));
                send_error_overlay(&params, crate::t!("error.asr_runtime"));
                return None;
            }
        }
    } else {
        None
    };

    let session_ctx = SessionCtx {
        language: LanguageMode::Multilingual {
            hint: vec!["zh-CN".into(), "en-US".into()],
        },
        hotwords: params.hotwords.clone(),
    };
    let opener = SessionOpener::new(provider, session_ctx);
    let (initial_session, initial_events) = match opener.open_initial().await {
        Ok(opened) => opened,
        Err(error) => {
            tracing::error!(recording_id = %recording_id, error = %error, "ASR open failed");
            discard_retained_audio(&recording_id, &mut rec).await;
            observe_asr_error(&mut trace, recording_started_instant, error);
            observe_finish_ms(&mut trace, "asr_open_error", 0);
            params.state.set_error(Some(recording_id));
            send_error_overlay(&params, crate::t!("error.asr_open"));
            return None;
        }
    };

    let mut session: Option<Box<dyn AsrSession>> = Some(initial_session);
    let mut events = initial_events;
    overlay_send(
        &params,
        OverlayCmd::SetState {
            state: OverlayState::Recording,
        },
    );
    observe_provider_opened(&mut trace, recording_started_instant);
    observe_session(
        &mut trace,
        SessionPhase::Start {
            index: 0,
            start_ms: 0,
        },
    );

    let first_audio_deadline = TokioInstant::now() + Duration::from_millis(FIRST_AUDIO_TIMEOUT_MS);
    let mut first_audio_seen = false;
    let mut sessions = Vec::new();
    let mut current = CurrentSessionCapture::new(0);
    let mut session_index = 0u32;
    let mut total_audio_samples = 0u64;
    let mut last_sent_sample = 0u64;
    let mut active = true;
    let mut stop_requested = false;
    let mut cancel_requested = false;
    let mut terminal_error = None;
    let mut meter = MeterCollector::new();

    'recording: loop {
        if active {
            let mut pause_requested = false;
            let mut provider_done = false;
            'active: loop {
                tokio::select! {
                    biased;
                    changed = control_rx.changed() => {
                        if changed.is_err() {
                            stop_requested = true;
                            break 'active;
                        }
                        match *control_rx.borrow_and_update() {
                            SessionControl::Stop => {
                                stop_requested = true;
                                break 'active;
                            }
                            SessionControl::Cancel => {
                                cancel_requested = true;
                                break 'recording;
                            }
                            SessionControl::Idle => {}
                        }
                    }
                    pcm = rec.recv() => {
                        match pcm {
                            None => {
                                stop_requested = true;
                                break 'active;
                            }
                            Some(samples) => {
                                observe_pcm(&mut trace, &samples);
                                emit_meters(&params, &recording_id, &mut meter, &samples);
                                if !first_audio_seen && frame_has_signal(&samples) {
                                    first_audio_seen = true;
                                }

                                let end_sample = if let Some(vad) = vad_pause.as_mut() {
                                    vad.timeline.push(&samples).end_sample()
                                } else {
                                    last_sent_sample + samples.len() as u64
                                };
                                let Some(active_session) = session.as_mut() else {
                                    terminal_error = Some(HistoryError {
                                        kind: "asr_session".to_string(),
                                        msg: "missing active ASR session".to_string(),
                                    });
                                    break 'recording;
                                };
                                if let Err(error) =
                                    send_pcm_chunk(active_session, &samples, &mut total_audio_samples).await
                                {
                                    terminal_error = Some(error);
                                    break 'recording;
                                }
                                current.record_sent_samples(samples.len() as u64);
                                last_sent_sample = end_sample;

                                if let Some(vad) = vad_pause.as_mut() {
                                    use crate::voice::vad::{VadFrame, VadTransition};
                                    for frame in vad.silero.accept(&samples) {
                                        meter.observe_vad(
                                            frame.probability,
                                            matches!(frame.frame, VadFrame::Speech),
                                        );
                                        if vad.controller.accept(frame.frame)
                                            == VadTransition::SilenceStarted
                                        {
                                            pause_requested = true;
                                            break;
                                        }
                                    }
                                }
                                if pause_requested {
                                    break 'active;
                                }
                            }
                        }
                    }
                    _ = sleep_until(first_audio_deadline), if !first_audio_seen => {
                        tracing::error!(
                            recording_id = %recording_id,
                            timeout_ms = FIRST_AUDIO_TIMEOUT_MS,
                            "no microphone audio received before timeout"
                        );
                        if let Some(active_session) = session.take() {
                            let _ = active_session.close().await;
                        }
                        discard_retained_audio(&recording_id, &mut rec).await;
                        observe_finish(&mut trace, "no_audio", total_audio_samples);
                        params.state.set_error(Some(recording_id));
                        send_error_overlay(&params, crate::t!("error.no_audio"));
                        return None;
                    }
                    event = events.recv() => {
                        match event {
                            None => {
                                terminal_error = Some(asr_stream_closed_error());
                                break 'recording;
                            }
                            Some(AsrEvent::Done) => {
                                observe_asr_event(&mut trace, recording_started_instant, &AsrEvent::Done);
                                // Provider 主动结束当前 session：不再向同一个 session 发 is_last，
                                // 也不能再等第二个 Done。VadPause 同时进入 Idle 等下一段 speech。
                                provider_done = true;
                                if mode == RecordingMode::VadPause {
                                    pause_requested = true;
                                }
                                break 'active;
                            }
                            Some(event) => {
                                if let Some(error) = handle_asr_event(
                                    event,
                                    &mut current,
                                    &params,
                                    &recording_id,
                                    recording_started_instant,
                                    &mut trace,
                                ) {
                                    terminal_error = Some(error);
                                    break 'recording;
                                }
                            }
                        }
                    }
                }
            }

            if stop_requested {
                refresh_stop_context(&params, &recording_id, &mut app_context);
                let Some(active_session) = session.as_mut() else {
                    break 'recording;
                };
                match drain_stop_audio(
                    &mut rec,
                    active_session,
                    &mut events,
                    &mut current,
                    vad_pause.as_mut(),
                    &mut total_audio_samples,
                    &mut last_sent_sample,
                    params.stop_delay_ms,
                    &mut control_rx,
                    &mut cancel_requested,
                    &mut meter,
                    &recording_id,
                    &mut trace,
                    &params,
                    recording_started_instant,
                )
                .await
                {
                    Ok(StopDrainOutcome::FinalizeRequired) => {}
                    Ok(StopDrainOutcome::ProviderDone) => provider_done = true,
                    Ok(StopDrainOutcome::Canceled) => break 'recording,
                    Err(error) => {
                        terminal_error = Some(error);
                        break 'recording;
                    }
                }
            }

            if !provider_done {
                observe_session(
                    &mut trace,
                    SessionPhase::FinalizeStart {
                        index: session_index,
                        t_ms: instant_elapsed_ms(recording_started_instant),
                    },
                );
                let Some(active_session) = session.as_mut() else {
                    break 'recording;
                };
                match finalize_provider_session(
                    active_session,
                    &mut events,
                    &mut current.segments,
                    &mut current.final_text,
                    &mut current.pending_overlay_segments,
                    params.finalize_timeout_ms,
                    &mut control_rx,
                    &mut terminal_error,
                    &mut trace,
                    recording_started_instant,
                    &params.state,
                    &recording_id,
                    params.overlay.as_ref(),
                )
                .await
                {
                    Ok(FinalizeOutcome::Done) => {}
                    Ok(FinalizeOutcome::Canceled) => {
                        cancel_requested = true;
                        break 'recording;
                    }
                    Err(error) => {
                        terminal_error = Some(error);
                        break 'recording;
                    }
                }
            }
            if let Some(active_session) = session.take() {
                let _ = active_session.close().await;
            }

            let session_start_ms = samples_to_ms(current.start_sample);
            let session_end_ms = samples_to_ms(current.start_sample + current.audio_samples);
            observe_session(
                &mut trace,
                SessionPhase::Done {
                    index: session_index,
                    start_ms: session_start_ms,
                    end_ms: session_end_ms,
                    audio_ms: samples_to_ms(current.audio_samples),
                },
            );
            if let Some(capture) =
                std::mem::replace(&mut current, CurrentSessionCapture::new(last_sent_sample))
                    .into_session(recording_started_instant)
            {
                sessions.push(capture);
            }

            if !should_enter_idle(mode, stop_requested, pause_requested) {
                break 'recording;
            }

            overlay_send(
                &params,
                OverlayCmd::SetState {
                    state: OverlayState::Idle,
                },
            );
            params
                .state
                .session_phase(recording_id.clone(), UiSessionPhase::Idle);
            if let Some(vad) = vad_pause.as_mut() {
                use crate::voice::vad::VadFrame;
                vad.controller.reset();
                vad.controller.accept(VadFrame::Silence);
            }
            active = false;
        } else {
            let mut speech_start = None;
            'idle: loop {
                tokio::select! {
                    biased;
                    changed = control_rx.changed() => {
                        if changed.is_err() {
                            stop_requested = true;
                            break 'idle;
                        }
                        match *control_rx.borrow_and_update() {
                            SessionControl::Stop => {
                                stop_requested = true;
                                break 'idle;
                            }
                            SessionControl::Cancel => {
                                cancel_requested = true;
                                break 'recording;
                            }
                            SessionControl::Idle => {}
                        }
                    }
                    pcm = rec.recv() => {
                        match pcm {
                            None => {
                                stop_requested = true;
                                break 'idle;
                            }
                            Some(samples) => {
                                observe_pcm(&mut trace, &samples);
                                emit_meters(&params, &recording_id, &mut meter, &samples);
                                let Some(vad) = vad_pause.as_mut() else {
                                    terminal_error = Some(HistoryError {
                                        kind: "vad_state".to_string(),
                                        msg: "VadPause mode missing VAD state".to_string(),
                                    });
                                    break 'recording;
                                };
                                vad.timeline.push(&samples);
                                use crate::voice::vad::{VadFrame, VadTransition};
                                for frame in vad.silero.accept(&samples) {
                                    meter.observe_vad(
                                        frame.probability,
                                        matches!(frame.frame, VadFrame::Speech),
                                    );
                                    if vad.controller.accept(frame.frame)
                                        == VadTransition::SpeechStarted
                                    {
                                        speech_start = Some(frame.start_sample);
                                        break;
                                    }
                                }
                                if speech_start.is_some() {
                                    break 'idle;
                                }
                            }
                        }
                    }
                }
            }

            if stop_requested {
                refresh_stop_context(&params, &recording_id, &mut app_context);
                break 'recording;
            }
            let Some(speech_start) = speech_start else {
                break 'recording;
            };
            let Some(vad) = vad_pause.as_mut() else {
                break 'recording;
            };
            let send_start = compute_resume_start_sample(
                speech_start,
                vad.pre_roll_samples,
                last_sent_sample,
                vad.max_overlap_samples,
                vad.timeline.oldest_sample(),
            );
            let next_index = session_index + 1;
            let (new_session, new_events) = match opener.open_resume(mode).await {
                Ok(Some(opened)) => opened,
                Ok(None) => {
                    terminal_error = Some(HistoryError {
                        kind: "asr_resume_mode".to_string(),
                        msg: "Continuous mode cannot open a resume session".to_string(),
                    });
                    break 'recording;
                }
                Err(error) => {
                    observe_session(
                        &mut trace,
                        SessionPhase::OpenError {
                            index: next_index,
                            t_ms: instant_elapsed_ms(recording_started_instant),
                            message: error.to_string(),
                        },
                    );
                    terminal_error = Some(HistoryError {
                        kind: "asr_resume_open".to_string(),
                        msg: error.to_string(),
                    });
                    break 'recording;
                }
            };
            session = Some(new_session);
            events = new_events;
            session_index = next_index;
            observe_provider_opened(&mut trace, recording_started_instant);
            observe_session(
                &mut trace,
                SessionPhase::Start {
                    index: session_index,
                    start_ms: samples_to_ms(send_start),
                },
            );

            let replay = vad.timeline.slice_from(send_start);
            current = CurrentSessionCapture::new(replay.start_sample);
            if !replay.samples.is_empty() {
                let Some(active_session) = session.as_mut() else {
                    break 'recording;
                };
                if let Err(error) =
                    send_pcm_chunk(active_session, &replay.samples, &mut total_audio_samples).await
                {
                    terminal_error = Some(error);
                    break 'recording;
                }
                current.record_sent_samples(replay.samples.len() as u64);
            }
            last_sent_sample = replay.end_sample();
            use crate::voice::vad::VadFrame;
            vad.controller.reset();
            vad.controller.accept(VadFrame::Speech);
            overlay_send(
                &params,
                OverlayCmd::SetState {
                    state: OverlayState::Recording,
                },
            );
            params
                .state
                .session_phase(recording_id.clone(), UiSessionPhase::Active);
            active = true;
        }
    }

    if current.audio_samples > 0
        || !current.segments.is_empty()
        || current
            .final_text
            .as_deref()
            .is_some_and(|text| !text.is_empty())
    {
        if let Some(capture) = current.into_session(recording_started_instant) {
            sessions.push(capture);
        }
    }
    if let Some(active_session) = session.take() {
        let _ = active_session.close().await;
    }
    // 取消时音频留存跟随「是否有内容」：有内容（可能误触）保留以便用户从 TUI
    // 找回，无内容则丢弃避免孤儿音频文件。正常完成 / terminal error 照常 finalize。
    if cancel_requested && !crate::voice::capture::has_archivable_content(&sessions) {
        discard_retained_audio(&recording_id, &mut rec).await;
    } else {
        finish_retained_audio(&params, &recording_id, &mut rec).await;
    }

    Some(EngineOutcome {
        provider_name: provider.name().to_string(),
        params,
        recording_id,
        recording_started_at,
        recording_started_instant,
        app_context,
        sessions,
        cancel_requested,
        terminal_error,
        total_audio_samples,
        trace,
    })
}

fn should_enter_idle(mode: RecordingMode, stop_requested: bool, pause_requested: bool) -> bool {
    mode == RecordingMode::VadPause && !stop_requested && pause_requested
}

fn handle_asr_event(
    event: AsrEvent,
    current: &mut CurrentSessionCapture,
    params: &SessionParams,
    recording_id: &str,
    recording_started_instant: Instant,
    trace: &mut RecordingObserver,
) -> Option<HistoryError> {
    observe_asr_event(trace, recording_started_instant, &event);
    match event {
        AsrEvent::Final { text } => {
            current.final_text = Some(text.clone());
            overlay_send(
                params,
                OverlayCmd::ReplaceRecentSegments {
                    segments: current.pending_overlay_segments,
                    text,
                },
            );
            current.pending_overlay_segments = 1;
        }
        AsrEvent::Partial { text, .. } => {
            params.state.partial(recording_id.to_string(), text.clone());
            let live_text = format!(
                "{}{}",
                current
                    .segments
                    .iter()
                    .map(|segment| segment.text.as_str())
                    .collect::<String>(),
                text
            );
            let words = crate::text_stats::compute(&live_text).words as u32;
            let dur_ms = recording_started_instant.elapsed().as_millis() as u64;
            params.state.stats(recording_id.to_string(), dur_ms, words);
            overlay_send(params, OverlayCmd::SetStats { dur_ms, words });
            overlay_send(
                params,
                OverlayCmd::SetText {
                    text,
                    kind: TextKind::Partial,
                },
            );
        }
        AsrEvent::Segment {
            text,
            started_at,
            ended_at,
        } => {
            params.state.segment(recording_id.to_string(), text.clone());
            overlay_send(params, OverlayCmd::AppendSegment { text: text.clone() });
            current.pending_overlay_segments += 1;
            current.segments.push(SegmentCapture {
                text,
                started_at,
                ended_at,
            });
        }
        AsrEvent::Error { err } => {
            tracing::error!(recording_id, error = %err, "ASR event error");
            return Some(HistoryError {
                kind: "asr_error".to_string(),
                msg: err.to_string(),
            });
        }
        AsrEvent::Done => {}
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopDrainOutcome {
    FinalizeRequired,
    ProviderDone,
    Canceled,
}

#[allow(clippy::too_many_arguments)]
async fn drain_stop_audio(
    rec: &mut recorder::RecordingStream,
    session: &mut Box<dyn AsrSession>,
    events: &mut mpsc::Receiver<AsrEvent>,
    current: &mut CurrentSessionCapture,
    mut vad_pause: Option<&mut VadPauseState>,
    total_audio_samples: &mut u64,
    last_sent_sample: &mut u64,
    stop_delay_ms: u32,
    control_rx: &mut watch::Receiver<SessionControl>,
    cancel_requested: &mut bool,
    meter: &mut MeterCollector,
    recording_id: &str,
    trace: &mut RecordingObserver,
    params: &SessionParams,
    recording_started_instant: Instant,
) -> Result<StopDrainOutcome, HistoryError> {
    let drain_until = Instant::now() + Duration::from_millis(stop_delay_ms as u64);
    while Instant::now() < drain_until {
        tokio::select! {
            biased;
            changed = control_rx.changed() => {
                if changed.is_ok()
                    && matches!(*control_rx.borrow_and_update(), SessionControl::Cancel)
                {
                    *cancel_requested = true;
                    return Ok(StopDrainOutcome::Canceled);
                }
            }
            _ = sleep_until(TokioInstant::from_std(drain_until)) => break,
            pcm = rec.recv() => {
                let Some(samples) = pcm else {
                    break;
                };
                observe_pcm(trace, &samples);
                emit_meters(params, recording_id, meter, &samples);
                send_pcm_chunk(session, &samples, total_audio_samples).await?;
                current.record_sent_samples(samples.len() as u64);
                *last_sent_sample = if let Some(vad) = vad_pause.as_mut() {
                    vad.timeline.push(&samples).end_sample()
                } else {
                    *last_sent_sample + samples.len() as u64
                };
            }
            event = events.recv() => {
                match event {
                    None => return Err(asr_stream_closed_error()),
                    Some(AsrEvent::Done) => {
                        observe_asr_event(trace, recording_started_instant, &AsrEvent::Done);
                        rec.stop();
                        while rec.try_recv().is_some() {}
                        return Ok(StopDrainOutcome::ProviderDone);
                    }
                    Some(event) => {
                        if let Some(error) = handle_asr_event(
                            event,
                            current,
                            params,
                            recording_id,
                            recording_started_instant,
                            trace,
                        ) {
                            return Err(error);
                        }
                    }
                }
            }
        }
    }

    let drained = rec.drain_after_stop().await;
    for samples in drained {
        observe_pcm(trace, &samples);
        emit_meters(params, recording_id, meter, &samples);
        send_pcm_chunk(session, &samples, total_audio_samples).await?;
        current.record_sent_samples(samples.len() as u64);
        *last_sent_sample = if let Some(vad) = vad_pause.as_mut() {
            vad.timeline.push(&samples).end_sample()
        } else {
            *last_sent_sample + samples.len() as u64
        };
    }
    Ok(StopDrainOutcome::FinalizeRequired)
}

fn refresh_stop_context(
    params: &SessionParams,
    recording_id: &str,
    app_context: &mut post::AppContext,
) {
    *app_context = post::app_context::frontmost_app();
    params
        .state
        .app(app_context.bundle_id.clone(), app_context.app_name.clone());
    overlay_send(
        params,
        OverlayCmd::SetApp {
            bundle_id: app_context.bundle_id.clone(),
            app_name: app_context.app_name.clone(),
            chain_summary: params.post_chain.name.clone(),
        },
    );
    overlay_send(
        params,
        OverlayCmd::SetState {
            state: OverlayState::Stopping,
        },
    );
    params.state.set_stopping(recording_id.to_string());
    params
        .state
        .session_phase(recording_id.to_string(), UiSessionPhase::Stopping);
}

fn compute_resume_start_sample(
    speech_start_sample: u64,
    pre_roll_samples: u64,
    last_sent_sample: u64,
    max_overlap_samples: u64,
    oldest_sample: u64,
) -> u64 {
    speech_start_sample
        .saturating_sub(pre_roll_samples)
        .max(last_sent_sample.saturating_sub(max_overlap_samples))
        .max(oldest_sample)
}

fn frame_has_signal(samples: &[i16]) -> bool {
    samples
        .iter()
        .any(|sample| sample.unsigned_abs() > MIN_NONZERO_AMPLITUDE as u16)
}

fn emit_meters(
    params: &SessionParams,
    recording_id: &str,
    collector: &mut MeterCollector,
    samples: &[i16],
) {
    for meter in collector.accept(samples) {
        overlay_send(params, OverlayCmd::SetLevel { rms: meter.rms });
        params.state.audio_meter(recording_id.to_string(), meter);
    }
}

fn emit_session_meta(
    state: &StateStore,
    recording_id: &str,
    provider: &dyn AsrProvider,
    params: &SessionParams,
) {
    state.session_meta(
        recording_id.to_string(),
        SessionMeta {
            provider: provider.name().to_string(),
            chain: params.post_chain.name.clone(),
            vad: format!("{:?}", params.vad.backend).to_lowercase(),
            hotwords: params.hotwords.len(),
        },
    );
}

fn asr_stream_closed_error() -> HistoryError {
    HistoryError {
        kind: "asr_stream_closed".to_string(),
        msg: "ASR event stream closed before Done".to_string(),
    }
}

async fn send_pcm_chunk(
    session: &mut Box<dyn AsrSession>,
    samples: &[i16],
    audio_samples_sent: &mut u64,
) -> Result<(), HistoryError> {
    session
        .send_pcm(samples, false)
        .await
        .map_err(|error| HistoryError {
            kind: "asr_send".to_string(),
            msg: error.to_string(),
        })?;
    *audio_samples_sent += samples.len() as u64;
    Ok(())
}

pub(crate) fn send_error_overlay(params: &SessionParams, message: String) {
    overlay_send(
        params,
        OverlayCmd::SetState {
            state: OverlayState::Error,
        },
    );
    overlay_send(
        params,
        OverlayCmd::SetText {
            text: message,
            kind: TextKind::Error,
        },
    );
}

pub(crate) fn overlay_send(params: &SessionParams, cmd: OverlayCmd) {
    if let Some(overlay) = &params.overlay {
        overlay.send(cmd);
    }
}

fn prepare_audio_output(params: &SessionParams, recording_id: &str) -> Option<audio::AudioOutput> {
    match audio::prepare(recording_id, params.record_audio) {
        Ok(output) => output,
        Err(error) => {
            publish_audio_failure(&params.state, params.overlay.as_ref(), recording_id, error);
            None
        }
    }
}

async fn finish_retained_audio(
    params: &SessionParams,
    recording_id: &str,
    recorder: &mut recorder::RecordingStream,
) {
    recorder.stop();
    match recorder.finish_audio().await {
        Ok(Some(path)) => {
            tracing::info!(recording_id, path = %path.display(), "retained audio saved");
        }
        Ok(None) => {}
        Err(error) => {
            publish_audio_failure(&params.state, params.overlay.as_ref(), recording_id, error);
        }
    }
}

async fn discard_retained_audio(recording_id: &str, recorder: &mut recorder::RecordingStream) {
    if let Err(error) = recorder.discard_audio().await {
        tracing::warn!(recording_id, error = ?error, "discard retained audio failed");
    }
}

fn publish_audio_failure(
    state: &StateStore,
    overlay: Option<&OverlayHandle>,
    recording_id: &str,
    error: anyhow::Error,
) {
    tracing::error!(recording_id, error = ?error, "retained audio save failed");
    state.error(
        Some(recording_id.to_string()),
        "audio_save",
        format!("{error:#}"),
    );
    if let Some(overlay) = overlay {
        overlay.send(OverlayCmd::Notice {
            text: crate::t!("notice.audio_save_failed"),
            ttl_ms: NOTICE_TTL_MS,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::asr::types::AsrError;

    struct FailingSendSession;

    #[async_trait]
    impl AsrSession for FailingSendSession {
        async fn send_pcm(&mut self, _pcm: &[i16], _is_last: bool) -> Result<(), AsrError> {
            Err(AsrError::Network("send failed".to_string()))
        }

        async fn close(self: Box<Self>) -> Result<(), AsrError> {
            Ok(())
        }
    }

    struct CountingProvider {
        opens: AtomicUsize,
    }

    #[async_trait]
    impl AsrProvider for CountingProvider {
        fn name(&self) -> &str {
            "counting"
        }

        fn caps(&self) -> crate::asr::types::Caps {
            crate::asr::types::Caps {
                hotwords: false,
                max_session_secs: None,
                multilingual: true,
            }
        }

        async fn open(&self, _ctx: SessionCtx) -> Result<OpenedSession, AsrError> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            let (_tx, rx) = mpsc::channel(1);
            Ok((Box::new(CollectingSession::default()), rx))
        }
    }

    #[derive(Default)]
    struct CollectingSession {
        sent: Arc<Mutex<Vec<Vec<i16>>>>,
    }

    #[async_trait]
    impl AsrSession for CollectingSession {
        async fn send_pcm(&mut self, pcm: &[i16], _is_last: bool) -> Result<(), AsrError> {
            self.sent.lock().unwrap().push(pcm.to_vec());
            Ok(())
        }

        async fn close(self: Box<Self>) -> Result<(), AsrError> {
            Ok(())
        }
    }

    fn test_session_ctx() -> SessionCtx {
        SessionCtx {
            language: LanguageMode::Single("zh-CN".to_string()),
            hotwords: Vec::new(),
        }
    }

    #[test]
    fn recording_mode_requires_both_idle_pause_and_silero() {
        let mut vad = crate::config::VoiceVadCfg::default();
        assert_eq!(
            RecordingMode::select(false, &vad),
            RecordingMode::Continuous
        );
        vad.backend = crate::config::VoiceVadBackend::Silero;
        assert_eq!(
            RecordingMode::select(false, &vad),
            RecordingMode::Continuous
        );
        assert_eq!(RecordingMode::select(true, &vad), RecordingMode::VadPause);
    }

    #[test]
    fn continuous_capture_produces_at_most_one_session() {
        let started_at = Instant::now();
        let mut capture = CurrentSessionCapture::new(0);
        capture.record_sent_samples(16_000);
        capture.segments.push(SegmentCapture {
            text: "hello".to_string(),
            started_at,
            ended_at: started_at + Duration::from_millis(500),
        });
        let session = capture.into_session(started_at).unwrap();
        assert_eq!(session.audio_samples, 16_000);
        assert_eq!(
            session
                .ended_at
                .saturating_duration_since(session.started_at)
                .as_millis(),
            1_000
        );
    }

    #[test]
    fn empty_continuous_capture_produces_no_session() {
        assert!(CurrentSessionCapture::new(0)
            .into_session(Instant::now())
            .is_none());
    }

    #[tokio::test]
    async fn continuous_opens_only_initial_provider_session() {
        let provider = CountingProvider {
            opens: AtomicUsize::new(0),
        };
        let opener = SessionOpener::new(&provider, test_session_ctx());

        let _initial = opener.open_initial().await.unwrap();
        let resumed = opener.open_resume(RecordingMode::Continuous).await.unwrap();

        assert!(resumed.is_none());
        assert_eq!(provider.opens.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn vad_pause_resume_opens_another_provider_session() {
        let provider = CountingProvider {
            opens: AtomicUsize::new(0),
        };
        let opener = SessionOpener::new(&provider, test_session_ctx());

        let _initial = opener.open_initial().await.unwrap();
        let resumed = opener.open_resume(RecordingMode::VadPause).await.unwrap();

        assert!(resumed.is_some());
        assert_eq!(provider.opens.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn vad_pause_resume_propagates_provider_open_error() {
        struct FailingProvider;
        #[async_trait]
        impl AsrProvider for FailingProvider {
            fn name(&self) -> &str {
                "failing"
            }
            fn caps(&self) -> crate::asr::types::Caps {
                crate::asr::types::Caps {
                    hotwords: false,
                    max_session_secs: None,
                    multilingual: true,
                }
            }
            async fn open(&self, _ctx: SessionCtx) -> Result<OpenedSession, AsrError> {
                Err(AsrError::Network("resume open denied".to_string()))
            }
        }

        let provider = FailingProvider;
        let opener = SessionOpener::new(&provider, test_session_ctx());
        match opener.open_resume(RecordingMode::VadPause).await {
            Err(AsrError::Network(_)) => {}
            Err(other) => panic!("expected AsrError::Network, got {other:?}"),
            Ok(_) => panic!("VadPause resume must surface provider open errors"),
        }
    }

    #[tokio::test]
    async fn continuous_pcm_delivery_preserves_every_chunk() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let mut session: Box<dyn AsrSession> = Box::new(CollectingSession { sent: sent.clone() });
        let mut audio_samples_sent = 0;

        send_pcm_chunk(&mut session, &[1, 2], &mut audio_samples_sent)
            .await
            .unwrap();
        send_pcm_chunk(&mut session, &[3, 4, 5], &mut audio_samples_sent)
            .await
            .unwrap();

        assert_eq!(*sent.lock().unwrap(), vec![vec![1, 2], vec![3, 4, 5]]);
        assert_eq!(audio_samples_sent, 5);
    }

    #[test]
    fn only_vad_pause_can_enter_idle() {
        assert!(!should_enter_idle(RecordingMode::Continuous, false, true));
        assert!(!should_enter_idle(RecordingMode::VadPause, true, true));
        assert!(!should_enter_idle(RecordingMode::VadPause, false, false));
        assert!(should_enter_idle(RecordingMode::VadPause, false, true));
    }

    #[tokio::test]
    async fn pcm_send_failure_does_not_count_unsent_audio() {
        let mut session: Box<dyn AsrSession> = Box::new(FailingSendSession);
        let mut audio_samples_sent = 7;
        let error = send_pcm_chunk(&mut session, &[1, 2, 3], &mut audio_samples_sent)
            .await
            .expect_err("failed PCM delivery must stop normal completion");
        assert_eq!(error.kind, "asr_send");
        assert_eq!(audio_samples_sent, 7);
    }

    #[test]
    fn audio_save_failure_emits_error_and_notice() {
        let state = StateStore::new();
        let (_, mut state_rx) = state.subscribe_with_snapshot();
        let (overlay, mut overlay_rx) = OverlayHandle::channel();
        publish_audio_failure(
            &state,
            Some(&overlay),
            "01HXYZ",
            anyhow::anyhow!("encode failed"),
        );
        match state_rx.try_recv().unwrap() {
            crate::state::StateEvent::Error { kind, msg, .. } => {
                assert_eq!(kind, "audio_save");
                assert!(msg.contains("encode failed"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
        match overlay_rx.try_recv().unwrap() {
            OverlayCmd::Notice { text, ttl_ms } => {
                assert_eq!(text, crate::t!("notice.audio_save_failed"));
                assert_eq!(ttl_ms, NOTICE_TTL_MS);
            }
            other => panic!("unexpected overlay command: {other:?}"),
        }
    }

    #[test]
    fn resume_start_uses_pre_roll_when_buffer_has_headroom() {
        assert_eq!(
            compute_resume_start_sample(16_000, 4_800, 0, 3_200, 0),
            11_200
        );
    }

    #[test]
    fn resume_start_bounded_by_last_sent_minus_max_overlap() {
        assert_eq!(
            compute_resume_start_sample(18_000, 8_000, 20_000, 3_200, 0),
            16_800
        );
    }

    #[test]
    fn resume_start_clamped_to_oldest_retained_sample() {
        assert_eq!(compute_resume_start_sample(2_000, 4_000, 0, 200, 800), 800);
    }

    #[test]
    fn resume_overlap_stays_within_cap() {
        let max_overlap = 200 * 16_000 / 1_000;
        let start = compute_resume_start_sample(10_000, 0, 16_000, max_overlap, 0);
        assert!(16_000 - start <= max_overlap);
    }

    #[test]
    fn audio_signal_threshold_rejects_zero_and_accepts_noise_floor() {
        assert!(!frame_has_signal(&[0; 480]));
        assert!(!frame_has_signal(&[1, -2, 8, -8]));
        assert!(frame_has_signal(&[0, MIN_NONZERO_AMPLITUDE + 1, 0]));
        assert!(frame_has_signal(&[80, -50, 60]));
    }
}
