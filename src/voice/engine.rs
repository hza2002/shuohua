//! 录音期间的统一 Active / Idle 引擎。
//!
//! `Continuous` 和 `VadPause` 只在 PCM 路由与 session 切换上不同。本模块负责
//! recorder/provider 初始化、ASR event、stop drain、provider finalize、
//! Active/Idle、错误/取消和 retained audio，完成后返回 [`EngineOutcome`]。

use std::time::{Duration, Instant};

use crate::asr::types::{AsrEvent, AsrProvider, AsrSession, LanguageMode, SessionCtx};
use crate::history::HistoryError;
use crate::overlay::{OverlayCmd, OverlayHandle, OverlayState, ProfileChoice, TextKind};
use crate::post;
use crate::state::{SessionMeta, SessionPhase as UiSessionPhase, StateStore};
use crate::voice::capture::{samples_to_ms, SegmentCapture, SessionCapture};
use crate::voice::finalize::{finalize_provider_session, FinalizeOutcome};
use crate::voice::meter::MeterCollector;
use crate::voice::observer::{
    instant_elapsed_ms, observe_asr_error, observe_asr_event, observe_finish, observe_finish_ms,
    observe_pcm, observe_provider_opened, observe_session, RecordingObserver, SessionPhase,
    TraceStart,
};
use crate::voice::{audio, recorder, CancelSignal, SessionControl};
use tokio::sync::mpsc;
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
    detector: VadDetector,
    controller: crate::voice::vad::VadController,
    timeline: crate::voice::timeline::PcmTimeline,
    pre_roll_samples: u64,
    max_overlap_samples: u64,
}

enum VadDetector {
    Silero(crate::voice::silero::SileroVad),
    #[cfg(test)]
    Scripted {
        frames: std::collections::VecDeque<crate::voice::vad::VadFrame>,
        buffered_samples: usize,
        sample_offset: u64,
    },
}

impl VadDetector {
    fn accept(&mut self, samples: &[i16]) -> Vec<crate::voice::silero::SileroFrame> {
        match self {
            Self::Silero(vad) => vad.accept(samples),
            #[cfg(test)]
            Self::Scripted {
                frames,
                buffered_samples,
                sample_offset,
            } => {
                let frame_samples = crate::voice::silero::SileroConfig::frame_samples();
                *buffered_samples += samples.len();
                let frame_count = *buffered_samples / frame_samples;
                *buffered_samples %= frame_samples;

                let mut out = Vec::with_capacity(frame_count);
                for _ in 0..frame_count {
                    let frame = frames
                        .pop_front()
                        .unwrap_or(crate::voice::vad::VadFrame::Silence);
                    let start_sample = *sample_offset;
                    *sample_offset += frame_samples as u64;
                    out.push(crate::voice::silero::SileroFrame {
                        start_sample,
                        probability: if matches!(frame, crate::voice::vad::VadFrame::Speech) {
                            1.0
                        } else {
                            0.0
                        },
                        frame,
                    });
                }
                out
            }
        }
    }
}

impl VadPauseState {
    fn new(config: &crate::config::VoiceVadCfg) -> anyhow::Result<Self> {
        use crate::voice::silero::{SileroConfig, SileroVad};

        let silero = SileroVad::new(SileroConfig {
            threshold: config.threshold,
        })?;
        Self::with_detector(config, VadDetector::Silero(silero))
    }

    fn with_detector(
        config: &crate::config::VoiceVadCfg,
        detector: VadDetector,
    ) -> anyhow::Result<Self> {
        use crate::voice::silero::SileroConfig;
        use crate::voice::timeline::{ms_to_samples, PcmTimeline};
        use crate::voice::vad::{VadController, VadPolicy};

        let controller = VadController::new(VadPolicy {
            min_start_voiced_frames: config.min_start_voiced_frames,
            pause_silence_ms: config.pause_silence_ms,
            frame_ms: SileroConfig::frame_ms(),
        });
        let retention_ms = config.pre_roll_ms + config.max_overlap_ms + 100;
        Ok(Self {
            detector,
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
    pub preprocess: crate::config::VoicePreprocessCfg,
    pub vad_trace: bool,
    pub idle_pause: bool,
    pub finalize_timeout_ms: u64,
    pub vad: crate::config::VoiceVadCfg,
    pub stop_delay_ms: u32,
    pub hotwords: Vec<String>,
    pub start_app_context: post::AppContext,
    /// 当前选中的 profile 名（overlay meta 行前缀显示）。
    pub profile_name: String,
    pub profile_choices: Vec<ProfileChoice>,
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

enum CaptureStream {
    Cpal(recorder::RecordingStream),
    #[cfg(target_os = "macos")]
    Apple(crate::voice::apple_source::RunningAppleVpSource),
}

impl CaptureStream {
    async fn recv(&mut self) -> anyhow::Result<Option<Vec<i16>>> {
        match self {
            Self::Cpal(recorder) => Ok(recorder.recv().await),
            #[cfg(target_os = "macos")]
            Self::Apple(source) => source.recv().await,
        }
    }

    fn try_recv(&mut self) -> anyhow::Result<Option<Vec<i16>>> {
        match self {
            Self::Cpal(recorder) => Ok(recorder.try_recv()),
            #[cfg(target_os = "macos")]
            Self::Apple(_) => Ok(None),
        }
    }

    fn stop(&mut self) {
        match self {
            Self::Cpal(recorder) => recorder.stop(),
            #[cfg(target_os = "macos")]
            Self::Apple(source) => source.request_stop(),
        }
    }

    async fn drain_after_stop(&mut self) -> anyhow::Result<Vec<Vec<i16>>> {
        match self {
            Self::Cpal(recorder) => Ok(recorder.drain_after_stop().await),
            #[cfg(target_os = "macos")]
            Self::Apple(source) => source.drain_after_stop().await,
        }
    }

    async fn finish_audio(&mut self) -> anyhow::Result<Option<std::path::PathBuf>> {
        match self {
            Self::Cpal(recorder) => recorder.finish_audio().await,
            #[cfg(target_os = "macos")]
            Self::Apple(source) => {
                source.stop().await?;
                Ok(None)
            }
        }
    }

    async fn discard_audio(&mut self) -> anyhow::Result<()> {
        match self {
            Self::Cpal(recorder) => recorder.discard_audio().await,
            #[cfg(target_os = "macos")]
            Self::Apple(source) => source.stop().await,
        }
    }
}

pub(crate) async fn run(
    provider: &dyn AsrProvider,
    params: SessionParams,
    control: SessionControl,
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

    let app_context =
        begin_recording_ui(&params, provider, &recording_id, recording_started_at, mode);
    let session_ctx = initial_session_ctx(&params);
    let opener = SessionOpener::new(provider, session_ctx);
    let (rec, initial) = match mode {
        RecordingMode::Continuous => {
            match start_capture_and_open_initial(
                &params,
                &recording_id,
                &opener,
                provider,
                &control,
            )
            .await
            {
                Ok((rec, session, events)) => (rec, Some((session, events))),
                Err(StartSessionError::Capture(error)) => {
                    tracing::error!(recording_id = %recording_id, error = ?error, "recorder start failed");
                    observe_finish_ms(&mut trace, "recorder_start_error", 0);
                    params.state.set_error(Some(recording_id));
                    send_error_overlay(&params, crate::t!("error.recorder_start"));
                    return None;
                }
                Err(StartSessionError::Asr(error)) => {
                    observe_asr_error(&mut trace, recording_started_instant, error);
                    observe_finish_ms(&mut trace, "asr_open_error", 0);
                    params.state.set_error(Some(recording_id));
                    send_error_overlay(&params, crate::t!("error.asr_open"));
                    return None;
                }
                Err(StartSessionError::Canceled) => {
                    observe_finish_ms(&mut trace, "startup_canceled", 0);
                    params.state.set_idle();
                    overlay_send(&params, OverlayCmd::Hide);
                    return None;
                }
            }
        }
        RecordingMode::VadPause => match start_capture_stream(&params, &recording_id).await {
            Ok(rec) => (rec, None),
            Err(error) => {
                tracing::error!(recording_id = %recording_id, error = ?error, "recorder start failed");
                observe_finish_ms(&mut trace, "recorder_start_error", 0);
                params.state.set_error(Some(recording_id));
                send_error_overlay(&params, crate::t!("error.recorder_start"));
                return None;
            }
        },
    };

    run_with_capture_stream(
        provider,
        params,
        control,
        rec,
        initial,
        app_context,
        recording_id,
        recording_started_at,
        recording_started_instant,
        mode,
        trace,
        None,
    )
    .await
}

enum StartSessionError {
    Capture(anyhow::Error),
    Asr(crate::asr::types::AsrError),
    Canceled,
}

async fn start_capture_and_open_initial(
    params: &SessionParams,
    recording_id: &str,
    opener: &SessionOpener<'_>,
    provider: &dyn AsrProvider,
    control: &SessionControl,
) -> Result<(CaptureStream, Box<dyn AsrSession>, mpsc::Receiver<AsrEvent>), StartSessionError> {
    let capture = start_capture_stream(params, recording_id);
    let asr = open_initial_session(recording_id, opener, provider);
    join_capture_and_asr(capture, asr, recording_id, control).await
}

async fn join_capture_and_asr<C, A>(
    capture: C,
    asr: A,
    recording_id: &str,
    control: &SessionControl,
) -> Result<(CaptureStream, Box<dyn AsrSession>, mpsc::Receiver<AsrEvent>), StartSessionError>
where
    C: std::future::Future<Output = anyhow::Result<CaptureStream>>,
    A: std::future::Future<Output = Result<OpenedSession, crate::asr::types::AsrError>>,
{
    tokio::pin!(capture);
    tokio::pin!(asr);

    tokio::select! {
        biased;
        _ = control.cancelled() => Err(StartSessionError::Canceled),
        _ = control.stopped() => Err(StartSessionError::Canceled),
        result = &mut capture => {
            match result {
                Ok(rec) => finish_startup_after_capture(rec, asr, recording_id, control).await,
                Err(error) => Err(StartSessionError::Capture(error)),
            }
        }
        result = &mut asr => {
            match result {
                Ok(opened) => finish_startup_after_asr(opened, capture, control).await,
                Err(error) => Err(StartSessionError::Asr(error)),
            }
        }
    }
}

async fn finish_startup_after_capture<A>(
    mut rec: CaptureStream,
    asr: std::pin::Pin<&mut A>,
    recording_id: &str,
    control: &SessionControl,
) -> Result<(CaptureStream, Box<dyn AsrSession>, mpsc::Receiver<AsrEvent>), StartSessionError>
where
    A: std::future::Future<Output = Result<OpenedSession, crate::asr::types::AsrError>>,
{
    tokio::select! {
        biased;
        _ = control.cancelled() => {
            discard_retained_audio(recording_id, &mut rec).await;
            Err(StartSessionError::Canceled)
        }
        _ = control.stopped() => {
            discard_retained_audio(recording_id, &mut rec).await;
            Err(StartSessionError::Canceled)
        }
        result = asr => {
            match result {
                Ok((session, events)) => Ok((rec, session, events)),
                Err(error) => {
                    discard_retained_audio(recording_id, &mut rec).await;
                    Err(StartSessionError::Asr(error))
                }
            }
        }
    }
}

async fn finish_startup_after_asr<C>(
    opened: OpenedSession,
    capture: std::pin::Pin<&mut C>,
    control: &SessionControl,
) -> Result<(CaptureStream, Box<dyn AsrSession>, mpsc::Receiver<AsrEvent>), StartSessionError>
where
    C: std::future::Future<Output = anyhow::Result<CaptureStream>>,
{
    let (session, events) = opened;
    tokio::select! {
        biased;
        _ = control.cancelled() => {
            let _ = session.close().await;
            Err(StartSessionError::Canceled)
        }
        _ = control.stopped() => {
            let _ = session.close().await;
            Err(StartSessionError::Canceled)
        }
        result = capture => {
            match result {
                Ok(rec) => Ok((rec, session, events)),
                Err(error) => {
                    let _ = session.close().await;
                    Err(StartSessionError::Capture(error))
                }
            }
        }
    }
}

async fn start_capture_stream(
    params: &SessionParams,
    recording_id: &str,
) -> anyhow::Result<CaptureStream> {
    let backend = params.preprocess.backend;
    let started = Instant::now();
    let result = match backend {
        crate::config::VoicePreprocessBackend::Off => {
            let audio_output = prepare_audio_output(params, recording_id);
            recorder::start(audio_output).map(CaptureStream::Cpal)
        }
        crate::config::VoicePreprocessBackend::Apple => start_apple_capture_stream().await,
    };
    if result.is_ok() {
        tracing::info!(
            recording_id,
            backend = ?backend,
            duration_ms = started.elapsed().as_millis() as u64,
            "capture stream started"
        );
    } else {
        tracing::warn!(
            recording_id,
            backend = ?backend,
            duration_ms = started.elapsed().as_millis() as u64,
            "capture stream start failed"
        );
    }
    result
}

fn initial_session_ctx(params: &SessionParams) -> SessionCtx {
    SessionCtx {
        language: LanguageMode::Multilingual {
            hint: vec!["zh-CN".into(), "en-US".into()],
        },
        hotwords: params.hotwords.clone(),
    }
}

async fn open_initial_session(
    recording_id: &str,
    opener: &SessionOpener<'_>,
    provider: &dyn AsrProvider,
) -> Result<OpenedSession, crate::asr::types::AsrError> {
    let asr_open_started = Instant::now();
    match opener.open_initial().await {
        Ok(opened) => {
            tracing::info!(
                recording_id = %recording_id,
                provider = %provider.name(),
                duration_ms = asr_open_started.elapsed().as_millis() as u64,
                "ASR session opened"
            );
            Ok(opened)
        }
        Err(error) => {
            tracing::warn!(
                recording_id = %recording_id,
                provider = %provider.name(),
                duration_ms = asr_open_started.elapsed().as_millis() as u64,
                "ASR session open failed"
            );
            tracing::error!(recording_id = %recording_id, error = %error, "ASR open failed");
            Err(error)
        }
    }
}

fn begin_recording_ui(
    params: &SessionParams,
    provider: &dyn AsrProvider,
    recording_id: &str,
    recording_started_at: time::OffsetDateTime,
    mode: RecordingMode,
) -> post::AppContext {
    let app_context = params.start_app_context.clone();
    params
        .state
        .set_recording(recording_id.to_string(), recording_started_at);
    emit_session_meta(&params.state, recording_id, provider, params);
    let initial_phase = match mode {
        RecordingMode::Continuous => UiSessionPhase::Active,
        RecordingMode::VadPause => UiSessionPhase::Idle,
    };
    params
        .state
        .session_phase(recording_id.to_string(), initial_phase);
    params
        .state
        .app(app_context.bundle_id.clone(), app_context.app_name.clone());
    overlay_send(
        params,
        OverlayCmd::SetState {
            state: OverlayState::Connecting,
        },
    );
    overlay_send(
        params,
        OverlayCmd::SetApp {
            bundle_id: app_context.bundle_id.clone(),
            app_name: app_context.app_name.clone(),
            profile: params.profile_name.clone(),
            profiles: params.profile_choices.clone(),
            chain_summary: params.post_chain.name.clone(),
        },
    );
    app_context
}

#[cfg(target_os = "macos")]
async fn start_apple_capture_stream() -> anyhow::Result<CaptureStream> {
    let source = crate::voice::apple_source::AppleVpSource::prepare_helper()?;
    source.start().await.map(CaptureStream::Apple)
}

#[cfg(not(target_os = "macos"))]
async fn start_apple_capture_stream() -> anyhow::Result<CaptureStream> {
    anyhow::bail!("voice preprocess backend \"apple\" is only implemented on macOS")
}

/// 录音引擎主循环。测试可直接调用并注入 [`recorder::RecordingStream::for_test`]
/// 构造的 fake recorder。
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_with_recorder(
    provider: &dyn AsrProvider,
    params: SessionParams,
    control: SessionControl,
    rec: recorder::RecordingStream,
    recording_id: String,
    recording_started_at: time::OffsetDateTime,
    recording_started_instant: Instant,
    mode: RecordingMode,
    trace: RecordingObserver,
) -> Option<EngineOutcome> {
    run_with_recorder_inner(
        provider,
        params,
        control,
        rec,
        recording_id,
        recording_started_at,
        recording_started_instant,
        mode,
        trace,
        None,
    )
    .await
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_with_recorder_and_vad_frames(
    provider: &dyn AsrProvider,
    params: SessionParams,
    control: SessionControl,
    rec: recorder::RecordingStream,
    recording_id: String,
    recording_started_at: time::OffsetDateTime,
    recording_started_instant: Instant,
    mode: RecordingMode,
    trace: RecordingObserver,
    frames: std::collections::VecDeque<crate::voice::vad::VadFrame>,
) -> Option<EngineOutcome> {
    run_with_recorder_inner(
        provider,
        params,
        control,
        rec,
        recording_id,
        recording_started_at,
        recording_started_instant,
        mode,
        trace,
        Some(VadDetector::Scripted {
            frames,
            buffered_samples: 0,
            sample_offset: 0,
        }),
    )
    .await
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
async fn run_with_recorder_inner(
    provider: &dyn AsrProvider,
    params: SessionParams,
    control: SessionControl,
    rec: recorder::RecordingStream,
    recording_id: String,
    recording_started_at: time::OffsetDateTime,
    recording_started_instant: Instant,
    mode: RecordingMode,
    mut trace: RecordingObserver,
    vad_detector: Option<VadDetector>,
) -> Option<EngineOutcome> {
    let app_context =
        begin_recording_ui(&params, provider, &recording_id, recording_started_at, mode);
    let session_ctx = initial_session_ctx(&params);
    let opener = SessionOpener::new(provider, session_ctx);
    let initial = if mode == RecordingMode::Continuous {
        match open_initial_session(&recording_id, &opener, provider).await {
            Ok(opened) => opened,
            Err(error) => {
                tracing::error!(recording_id = %recording_id, error = %error, "ASR open failed");
                observe_asr_error(&mut trace, recording_started_instant, error);
                observe_finish_ms(&mut trace, "asr_open_error", 0);
                params.state.set_error(Some(recording_id));
                send_error_overlay(&params, crate::t!("error.asr_open"));
                return None;
            }
        }
        .into()
    } else {
        None
    };
    run_with_capture_stream(
        provider,
        params,
        control,
        CaptureStream::Cpal(rec),
        initial,
        app_context,
        recording_id,
        recording_started_at,
        recording_started_instant,
        mode,
        trace,
        vad_detector,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_with_capture_stream(
    provider: &dyn AsrProvider,
    params: SessionParams,
    control: SessionControl,
    mut rec: CaptureStream,
    initial: Option<OpenedSession>,
    mut app_context: post::AppContext,
    recording_id: String,
    recording_started_at: time::OffsetDateTime,
    recording_started_instant: Instant,
    mode: RecordingMode,
    mut trace: RecordingObserver,
    #[cfg_attr(not(test), allow(unused_variables))] vad_detector: Option<VadDetector>,
) -> Option<EngineOutcome> {
    let (mut session, mut events, mut active) = match initial {
        Some((session, events)) => (Some(session), Some(events), true),
        None => (None, None, false),
    };
    let mut vad_pause = if mode == RecordingMode::VadPause {
        let state = match vad_detector {
            Some(detector) => VadPauseState::with_detector(&params.vad, detector),
            None => VadPauseState::new(&params.vad),
        };
        match state {
            Ok(state) => Some(state),
            Err(error) => {
                tracing::error!(recording_id = %recording_id, error = ?error, "Silero VAD init failed");
                if let Some(active_session) = session.take() {
                    cleanup_started_session(&recording_id, active_session, &mut rec).await;
                } else {
                    discard_retained_audio(&recording_id, &mut rec).await;
                }
                observe_finish_ms(&mut trace, "vad_init_error", 0);
                params.state.set_error(Some(recording_id));
                send_error_overlay(&params, crate::t!("error.asr_runtime"));
                return None;
            }
        }
    } else {
        None
    };

    let session_ctx = initial_session_ctx(&params);
    let opener = SessionOpener::new(provider, session_ctx);

    match mode {
        RecordingMode::Continuous => {
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
        }
        RecordingMode::VadPause => {
            overlay_send(
                &params,
                OverlayCmd::SetState {
                    state: OverlayState::Idle,
                },
            );
            params
                .state
                .session_phase(recording_id.clone(), UiSessionPhase::Idle);
        }
    }

    let mut first_audio_deadline =
        TokioInstant::now() + Duration::from_millis(FIRST_AUDIO_TIMEOUT_MS);
    let mut first_audio_seen = false;
    let mut sessions = Vec::new();
    let mut current = CurrentSessionCapture::new(0);
    let mut session_index = 0u32;
    let mut total_audio_samples = 0u64;
    let mut last_sent_sample = 0u64;
    let mut stop_requested = false;
    let mut cancel_requested = false;
    let mut terminal_error = None;
    let mut meter = MeterCollector::new();

    'recording: loop {
        if active {
            if session.is_none() || events.is_none() {
                terminal_error = Some(HistoryError {
                    kind: "asr_session".to_string(),
                    msg: "active recording state missing ASR session".to_string(),
                });
                break 'recording;
            }
            let mut pause_requested = false;
            let mut provider_done = false;
            'active: loop {
                tokio::select! {
                    biased;
                    _ = control.cancelled() => {
                        cancel_requested = true;
                        break 'recording;
                    }
                    _ = control.stopped() => {
                        stop_requested = true;
                        break 'active;
                    }
                    pcm = rec.recv() => {
                        match pcm {
                            Ok(None) => {
                                stop_requested = true;
                                break 'active;
                            }
                            Ok(Some(samples)) => {
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
                                    for frame in vad.detector.accept(&samples) {
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
                            Err(error) => {
                                terminal_error = Some(capture_error(error));
                                break 'recording;
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
                    event = events.as_mut().expect("checked active events").recv() => {
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
                let Some(active_events) = events.as_mut() else {
                    terminal_error = Some(HistoryError {
                        kind: "asr_events".to_string(),
                        msg: "missing active ASR event stream".to_string(),
                    });
                    break 'recording;
                };
                match drain_stop_audio(
                    &mut rec,
                    active_session,
                    active_events,
                    &mut current,
                    vad_pause.as_mut(),
                    &mut total_audio_samples,
                    &mut last_sent_sample,
                    params.stop_delay_ms,
                    control.cancel_signal(),
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
                let Some(active_events) = events.as_mut() else {
                    terminal_error = Some(HistoryError {
                        kind: "asr_events".to_string(),
                        msg: "missing active ASR event stream".to_string(),
                    });
                    break 'recording;
                };
                match finalize_provider_session(
                    active_session,
                    active_events,
                    &mut current.segments,
                    &mut current.final_text,
                    &mut current.pending_overlay_segments,
                    params.finalize_timeout_ms,
                    control.cancel_signal(),
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

            // 进 idle 前用电平复核 control。finalize / drain 只观察 cancel，按设计拿不到
            // stop —— 一个在 finalize 窗口里发来的 stop 不会被任何人"消费"，但它存在于 stop
            // 闩里。这里读电平把它折进 stop_requested，否则会误入 idle（且 cancel 优先）。
            if control.is_cancelled() {
                cancel_requested = true;
                break 'recording;
            }
            if control.is_stop_requested() {
                stop_requested = true;
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
                    _ = control.cancelled() => {
                        cancel_requested = true;
                        break 'recording;
                    }
                    _ = control.stopped() => {
                        stop_requested = true;
                        break 'idle;
                    }
                    pcm = rec.recv() => {
                        match pcm {
                            Ok(None) => {
                                stop_requested = true;
                                break 'idle;
                            }
                            Ok(Some(samples)) => {
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
                                for frame in vad.detector.accept(&samples) {
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
                            Err(error) => {
                                terminal_error = Some(capture_error(error));
                                break 'recording;
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
            if control.is_cancelled() {
                cancel_requested = true;
                break 'recording;
            }
            if control.is_stop_requested() {
                refresh_stop_context(&params, &recording_id, &mut app_context);
                break 'recording;
            }
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
            let next_index = if sessions.is_empty() && current.audio_samples == 0 {
                0
            } else {
                session_index + 1
            };
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
            events = Some(new_events);
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
            first_audio_seen = true;
            first_audio_deadline =
                TokioInstant::now() + Duration::from_millis(FIRST_AUDIO_TIMEOUT_MS);
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
    let has_content = crate::voice::capture::has_archivable_content(&sessions);
    if !has_content {
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
    rec: &mut CaptureStream,
    session: &mut Box<dyn AsrSession>,
    events: &mut mpsc::Receiver<AsrEvent>,
    current: &mut CurrentSessionCapture,
    mut vad_pause: Option<&mut VadPauseState>,
    total_audio_samples: &mut u64,
    last_sent_sample: &mut u64,
    stop_delay_ms: u32,
    cancel: CancelSignal<'_>,
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
            _ = cancel.cancelled() => {
                *cancel_requested = true;
                return Ok(StopDrainOutcome::Canceled);
            }
            _ = sleep_until(TokioInstant::from_std(drain_until)) => break,
            pcm = rec.recv() => {
                match pcm {
                    Ok(Some(samples)) => {
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
                    Ok(None) => break,
                    Err(error) => return Err(capture_error(error)),
                }
            }
            event = events.recv() => {
                match event {
                    None => return Err(asr_stream_closed_error()),
                    Some(AsrEvent::Done) => {
                        observe_asr_event(trace, recording_started_instant, &AsrEvent::Done);
                        rec.stop();
                        while rec.try_recv().map_err(capture_error)?.is_some() {}
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

    let drained = rec.drain_after_stop().await.map_err(capture_error)?;
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
            profile: params.profile_name.clone(),
            profiles: params.profile_choices.clone(),
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

fn capture_error(error: anyhow::Error) -> HistoryError {
    HistoryError {
        kind: "capture".to_string(),
        msg: format!("{error:#}"),
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
    recorder: &mut CaptureStream,
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

async fn discard_retained_audio(recording_id: &str, recorder: &mut CaptureStream) {
    if let Err(error) = recorder.discard_audio().await {
        tracing::warn!(recording_id, error = ?error, "discard retained audio failed");
    }
}

async fn cleanup_started_session(
    recording_id: &str,
    session: Box<dyn AsrSession>,
    recorder: &mut CaptureStream,
) {
    let _ = session.close().await;
    discard_retained_audio(recording_id, recorder).await;
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
    use crate::voice::recorder::RecordingStream;

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

    #[derive(Default)]
    struct CloseTrackingSession {
        closes: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl AsrSession for CloseTrackingSession {
        async fn send_pcm(&mut self, _pcm: &[i16], _is_last: bool) -> Result<(), AsrError> {
            Ok(())
        }

        async fn close(self: Box<Self>) -> Result<(), AsrError> {
            self.closes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn test_session_ctx() -> SessionCtx {
        SessionCtx {
            language: LanguageMode::Single("zh-CN".to_string()),
            hotwords: Vec::new(),
        }
    }

    fn test_session_params(state: StateStore, overlay: Option<OverlayHandle>) -> SessionParams {
        SessionParams {
            auto_paste: false,
            record_audio: crate::config::RecordAudioMode::Off,
            preprocess: crate::config::VoicePreprocessCfg::default(),
            vad_trace: false,
            idle_pause: false,
            finalize_timeout_ms: 100,
            vad: crate::config::VoiceVadCfg::default(),
            stop_delay_ms: 0,
            hotwords: Vec::new(),
            start_app_context: post::AppContext {
                bundle_id: Some("com.example.Editor".to_string()),
                app_name: Some("Editor".to_string()),
            },
            profile_name: "Default".to_string(),
            profile_choices: vec![ProfileChoice {
                id: "default".to_string(),
                display_name: "Default".to_string(),
                asr_provider: "fake".to_string(),
                chain_summary: "default".to_string(),
            }],
            post_chain: post::PostChain {
                name: "default".to_string(),
                processors: Vec::new(),
            },
            post_timeout_ms: 100,
            overlay,
            state,
        }
    }

    #[test]
    fn begin_recording_ui_emits_connecting_without_capture_stream() {
        let provider = CountingProvider {
            opens: AtomicUsize::new(0),
        };
        let state = StateStore::new();
        let (overlay, mut overlay_rx) = OverlayHandle::channel();
        let params = test_session_params(state.clone(), Some(overlay));
        let recording_id = "test-recording".to_string();
        let started_at = time::OffsetDateTime::now_utc();

        let app_context = begin_recording_ui(
            &params,
            &provider,
            &recording_id,
            started_at,
            RecordingMode::Continuous,
        );

        assert_eq!(app_context.app_name.as_deref(), Some("Editor"));
        let snapshot = state.snapshot();
        assert_eq!(snapshot.recording_id.as_deref(), Some("test-recording"));
        assert_eq!(provider.opens.load(Ordering::SeqCst), 0);
        match overlay_rx.try_recv().unwrap() {
            OverlayCmd::SetState {
                state: OverlayState::Connecting,
            } => {}
            other => panic!("unexpected first overlay command: {other:?}"),
        }
    }

    #[tokio::test]
    async fn capture_and_initial_asr_open_are_joined_concurrently() {
        let (_pcm_tx, pcm_rx) = mpsc::unbounded_channel();
        let (_event_tx, events) = mpsc::channel(1);
        let capture = async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            Ok(CaptureStream::Cpal(RecordingStream::for_test(pcm_rx)))
        };
        let asr = async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            Ok::<OpenedSession, AsrError>((Box::new(CollectingSession::default()), events))
        };
        let started = Instant::now();
        let control = SessionControl::new();

        let (_rec, _session, _events) =
            match join_capture_and_asr(capture, asr, "test", &control).await {
                Ok(started) => started,
                Err(_) => panic!("expected capture and ASR open to succeed"),
            };

        assert!(
            started.elapsed() < Duration::from_millis(140),
            "capture/asr start should run concurrently, elapsed={:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn initial_asr_open_failure_discards_started_capture() {
        let (_pcm_tx, pcm_rx) = mpsc::unbounded_channel();
        let capture = async move { Ok(CaptureStream::Cpal(RecordingStream::for_test(pcm_rx))) };
        let asr = async {
            Err::<OpenedSession, AsrError>(AsrError::Network("scripted open failure".into()))
        };
        let control = SessionControl::new();

        let error = match join_capture_and_asr(capture, asr, "test", &control).await {
            Ok(_) => panic!("expected ASR open failure"),
            Err(error) => error,
        };

        match error {
            StartSessionError::Asr(AsrError::Network(message)) => {
                assert!(message.contains("scripted open failure"));
            }
            _ => panic!("expected ASR network error"),
        }
    }

    #[tokio::test]
    async fn capture_failure_does_not_wait_for_pending_asr_open() {
        let capture = async { Err::<CaptureStream, anyhow::Error>(anyhow::anyhow!("no input")) };
        let asr = async { std::future::pending::<Result<OpenedSession, AsrError>>().await };
        let control = SessionControl::new();

        let error = tokio::time::timeout(
            Duration::from_millis(50),
            join_capture_and_asr(capture, asr, "test", &control),
        )
        .await
        .expect("capture failure must return without waiting for ASR open");

        match error {
            Err(StartSessionError::Capture(error)) => {
                assert!(error.to_string().contains("no input"), "{error:#}");
            }
            _ => panic!("expected capture startup error"),
        }
    }

    #[tokio::test]
    async fn startup_cancel_before_capture_start_does_not_wait_for_asr_open() {
        let (_pcm_tx, pcm_rx) = mpsc::unbounded_channel();
        let capture = async move { Ok(CaptureStream::Cpal(RecordingStream::for_test(pcm_rx))) };
        let asr = async { std::future::pending::<Result<OpenedSession, AsrError>>().await };
        let control = SessionControl::new();
        control.request_cancel();

        let error = tokio::time::timeout(
            Duration::from_millis(50),
            join_capture_and_asr(capture, asr, "test", &control),
        )
        .await
        .expect("startup cancel must not wait for ASR open");

        assert!(matches!(error, Err(StartSessionError::Canceled)));
    }

    #[tokio::test]
    async fn startup_cancel_after_capture_start_does_not_wait_for_asr_open() {
        let (_pcm_tx, pcm_rx) = mpsc::unbounded_channel();
        let capture = async move { Ok(CaptureStream::Cpal(RecordingStream::for_test(pcm_rx))) };
        let asr = async { std::future::pending::<Result<OpenedSession, AsrError>>().await };
        let control = SessionControl::new();
        let cancel = control.clone();

        let started = join_capture_and_asr(capture, asr, "test", &control);
        tokio::pin!(started);
        tokio::task::yield_now().await;
        cancel.request_cancel();

        let error = tokio::time::timeout(Duration::from_millis(50), &mut started)
            .await
            .expect("startup cancel after capture start must not wait for ASR open");

        assert!(matches!(error, Err(StartSessionError::Canceled)));
    }

    #[tokio::test]
    async fn asr_open_failure_does_not_wait_for_pending_capture_start() {
        let capture = async { std::future::pending::<anyhow::Result<CaptureStream>>().await };
        let asr =
            async { Err::<OpenedSession, AsrError>(AsrError::Network("open denied".to_string())) };
        let control = SessionControl::new();

        let error = tokio::time::timeout(
            Duration::from_millis(50),
            join_capture_and_asr(capture, asr, "test", &control),
        )
        .await
        .expect("ASR open failure must not wait for capture start");

        match error {
            Err(StartSessionError::Asr(AsrError::Network(message))) => {
                assert!(message.contains("open denied"));
            }
            _ => panic!("expected ASR startup error"),
        }
    }

    #[tokio::test]
    async fn capture_failure_after_asr_open_closes_session() {
        let closes = Arc::new(AtomicUsize::new(0));
        let capture = async {
            tokio::task::yield_now().await;
            Err::<CaptureStream, anyhow::Error>(anyhow::anyhow!("no input"))
        };
        let (_event_tx, events) = mpsc::channel(1);
        let asr = async {
            Ok::<OpenedSession, AsrError>((
                Box::new(CloseTrackingSession {
                    closes: closes.clone(),
                }),
                events,
            ))
        };
        let control = SessionControl::new();

        let error = match join_capture_and_asr(capture, asr, "test", &control).await {
            Ok(_) => panic!("expected capture startup error"),
            Err(error) => error,
        };

        assert!(matches!(error, StartSessionError::Capture(_)));
        assert_eq!(closes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cleanup_started_session_closes_asr_session() {
        let closes = Arc::new(AtomicUsize::new(0));
        let session: Box<dyn AsrSession> = Box::new(CloseTrackingSession {
            closes: closes.clone(),
        });
        let (_pcm_tx, pcm_rx) = mpsc::unbounded_channel();
        let mut rec = CaptureStream::Cpal(RecordingStream::for_test(pcm_rx));

        cleanup_started_session("test", session, &mut rec).await;

        assert_eq!(closes.load(Ordering::SeqCst), 1);
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
