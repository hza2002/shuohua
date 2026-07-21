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
use crate::voice::finalize::{
    emit_stats, finalize_provider_session, FinalizeOutcome, TranscriptDisplay,
};
use crate::voice::meter::MeterCollector;
use crate::voice::observer::{
    instant_elapsed_ms, observe_asr_error, observe_asr_event, observe_asr_event_at, observe_finish,
    observe_finish_ms, observe_pcm, observe_provider_opened, observe_session, RecordingObserver,
    SessionPhase, TraceStart,
};
use crate::voice::{audio, recorder, CancelSignal, SessionControl};
use tokio::sync::mpsc;
use tokio::time::{sleep_until, Instant as TokioInstant};

const FIRST_AUDIO_TIMEOUT_MS: u64 = 1000;
const MIN_NONZERO_AMPLITUDE: i16 = 8;
const STOP_RESIDUAL_DRAIN_TIMEOUT_MS: u64 = 1000;
const NOTICE_TTL_MS: u32 = 3_000;
const STARTUP_SIGNAL_LOOKBACK_MS: u32 = 1_200;
const STARTUP_SIGNAL_MARGIN_MS: u32 = 120;
const ASR_SEND_SLOW_WARN_MS: u128 = 80;
const ASR_SEND_TIMEOUT: Duration = crate::asr::providers::SESSION_IO_TIMEOUT;

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

struct StartedSession {
    rec: CaptureStream,
    session: Box<dyn AsrSession>,
    events: mpsc::Receiver<AsrEvent>,
    startup_pcm: Vec<Vec<i16>>,
}

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
    first_signal_sample: Option<u64>,
}

struct CaptureDrain<T> {
    output: T,
    drained_pcm: Vec<Vec<i16>>,
    capture_eof: bool,
    capture_error: Option<anyhow::Error>,
    interrupted: Option<CaptureDrainInterrupt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureDrainInterrupt {
    Cancel,
    Stop,
}

#[derive(Debug, Default)]
struct CaptureDiagnostics {
    chunks: u64,
    samples: u64,
    first_pcm_ms: Option<u64>,
    first_signal_ms: Option<u64>,
    max_peak_abs: u16,
}

impl CaptureDiagnostics {
    fn observe(
        &mut self,
        samples: &[i16],
        recording_started_instant: Instant,
        recording_id: &str,
        mode: RecordingMode,
        backend: crate::config::VoicePreprocessBackend,
    ) {
        self.chunks += 1;
        self.samples += samples.len() as u64;
        let t_ms = instant_elapsed_ms(recording_started_instant);
        if self.first_pcm_ms.is_none() {
            self.first_pcm_ms = Some(t_ms);
            tracing::debug!(
                recording_id = %recording_id,
                mode = ?mode,
                backend = ?backend,
                chunk_samples = samples.len(),
                timeline_ms = samples_to_ms(self.samples),
                t_ms,
                "capture first PCM observed by engine"
            );
        }

        let peak_abs = samples
            .iter()
            .map(|sample| sample.unsigned_abs())
            .max()
            .unwrap_or(0);
        self.max_peak_abs = self.max_peak_abs.max(peak_abs);
        if self.first_signal_ms.is_none() && peak_abs > MIN_NONZERO_AMPLITUDE as u16 {
            self.first_signal_ms = Some(t_ms);
            tracing::debug!(
                recording_id = %recording_id,
                mode = ?mode,
                backend = ?backend,
                chunk_samples = samples.len(),
                peak_abs,
                timeline_ms = samples_to_ms(self.samples),
                t_ms,
                "capture first non-silent PCM observed by engine"
            );
        }
    }

    fn log_summary(
        &self,
        recording_id: &str,
        mode: RecordingMode,
        backend: crate::config::VoicePreprocessBackend,
    ) {
        tracing::info!(
            recording_id = %recording_id,
            mode = ?mode,
            backend = ?backend,
            chunks = self.chunks,
            samples = self.samples,
            audio_ms = samples_to_ms(self.samples),
            first_pcm_ms = self.first_pcm_ms,
            first_signal_ms = self.first_signal_ms,
            max_peak_abs = self.max_peak_abs,
            "capture diagnostics summary"
        );
    }
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
        let retention_ms = startup_timeline_retention_ms(config);
        Ok(Self {
            detector,
            controller,
            timeline: PcmTimeline::new(retention_ms),
            pre_roll_samples: ms_to_samples(config.pre_roll_ms),
            max_overlap_samples: ms_to_samples(config.max_overlap_ms),
            first_signal_sample: None,
        })
    }

    fn push_idle_pcm(&mut self, samples: &[i16]) {
        let chunk = self.timeline.push(samples);
        if self.first_signal_sample.is_none() {
            if let Some(offset) = first_signal_offset(samples) {
                self.first_signal_sample = Some(chunk.start_sample + offset as u64);
            }
        }
    }
}

struct CurrentSessionCapture {
    start_sample: u64,
    audio_samples: u64,
    segments: Vec<SegmentCapture>,
    final_text: Option<String>,
    partial_text: String,
    pending_overlay_segments: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PartialTextPolicy {
    DiscardTentative,
    PreserveRecoverableSnapshot,
}

impl PartialTextPolicy {
    fn preserves_partial(self) -> bool {
        matches!(self, Self::PreserveRecoverableSnapshot)
    }
}

impl CurrentSessionCapture {
    fn new(start_sample: u64) -> Self {
        Self {
            start_sample,
            audio_samples: 0,
            segments: Vec::new(),
            final_text: None,
            partial_text: String::new(),
            pending_overlay_segments: 0,
        }
    }

    fn record_sent_samples(&mut self, samples: u64) {
        self.audio_samples += samples;
    }

    fn into_session(
        self,
        recording_started: Instant,
        partial_text_policy: PartialTextPolicy,
    ) -> Option<SessionCapture> {
        let preserve_partial_text = partial_text_policy.preserves_partial();
        if self.audio_samples == 0
            && self.segments.is_empty()
            && self.final_text.as_deref().unwrap_or("").is_empty()
            && (!preserve_partial_text || self.partial_text.is_empty())
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
            partial_text: (preserve_partial_text && !self.partial_text.is_empty())
                .then_some(self.partial_text),
        })
    }
}

pub struct SessionParams {
    pub auto_paste: bool,
    pub record_audio: crate::config::RecordAudioMode,
    pub preprocess: crate::config::VoicePreprocessCfg,
    pub vad_trace: bool,
    pub apple_backend_trace: bool,
    pub idle_pause: bool,
    pub open_timeout_ms: u64,
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
    pub start: crate::voice::resume::RecordingStart,
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
    Apple(Box<AppleCapture>),
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
            Self::Apple(source) => source.try_recv(),
        }
    }

    fn stop(&mut self) {
        match self {
            Self::Cpal(recorder) => recorder.stop(),
            #[cfg(target_os = "macos")]
            Self::Apple(source) => source.stop(),
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
            Self::Apple(source) => source.finish_audio().await,
        }
    }

    async fn discard_audio(&mut self) -> anyhow::Result<()> {
        match self {
            Self::Cpal(recorder) => recorder.discard_audio().await,
            #[cfg(target_os = "macos")]
            Self::Apple(source) => source.discard_audio().await,
        }
    }
}

#[cfg(target_os = "macos")]
enum AppleCaptureState {
    Apple(crate::voice::apple_source::RunningAppleVpSource),
    StoppingApple {
        stop_drain: AppleStopDrainTask,
        residual: std::collections::VecDeque<Vec<i16>>,
    },
    RawFallback(recorder::RecordingStream),
    Done,
}

#[cfg(target_os = "macos")]
struct AppleCapture {
    recording_id: String,
    state: AppleCaptureState,
}

#[cfg(target_os = "macos")]
struct AppleStopDrainTask {
    join: Option<tokio::task::JoinHandle<anyhow::Result<Vec<Vec<i16>>>>>,
}

#[cfg(target_os = "macos")]
impl AppleCapture {
    const APPLE_STOP_TIMEOUT: Duration = Duration::from_millis(500);

    fn apple(
        recording_id: impl Into<String>,
        apple: crate::voice::apple_source::RunningAppleVpSource,
    ) -> Self {
        Self {
            recording_id: recording_id.into(),
            state: AppleCaptureState::Apple(apple),
        }
    }

    fn raw_fallback(recording_id: impl Into<String>, raw: recorder::RecordingStream) -> Self {
        Self {
            recording_id: recording_id.into(),
            state: AppleCaptureState::RawFallback(raw),
        }
    }

    async fn recv(&mut self) -> anyhow::Result<Option<Vec<i16>>> {
        match &mut self.state {
            AppleCaptureState::Apple(apple) => apple.recv().await,
            AppleCaptureState::StoppingApple { .. } => self.recv_stopping_apple().await,
            AppleCaptureState::RawFallback(raw) => Ok(raw.recv().await),
            AppleCaptureState::Done => Ok(None),
        }
    }

    fn try_recv(&mut self) -> anyhow::Result<Option<Vec<i16>>> {
        match &mut self.state {
            AppleCaptureState::RawFallback(raw) => Ok(raw.try_recv()),
            AppleCaptureState::StoppingApple { residual, .. } => Ok(residual.pop_front()),
            AppleCaptureState::Apple(_) | AppleCaptureState::Done => Ok(None),
        }
    }

    fn stop(&mut self) {
        let state = std::mem::replace(&mut self.state, AppleCaptureState::Done);
        self.state = match state {
            AppleCaptureState::Apple(apple) => AppleCaptureState::StoppingApple {
                stop_drain: AppleStopDrainTask::start(apple),
                residual: std::collections::VecDeque::new(),
            },
            AppleCaptureState::RawFallback(mut raw) => {
                raw.stop();
                AppleCaptureState::RawFallback(raw)
            }
            other => other,
        };
    }

    async fn recv_stopping_apple(&mut self) -> anyhow::Result<Option<Vec<i16>>> {
        let AppleCaptureState::StoppingApple {
            stop_drain,
            residual,
        } = &mut self.state
        else {
            unreachable!("recv_stopping_apple called outside stopping state");
        };

        if let Some(samples) = residual.pop_front() {
            return Ok(Some(samples));
        }

        let result = stop_drain.take_completed().await;
        self.state = AppleCaptureState::Done;
        match result {
            Ok(frames) => {
                let mut frames = std::collections::VecDeque::from(frames);
                Ok(frames.pop_front())
            }
            Err(error) => {
                tracing::warn!(
                    recording_id = %self.recording_id,
                    error = ?error,
                    "Apple voice processing stop drain failed"
                );
                Ok(None)
            }
        }
    }

    async fn drain_after_stop(&mut self) -> anyhow::Result<Vec<Vec<i16>>> {
        self.stop();
        match &mut self.state {
            AppleCaptureState::RawFallback(raw) => Ok(raw.drain_after_stop().await),
            _ => {
                let mut out = Vec::new();
                while let Some(samples) = self.recv().await? {
                    out.push(samples);
                }
                Ok(out)
            }
        }
    }

    async fn finish_audio(&mut self) -> anyhow::Result<Option<std::path::PathBuf>> {
        self.finish_or_discard_cleanup().await;
        Ok(None)
    }

    async fn discard_audio(&mut self) -> anyhow::Result<()> {
        self.finish_or_discard_cleanup().await;
        Ok(())
    }

    async fn finish_or_discard_cleanup(&mut self) {
        match std::mem::replace(&mut self.state, AppleCaptureState::Done) {
            AppleCaptureState::Apple(mut apple) => {
                apple.request_stop();
            }
            AppleCaptureState::StoppingApple { mut stop_drain, .. } => {
                if tokio::time::timeout(Self::APPLE_STOP_TIMEOUT, stop_drain.take_completed())
                    .await
                    .is_err()
                {
                    stop_drain.abort();
                }
            }
            AppleCaptureState::RawFallback(mut raw) => {
                let _ = raw.discard_audio().await;
            }
            AppleCaptureState::Done => {}
        }
    }
}

#[cfg(target_os = "macos")]
impl AppleStopDrainTask {
    fn start(mut apple: crate::voice::apple_source::RunningAppleVpSource) -> Self {
        let join = tokio::spawn(async move {
            tokio::time::timeout(AppleCapture::APPLE_STOP_TIMEOUT, apple.drain_after_stop())
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("apple stop drain timed out")))
        });
        Self { join: Some(join) }
    }

    async fn take_completed(&mut self) -> anyhow::Result<Vec<Vec<i16>>> {
        let result = match self.join.as_mut() {
            Some(join) => join.await.unwrap_or_else(|error| Err(error.into())),
            None => Ok(Vec::new()),
        };
        self.join = None;
        result
    }

    fn abort(&mut self) {
        if let Some(join) = self.join.as_ref() {
            join.abort();
        }
    }
}

pub(crate) async fn run(
    provider: &dyn AsrProvider,
    params: SessionParams,
    control: SessionControl,
) -> Option<EngineOutcome> {
    let mode = RecordingMode::select(params.idle_pause, &params.vad);
    let recording_id = ulid::Ulid::generate().to_string();
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
    let (rec, initial, startup_pcm) = match mode {
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
                Ok(started) => (
                    started.rec,
                    Some((started.session, started.events)),
                    started.startup_pcm,
                ),
                Err(StartSessionError::Capture(error)) => {
                    tracing::error!(recording_id = %recording_id, error = ?error, "recorder start failed");
                    observe_finish_ms(&mut trace, "recorder_start_error", 0);
                    params.state.set_error(Some(recording_id));
                    send_error_overlay(&params, crate::t!("error.recorder_start"));
                    return None;
                }
                Err(StartSessionError::Asr(error)) => {
                    let message = asr_open_error_message(&error);
                    observe_asr_error(&mut trace, recording_started_instant, error);
                    observe_finish_ms(&mut trace, "asr_open_error", 0);
                    params.state.set_error(Some(recording_id));
                    send_error_overlay(&params, message);
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
            Ok(rec) => (rec, None, Vec::new()),
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
        startup_pcm,
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

#[derive(Debug)]
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
) -> Result<StartedSession, StartSessionError> {
    let capture = start_capture_stream(params, recording_id);
    let asr = open_initial_session(recording_id, opener, provider, params.open_timeout_ms);
    join_capture_and_asr(capture, asr, recording_id, control).await
}

async fn join_capture_and_asr<C, A>(
    capture: C,
    asr: A,
    recording_id: &str,
    control: &SessionControl,
) -> Result<StartedSession, StartSessionError>
where
    C: std::future::Future<Output = anyhow::Result<CaptureStream>>,
    A: std::future::Future<Output = Result<OpenedSession, crate::asr::types::AsrError>>,
{
    tokio::pin!(capture);
    tokio::pin!(asr);

    tokio::select! {
        biased;
        _ = control.cancelled() => Err(StartSessionError::Canceled),
        result = &mut capture => {
            match result {
                Ok(rec) => finish_startup_after_capture(rec, asr, recording_id, control).await,
                Err(error) => Err(StartSessionError::Capture(error)),
            }
        }
        _ = control.stopped() => Err(StartSessionError::Canceled),
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
) -> Result<StartedSession, StartSessionError>
where
    A: std::future::Future<Output = Result<OpenedSession, crate::asr::types::AsrError>>,
{
    let mut startup_pcm = Vec::new();
    let mut stop_requested = false;
    tokio::pin!(asr);

    loop {
        tokio::select! {
            biased;
            _ = control.cancelled() => {
                discard_retained_audio(recording_id, &mut rec).await;
                return Err(StartSessionError::Canceled);
            }
            _ = control.stopped(), if !stop_requested => {
                stop_requested = true;
                rec.stop();
                match rec.drain_after_stop().await {
                    Ok(drained) => startup_pcm.extend(drained),
                    Err(error) => {
                        discard_retained_audio(recording_id, &mut rec).await;
                        return Err(StartSessionError::Capture(error));
                    }
                }
            }
            pcm = rec.recv(), if !stop_requested => {
                match pcm {
                    Ok(Some(samples)) => startup_pcm.push(samples),
                    Ok(None) => {
                        stop_requested = true;
                    }
                    Err(error) => {
                        discard_retained_audio(recording_id, &mut rec).await;
                        return Err(StartSessionError::Capture(error));
                    }
                }
            }
            result = &mut asr => {
                match result {
                    Ok((session, events)) => {
                        return Ok(StartedSession {
                            rec,
                            session,
                            events,
                            startup_pcm,
                        });
                    }
                    Err(error) => {
                        discard_retained_audio(recording_id, &mut rec).await;
                        return Err(StartSessionError::Asr(error));
                    }
                }
            }
        }
    }
}

async fn finish_startup_after_asr<C>(
    opened: OpenedSession,
    capture: std::pin::Pin<&mut C>,
    control: &SessionControl,
) -> Result<StartedSession, StartSessionError>
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
                Ok(rec) => Ok(StartedSession {
                    rec,
                    session,
                    events,
                    startup_pcm: Vec::new(),
                }),
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
    log_recording_input(recording_id, backend);
    let started = Instant::now();
    let result = match backend {
        crate::config::VoicePreprocessBackend::Off => {
            let audio_output = prepare_audio_output(params, recording_id);
            recorder::start(audio_output, backend, params.apple_backend_trace)
                .map(CaptureStream::Cpal)
        }
        crate::config::VoicePreprocessBackend::WebRtc => {
            let audio_output = prepare_audio_output(params, recording_id);
            recorder::start(audio_output, backend, params.apple_backend_trace)
                .map(CaptureStream::Cpal)
        }
        crate::config::VoicePreprocessBackend::Apple => {
            start_apple_capture_stream(recording_id, params.apple_backend_trace).await
        }
    };
    if result.is_ok() {
        tracing::info!(
            recording_id,
            backend = ?backend,
            duration_ms = started.elapsed().as_millis() as u64,
            "capture stream started"
        );
    } else if let Err(error) = &result {
        tracing::warn!(
            recording_id,
            backend = ?backend,
            duration_ms = started.elapsed().as_millis() as u64,
            error = ?error,
            "capture stream start failed"
        );
    }
    result
}

/// 每次录音起始锚点日志：cpal 实际探到的 default input（backend=apple 时反映
/// 打开 voice processing 之前的设备）。channels 异常（如 VP aggregate 残留导致
/// 的 3ch）在这里第一时间可见，是 failure chain 的起点。非 dev、恒进日志。
fn log_recording_input(recording_id: &str, backend: crate::config::VoicePreprocessBackend) {
    match recorder::probe_default_input() {
        Ok(info) => tracing::info!(
            recording_id,
            backend = ?backend,
            device = info.name.as_deref().unwrap_or("<unknown>"),
            sample_rate = info.sample_rate,
            channels = info.channels,
            sample_format = ?info.sample_format,
            "recording input device"
        ),
        Err(error) => tracing::debug!(
            recording_id,
            backend = ?backend,
            error = ?error,
            "recording input device probe failed"
        ),
    }
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
    timeout_ms: u64,
) -> Result<OpenedSession, crate::asr::types::AsrError> {
    let asr_open_started = Instant::now();
    match open_with_timeout(opener.open_initial(), timeout_ms).await {
        Ok(opened) => {
            tracing::info!(
                recording_id = %recording_id,
                provider = %provider.name(),
                duration_ms = asr_open_started.elapsed().as_millis() as u64,
                timeout_ms,
                "ASR session opened"
            );
            Ok(opened)
        }
        Err(error) => {
            tracing::error!(
                recording_id = %recording_id,
                provider = %provider.name(),
                duration_ms = asr_open_started.elapsed().as_millis() as u64,
                timeout_ms,
                error = %error,
                "ASR open failed"
            );
            Err(error)
        }
    }
}

async fn open_resume_session(
    recording_id: &str,
    opener: &SessionOpener<'_>,
    mode: RecordingMode,
    index: u32,
    timeout_ms: u64,
) -> Result<Option<OpenedSession>, crate::asr::types::AsrError> {
    let asr_open_started = Instant::now();
    match tokio::time::timeout(Duration::from_millis(timeout_ms), opener.open_resume(mode)).await {
        Ok(Ok(Some(opened))) => {
            tracing::info!(
                recording_id = %recording_id,
                session_index = index,
                duration_ms = asr_open_started.elapsed().as_millis() as u64,
                timeout_ms,
                "ASR resume session opened"
            );
            Ok(Some(opened))
        }
        Ok(Ok(None)) => Ok(None),
        Ok(Err(error)) => {
            tracing::warn!(
                recording_id = %recording_id,
                session_index = index,
                duration_ms = asr_open_started.elapsed().as_millis() as u64,
                timeout_ms,
                error = %error,
                "ASR resume session open failed"
            );
            Err(error)
        }
        Err(_) => {
            tracing::warn!(
                recording_id = %recording_id,
                session_index = index,
                duration_ms = asr_open_started.elapsed().as_millis() as u64,
                timeout_ms,
                "ASR resume session open timed out"
            );
            Err(crate::asr::types::AsrError::OpenTimeout)
        }
    }
}

async fn open_with_timeout<F>(
    open: F,
    timeout_ms: u64,
) -> Result<OpenedSession, crate::asr::types::AsrError>
where
    F: std::future::Future<Output = Result<OpenedSession, crate::asr::types::AsrError>>,
{
    match tokio::time::timeout(Duration::from_millis(timeout_ms), open).await {
        Ok(result) => result,
        Err(_) => Err(crate::asr::types::AsrError::OpenTimeout),
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

/// 录音起始时按「开始方式」发 overlay 提示。必须在 `SetState(Connecting)` 清屏
/// 之后调用——在 daemon 侧先发会被随后的 Connecting 清掉。
///
/// - resume 续写（`Seed`）：把旧 ASR 文本作为已提交 segment 铺到 overlay / state，
///   让用户直观看到「接着上次继续说」，并发「继续上一段」notice。seed 只用于展示
///   —— 不进 capture sessions（finish 另行把 seed 拼进 history/post），也不占
///   provider audio，因此不碰 `current`。
/// - resume 但无可恢复（`NewFromResume`）：只发「新录音」notice，告诉用户热键生效、
///   确实没有可续写内容。
/// - 普通开始（`Fresh`）：不发提示。
fn apply_start_notice(
    params: &SessionParams,
    recording_id: &str,
    recording_started_instant: Instant,
    transcript: &mut TranscriptDisplay,
) {
    use crate::voice::resume::RecordingStart;
    match &params.start {
        RecordingStart::Fresh => {}
        RecordingStart::NewFromResume => {
            overlay_send(
                params,
                OverlayCmd::Notice {
                    text: crate::t!("notice.new_recording"),
                    ttl_ms: NOTICE_TTL_MS,
                },
            );
        }
        RecordingStart::Seed(seed) => {
            let Some(seed_text) = seed.non_empty_text() else {
                return;
            };
            transcript.append_segment(seed_text.to_string());
            params
                .state
                .segment(recording_id.to_string(), seed_text.to_string());
            overlay_send(
                params,
                OverlayCmd::AppendSegment {
                    text: seed_text.to_string(),
                },
            );
            overlay_send(
                params,
                OverlayCmd::Notice {
                    text: crate::t!("notice.resume_recording"),
                    ttl_ms: NOTICE_TTL_MS,
                },
            );
            emit_stats(
                transcript,
                recording_started_instant,
                &params.state,
                recording_id,
                params.overlay.as_ref(),
            );
        }
    }
}

#[cfg(target_os = "macos")]
async fn start_apple_capture_stream(
    recording_id: &str,
    apple_backend_trace: bool,
) -> anyhow::Result<CaptureStream> {
    let source = match crate::voice::apple_source::AppleVpSource::prepare_helper() {
        Ok(source) => source,
        Err(error) => {
            if let Ok(raw) = recorder::start(
                None,
                crate::config::VoicePreprocessBackend::Off,
                apple_backend_trace,
            ) {
                tracing::warn!(
                    recording_id,
                    error = ?error,
                    "Apple voice processing helper unavailable; continuing with raw cpal capture"
                );
                return Ok(CaptureStream::Apple(Box::new(AppleCapture::raw_fallback(
                    recording_id,
                    raw,
                ))));
            }
            return Err(error);
        }
    };
    match source.start().await {
        Ok(apple) => Ok(CaptureStream::Apple(Box::new(AppleCapture::apple(
            recording_id,
            apple,
        )))),
        Err(error) => {
            if let Ok(raw) = recorder::start(
                None,
                crate::config::VoicePreprocessBackend::Off,
                apple_backend_trace,
            ) {
                tracing::warn!(
                    recording_id,
                    error = ?error,
                    "Apple voice processing startup failed; continuing with raw cpal capture"
                );
                return Ok(CaptureStream::Apple(Box::new(AppleCapture::raw_fallback(
                    recording_id,
                    raw,
                ))));
            }
            Err(error)
        }
    }
}

#[cfg(not(target_os = "macos"))]
async fn start_apple_capture_stream(
    _recording_id: &str,
    _apple_backend_trace: bool,
) -> anyhow::Result<CaptureStream> {
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
        match open_initial_session(&recording_id, &opener, provider, params.open_timeout_ms).await {
            Ok(opened) => opened,
            Err(error) => {
                let message = asr_open_error_message(&error);
                tracing::error!(recording_id = %recording_id, error = %error, "ASR open failed");
                observe_asr_error(&mut trace, recording_started_instant, error);
                observe_finish_ms(&mut trace, "asr_open_error", 0);
                params.state.set_error(Some(recording_id));
                send_error_overlay(&params, message);
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
        Vec::new(),
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
    startup_pcm: Vec<Vec<i16>>,
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
    let mut transcript = TranscriptDisplay::new();
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

    apply_start_notice(
        &params,
        &recording_id,
        recording_started_instant,
        &mut transcript,
    );

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
    let mut capture_diag = CaptureDiagnostics::default();
    let mut queued_idle_pcm = Vec::new();

    if active && !startup_pcm.is_empty() {
        match session.as_mut() {
            Some(active_session) => {
                for samples in startup_pcm {
                    capture_diag.observe(
                        &samples,
                        recording_started_instant,
                        &recording_id,
                        mode,
                        params.preprocess.backend,
                    );
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
                    if let Err(error) =
                        send_pcm_chunk(active_session, &samples, &mut total_audio_samples).await
                    {
                        terminal_error = Some(error);
                        break;
                    }
                    current.record_sent_samples(samples.len() as u64);
                    last_sent_sample = end_sample;
                }
            }
            None => {
                terminal_error = Some(HistoryError {
                    kind: "asr_session".to_string(),
                    msg: "active recording state missing ASR session".to_string(),
                });
            }
        }
    }

    'recording: loop {
        if terminal_error.is_some() {
            break 'recording;
        }
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
                                capture_diag.observe(
                                    &samples,
                                    recording_started_instant,
                                    &recording_id,
                                    mode,
                                    params.preprocess.backend,
                                );
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
                                tracing::error!(
                                    recording_id,
                                    backend = ?params.preprocess.backend,
                                    mode = ?mode,
                                    active,
                                    total_audio_ms = samples_to_ms(total_audio_samples),
                                    error = ?error,
                                    "capture stream failed"
                                );
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
                                    &mut transcript,
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
                    &mut transcript,
                    &params,
                    recording_started_instant,
                    &mut capture_diag,
                    mode,
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

            let mut pending_idle_pcm = Vec::new();
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
                let mut finalize_events = Vec::new();
                let drain = drain_capture_while(
                    &mut rec,
                    &mut capture_diag,
                    &mut trace,
                    &params,
                    &recording_id,
                    &mut meter,
                    recording_started_instant,
                    mode,
                    None,
                    finalize_provider_session(
                        active_session,
                        active_events,
                        &mut current.segments,
                        &mut current.final_text,
                        &mut current.partial_text,
                        &mut current.pending_overlay_segments,
                        params.finalize_timeout_ms,
                        control.cancel_signal(),
                        &mut terminal_error,
                        recording_started_instant,
                        &mut finalize_events,
                        &mut transcript,
                        &params.state,
                        &recording_id,
                        params.overlay.as_ref(),
                    ),
                )
                .await;
                for (t_ms, event) in &finalize_events {
                    observe_asr_event_at(&mut trace, *t_ms, event);
                }
                debug_assert!(drain.interrupted.is_none());
                if drain.capture_eof {
                    stop_requested = true;
                }
                if mode == RecordingMode::VadPause {
                    pending_idle_pcm.extend(drain.drained_pcm);
                }
                if let Some(error) = drain.capture_error {
                    terminal_error = Some(capture_error(error));
                    break 'recording;
                }
                match drain.output {
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
                let drain = drain_capture_while(
                    &mut rec,
                    &mut capture_diag,
                    &mut trace,
                    &params,
                    &recording_id,
                    &mut meter,
                    recording_started_instant,
                    mode,
                    None,
                    active_session.close(),
                )
                .await;
                debug_assert!(drain.interrupted.is_none());
                if drain.capture_eof {
                    stop_requested = true;
                }
                if mode == RecordingMode::VadPause {
                    pending_idle_pcm.extend(drain.drained_pcm);
                }
                if let Some(error) = drain.capture_error {
                    terminal_error = Some(capture_error(error));
                    break 'recording;
                }
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
                    .into_session(
                        recording_started_instant,
                        PartialTextPolicy::DiscardTentative,
                    )
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
            queued_idle_pcm = pending_idle_pcm;
            active = false;
        } else {
            let mut speech_start = None;
            if !queued_idle_pcm.is_empty() {
                let queued = std::mem::take(&mut queued_idle_pcm);
                let Some(vad) = vad_pause.as_mut() else {
                    terminal_error = Some(HistoryError {
                        kind: "vad_state".to_string(),
                        msg: "VadPause mode missing VAD state".to_string(),
                    });
                    break 'recording;
                };
                speech_start = consume_pending_idle_pcm(&queued, vad, &mut meter);
            }
            'idle: loop {
                if speech_start.is_some() {
                    break 'idle;
                }
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
                                capture_diag.observe(
                                    &samples,
                                    recording_started_instant,
                                    &recording_id,
                                    mode,
                                    params.preprocess.backend,
                                );
                                observe_pcm(&mut trace, &samples);
                                emit_meters(&params, &recording_id, &mut meter, &samples);
                                let Some(vad) = vad_pause.as_mut() else {
                                    terminal_error = Some(HistoryError {
                                        kind: "vad_state".to_string(),
                                        msg: "VadPause mode missing VAD state".to_string(),
                                    });
                                    break 'recording;
                                };
                                vad.push_idle_pcm(&samples);
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
                                        log_vad_pause_resume_step(
                                            &params,
                                            &recording_id,
                                            recording_started_instant,
                                            "speech_started",
                                            [
                                                ("speech_start_sample", frame.start_sample),
                                                ("speech_start_ms", samples_to_ms(frame.start_sample)),
                                                ("chunk_samples", samples.len() as u64),
                                                ("timeline_end", vad.timeline.end_sample()),
                                            ],
                                        );
                                        break;
                                    }
                                }
                                if speech_start.is_some() {
                                    break 'idle;
                                }
                            }
                            Err(error) => {
                                tracing::error!(
                                    recording_id,
                                    backend = ?params.preprocess.backend,
                                    mode = ?mode,
                                    active,
                                    total_audio_ms = samples_to_ms(total_audio_samples),
                                    error = ?error,
                                    "capture stream failed"
                                );
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
            let Some(vad) = vad_pause.as_ref() else {
                break 'recording;
            };
            let is_first_session = sessions.is_empty() && current.audio_samples == 0;
            let startup_signal_sample = if is_first_session {
                vad.first_signal_sample
            } else {
                None
            };
            let next_index = if sessions.is_empty() && current.audio_samples == 0 {
                0
            } else {
                session_index + 1
            };
            let resume_open_started = Instant::now();
            log_vad_pause_resume_step(
                &params,
                &recording_id,
                recording_started_instant,
                "open_start",
                [
                    ("session_index", next_index as u64),
                    ("speech_start_sample", speech_start),
                    ("speech_start_ms", samples_to_ms(speech_start)),
                    ("last_sent_sample", last_sent_sample),
                ],
            );
            let drain = drain_capture_while(
                &mut rec,
                &mut capture_diag,
                &mut trace,
                &params,
                &recording_id,
                &mut meter,
                recording_started_instant,
                mode,
                Some(&control),
                open_resume_session(
                    &recording_id,
                    &opener,
                    mode,
                    next_index,
                    params.open_timeout_ms,
                ),
            )
            .await;
            log_vad_pause_resume_step(
                &params,
                &recording_id,
                recording_started_instant,
                "open_done",
                [
                    ("session_index", next_index as u64),
                    (
                        "open_elapsed_ms",
                        resume_open_started.elapsed().as_millis() as u64,
                    ),
                    ("drained_chunks", drain.drained_pcm.len() as u64),
                    (
                        "drained_samples",
                        drain.drained_pcm.iter().map(|pcm| pcm.len() as u64).sum(),
                    ),
                ],
            );
            if let Some(interrupted) = drain.interrupted {
                if let Ok(Some((session, _))) = drain.output {
                    let _ = session.close().await;
                }
                match interrupted {
                    CaptureDrainInterrupt::Cancel => cancel_requested = true,
                    CaptureDrainInterrupt::Stop => {
                        refresh_stop_context(&params, &recording_id, &mut app_context);
                    }
                }
                break 'recording;
            }
            if drain.capture_eof {
                stop_requested = true;
            }
            if !drain.drained_pcm.is_empty() {
                let Some(vad) = vad_pause.as_mut() else {
                    terminal_error = Some(HistoryError {
                        kind: "vad_state".to_string(),
                        msg: "VadPause mode missing VAD state".to_string(),
                    });
                    break 'recording;
                };
                let _ = consume_pending_idle_pcm(&drain.drained_pcm, vad, &mut meter);
            }
            let (new_session, new_events) = match drain.output {
                Ok(Some(opened)) => {
                    if let Some(error) = drain.capture_error {
                        let (session, _) = opened;
                        let _ = session.close().await;
                        terminal_error = Some(capture_error(error));
                        break 'recording;
                    }
                    opened
                }
                Ok(None) => {
                    if let Some(error) = drain.capture_error {
                        terminal_error = Some(capture_error(error));
                        break 'recording;
                    }
                    terminal_error = Some(HistoryError {
                        kind: "asr_resume_mode".to_string(),
                        msg: "Continuous mode cannot open a resume session".to_string(),
                    });
                    break 'recording;
                }
                Err(error) => {
                    if let Some(capture_err) = drain.capture_error {
                        terminal_error = Some(capture_error(capture_err));
                        break 'recording;
                    }
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
            let Some(vad) = vad_pause.as_mut() else {
                break 'recording;
            };
            let oldest_sample = vad.timeline.oldest_sample();
            let send_start = compute_resume_start_sample(
                speech_start,
                vad.pre_roll_samples,
                last_sent_sample,
                vad.max_overlap_samples,
                oldest_sample,
                startup_signal_sample,
            );
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
            let replay_clamped = replay.start_sample > send_start;
            log_vad_pause_replay_window(
                &recording_id,
                params.preprocess.backend,
                next_index,
                speech_start,
                send_start,
                &replay,
                oldest_sample,
                vad.timeline.end_sample(),
                vad.pre_roll_samples,
                vad.max_overlap_samples,
                last_sent_sample,
                startup_signal_sample,
                is_first_session,
                replay_clamped,
                startup_timeline_retention_ms(&params.vad),
            );
            current = CurrentSessionCapture::new(replay.start_sample);
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
            log_vad_pause_resume_step(
                &params,
                &recording_id,
                recording_started_instant,
                "ui_active",
                [
                    ("session_index", session_index as u64),
                    ("replay_samples", replay.samples.len() as u64),
                    ("replay_start_sample", replay.start_sample),
                    ("timeline_end", vad.timeline.end_sample()),
                ],
            );
            if !replay.samples.is_empty() {
                let Some(active_session) = session.as_mut() else {
                    break 'recording;
                };
                let replay_send_started = Instant::now();
                log_vad_pause_resume_step(
                    &params,
                    &recording_id,
                    recording_started_instant,
                    "replay_send_start",
                    [
                        ("session_index", session_index as u64),
                        ("replay_samples", replay.samples.len() as u64),
                        ("replay_start_sample", replay.start_sample),
                        ("replay_end_sample", replay.end_sample()),
                    ],
                );
                let drain = drain_capture_while(
                    &mut rec,
                    &mut capture_diag,
                    &mut trace,
                    &params,
                    &recording_id,
                    &mut meter,
                    recording_started_instant,
                    mode,
                    Some(&control),
                    send_pcm_chunk(active_session, &replay.samples, &mut total_audio_samples),
                )
                .await;
                log_vad_pause_resume_step(
                    &params,
                    &recording_id,
                    recording_started_instant,
                    "replay_send_done",
                    [
                        ("session_index", session_index as u64),
                        (
                            "send_elapsed_ms",
                            replay_send_started.elapsed().as_millis() as u64,
                        ),
                        ("drained_chunks", drain.drained_pcm.len() as u64),
                        (
                            "drained_samples",
                            drain.drained_pcm.iter().map(|pcm| pcm.len() as u64).sum(),
                        ),
                    ],
                );
                if let Some(interrupted) = drain.interrupted {
                    if let Some(active_session) = session.take() {
                        let _ = active_session.close().await;
                    }
                    match interrupted {
                        CaptureDrainInterrupt::Cancel => cancel_requested = true,
                        CaptureDrainInterrupt::Stop => {
                            refresh_stop_context(&params, &recording_id, &mut app_context);
                        }
                    }
                    break 'recording;
                }
                if drain.capture_eof {
                    stop_requested = true;
                }
                if let Some(error) = drain.capture_error {
                    terminal_error = Some(capture_error(error));
                    break 'recording;
                }
                if let Err(error) = drain.output {
                    terminal_error = Some(error);
                    break 'recording;
                }
                current.record_sent_samples(replay.samples.len() as u64);
                if !drain.drained_pcm.is_empty() {
                    let Some(vad) = vad_pause.as_mut() else {
                        terminal_error = Some(HistoryError {
                            kind: "vad_state".to_string(),
                            msg: "VadPause mode missing VAD state".to_string(),
                        });
                        break 'recording;
                    };
                    let _ = consume_pending_idle_pcm(&drain.drained_pcm, vad, &mut meter);
                    let Some(active_session) = session.as_mut() else {
                        break 'recording;
                    };
                    let live_flush_started = Instant::now();
                    let live_flush_chunks = drain.drained_pcm.len() as u64;
                    let live_flush_samples: u64 =
                        drain.drained_pcm.iter().map(|pcm| pcm.len() as u64).sum();
                    log_vad_pause_resume_step(
                        &params,
                        &recording_id,
                        recording_started_instant,
                        "live_flush_start",
                        [
                            ("session_index", session_index as u64),
                            ("chunks", live_flush_chunks),
                            ("samples", live_flush_samples),
                        ],
                    );
                    for samples in drain.drained_pcm {
                        if let Err(error) =
                            send_pcm_chunk(active_session, &samples, &mut total_audio_samples).await
                        {
                            terminal_error = Some(error);
                            break 'recording;
                        }
                        current.record_sent_samples(samples.len() as u64);
                    }
                    log_vad_pause_resume_step(
                        &params,
                        &recording_id,
                        recording_started_instant,
                        "live_flush_done",
                        [
                            ("session_index", session_index as u64),
                            (
                                "elapsed_ms",
                                live_flush_started.elapsed().as_millis() as u64,
                            ),
                            ("samples", live_flush_samples),
                        ],
                    );
                }
            }
            if let Some(vad) = vad_pause.as_ref() {
                last_sent_sample = vad.timeline.end_sample();
            } else {
                last_sent_sample = replay.end_sample();
            }
            active = true;
        }
    }

    let partial_text_policy = if cancel_requested
        || terminal_error
            .as_ref()
            .is_some_and(|error| error.kind == "asr_timeout")
    {
        PartialTextPolicy::PreserveRecoverableSnapshot
    } else {
        PartialTextPolicy::DiscardTentative
    };
    if current.audio_samples > 0
        || !current.segments.is_empty()
        || current
            .final_text
            .as_deref()
            .is_some_and(|text| !text.is_empty())
        || (partial_text_policy.preserves_partial() && !current.partial_text.is_empty())
    {
        if let Some(capture) = current.into_session(recording_started_instant, partial_text_policy)
        {
            sessions.push(capture);
        }
    }
    if let Some(active_session) = session.take() {
        let _ = active_session.close().await;
    }
    capture_diag.log_summary(&recording_id, mode, params.preprocess.backend);
    // 取消时音频留存跟随「是否有内容」：有内容（可能误触）保留以便用户从 TUI
    // 找回，无内容则丢弃避免孤儿音频文件。正常完成 / terminal error 照常 finalize。
    // resume 录音只有音频没新文本时算无内容——与 finish 的 history 判定同源。
    let has_content =
        crate::voice::capture::has_archivable_content_for(&sessions, params.start.is_seed());
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

#[allow(clippy::too_many_arguments)]
async fn drain_capture_while<F, T>(
    rec: &mut CaptureStream,
    capture_diag: &mut CaptureDiagnostics,
    trace: &mut RecordingObserver,
    params: &SessionParams,
    recording_id: &str,
    meter: &mut MeterCollector,
    recording_started_instant: Instant,
    mode: RecordingMode,
    control: Option<&SessionControl>,
    future: F,
) -> CaptureDrain<T>
where
    F: std::future::Future<Output = T>,
{
    tokio::pin!(future);
    let mut drained_pcm = Vec::new();
    let mut capture_eof = false;
    let mut capture_error = None;
    let mut interrupted = None;

    loop {
        tokio::select! {
            biased;
            interrupt = wait_capture_drain_control(control), if control.is_some() && interrupted.is_none() => {
                interrupted = Some(interrupt);
            }
            output = &mut future => {
                return CaptureDrain {
                    output,
                    drained_pcm,
                    capture_eof,
                    capture_error,
                    interrupted,
                };
            }
            pcm = rec.recv(), if !capture_eof && capture_error.is_none() && interrupted.is_none() => {
                match pcm {
                    Ok(Some(samples)) => {
                        capture_diag.observe(
                            &samples,
                            recording_started_instant,
                            recording_id,
                            mode,
                            params.preprocess.backend,
                        );
                        observe_pcm(trace, &samples);
                        emit_meters(params, recording_id, meter, &samples);
                        drained_pcm.push(samples);
                    }
                    Ok(None) => {
                        capture_eof = true;
                    }
                    Err(error) => {
                        capture_error = Some(error);
                    }
                }
            }
        }
    }
}

async fn wait_capture_drain_control(control: Option<&SessionControl>) -> CaptureDrainInterrupt {
    let control = control.expect("capture drain control must exist when branch is enabled");
    tokio::select! {
        biased;
        _ = control.cancelled() => CaptureDrainInterrupt::Cancel,
        _ = control.stopped() => CaptureDrainInterrupt::Stop,
    }
}

fn consume_pending_idle_pcm(
    chunks: &[Vec<i16>],
    vad: &mut VadPauseState,
    meter: &mut MeterCollector,
) -> Option<u64> {
    use crate::voice::vad::{VadFrame, VadTransition};

    let mut speech_start = None;
    for samples in chunks {
        vad.push_idle_pcm(samples);
        for frame in vad.detector.accept(samples) {
            meter.observe_vad(frame.probability, matches!(frame.frame, VadFrame::Speech));
            let transition = vad.controller.accept(frame.frame);
            if speech_start.is_none() && transition == VadTransition::SpeechStarted {
                speech_start = Some(frame.start_sample);
            }
        }
    }
    speech_start
}

fn handle_asr_event(
    event: AsrEvent,
    current: &mut CurrentSessionCapture,
    params: &SessionParams,
    recording_id: &str,
    recording_started_instant: Instant,
    trace: &mut RecordingObserver,
    transcript: &mut TranscriptDisplay,
) -> Option<HistoryError> {
    observe_asr_event(trace, recording_started_instant, &event);
    match event {
        AsrEvent::Final { text } => {
            current.final_text = Some(text.clone());
            current.partial_text.clear();
            transcript.replace_recent_segments(current.pending_overlay_segments, text.clone());
            overlay_send(
                params,
                OverlayCmd::ReplaceRecentSegments {
                    segments: current.pending_overlay_segments,
                    text,
                },
            );
            current.pending_overlay_segments = 1;
            emit_stats(
                transcript,
                recording_started_instant,
                &params.state,
                recording_id,
                params.overlay.as_ref(),
            );
        }
        AsrEvent::Partial { text, .. } => {
            current.partial_text = text.clone();
            transcript.set_partial(text.clone());
            params.state.partial(recording_id.to_string(), text.clone());
            overlay_send(
                params,
                OverlayCmd::SetText {
                    text,
                    kind: TextKind::Partial,
                },
            );
            emit_stats(
                transcript,
                recording_started_instant,
                &params.state,
                recording_id,
                params.overlay.as_ref(),
            );
        }
        AsrEvent::Segment {
            text,
            started_at,
            ended_at,
        } => {
            current.partial_text.clear();
            params.state.segment(recording_id.to_string(), text.clone());
            overlay_send(params, OverlayCmd::AppendSegment { text: text.clone() });
            transcript.append_segment(text.clone());
            emit_stats(
                transcript,
                recording_started_instant,
                &params.state,
                recording_id,
                params.overlay.as_ref(),
            );
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
    transcript: &mut TranscriptDisplay,
    params: &SessionParams,
    recording_started_instant: Instant,
    capture_diag: &mut CaptureDiagnostics,
    mode: RecordingMode,
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
                        capture_diag.observe(
                            &samples,
                            recording_started_instant,
                            recording_id,
                            mode,
                            params.preprocess.backend,
                        );
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
                            transcript,
                        ) {
                            return Err(error);
                        }
                    }
                }
            }
        }
    }

    let drained = match tokio::time::timeout(
        Duration::from_millis(STOP_RESIDUAL_DRAIN_TIMEOUT_MS),
        rec.drain_after_stop(),
    )
    .await
    {
        Ok(result) => result.map_err(capture_error)?,
        Err(_) => {
            tracing::warn!(
                recording_id,
                timeout_ms = STOP_RESIDUAL_DRAIN_TIMEOUT_MS,
                "stop residual drain timed out; continuing to finalize"
            );
            Vec::new()
        }
    };
    for samples in drained {
        capture_diag.observe(
            &samples,
            recording_started_instant,
            recording_id,
            mode,
            params.preprocess.backend,
        );
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
    startup_signal_sample: Option<u64>,
) -> u64 {
    let pre_roll_start = speech_start_sample.saturating_sub(pre_roll_samples);
    let signal_start = startup_signal_sample
        .map(|sample| sample.saturating_sub(startup_signal_margin_samples()))
        .unwrap_or(pre_roll_start);
    pre_roll_start
        .min(signal_start)
        .max(last_sent_sample.saturating_sub(max_overlap_samples))
        .max(oldest_sample)
}

#[allow(clippy::too_many_arguments)]
fn log_vad_pause_replay_window(
    recording_id: &str,
    backend: crate::config::VoicePreprocessBackend,
    session_index: u32,
    speech_start_sample: u64,
    send_start_sample: u64,
    replay: &crate::voice::timeline::PcmChunk,
    timeline_oldest_sample: u64,
    timeline_end_sample: u64,
    pre_roll_samples: u64,
    max_overlap_samples: u64,
    last_sent_sample: u64,
    startup_signal_sample: Option<u64>,
    is_first_session: bool,
    replay_clamped: bool,
    timeline_retention_ms: u32,
) {
    if is_first_session || replay_clamped {
        tracing::info!(
            recording_id,
            backend = ?backend,
            session_index,
            speech_start_sample,
            speech_start_ms = samples_to_ms(speech_start_sample),
            send_start_sample,
            send_start_ms = samples_to_ms(send_start_sample),
            replay_start_sample = replay.start_sample,
            replay_start_ms = samples_to_ms(replay.start_sample),
            replay_samples = replay.samples.len(),
            replay_ms = samples_to_ms(replay.samples.len() as u64),
            timeline_oldest_sample,
            timeline_oldest_ms = samples_to_ms(timeline_oldest_sample),
            timeline_end_sample,
            timeline_end_ms = samples_to_ms(timeline_end_sample),
            pre_roll_samples,
            pre_roll_ms = samples_to_ms(pre_roll_samples),
            startup_signal_sample,
            startup_signal_ms = startup_signal_sample.map(samples_to_ms),
            startup_signal_margin_samples = startup_signal_margin_samples(),
            startup_signal_margin_ms = samples_to_ms(startup_signal_margin_samples()),
            startup_signal_lookback_ms = STARTUP_SIGNAL_LOOKBACK_MS,
            timeline_retention_ms,
            max_overlap_samples,
            max_overlap_ms = samples_to_ms(max_overlap_samples),
            last_sent_sample,
            last_sent_ms = samples_to_ms(last_sent_sample),
            replay_clamped,
            "VadPause resume replay window"
        );
    } else {
        tracing::debug!(
            recording_id,
            backend = ?backend,
            session_index,
            speech_start_sample,
            speech_start_ms = samples_to_ms(speech_start_sample),
            send_start_sample,
            send_start_ms = samples_to_ms(send_start_sample),
            replay_start_sample = replay.start_sample,
            replay_start_ms = samples_to_ms(replay.start_sample),
            replay_samples = replay.samples.len(),
            replay_ms = samples_to_ms(replay.samples.len() as u64),
            timeline_oldest_sample,
            timeline_oldest_ms = samples_to_ms(timeline_oldest_sample),
            timeline_end_sample,
            timeline_end_ms = samples_to_ms(timeline_end_sample),
            pre_roll_samples,
            pre_roll_ms = samples_to_ms(pre_roll_samples),
            max_overlap_samples,
            max_overlap_ms = samples_to_ms(max_overlap_samples),
            last_sent_sample,
            last_sent_ms = samples_to_ms(last_sent_sample),
            replay_clamped,
            "VadPause resume replay window"
        );
    }
}

fn startup_signal_margin_samples() -> u64 {
    (STARTUP_SIGNAL_MARGIN_MS as u64) * crate::voice::timeline::SAMPLE_RATE / 1000
}

fn startup_timeline_retention_ms(config: &crate::config::VoiceVadCfg) -> u32 {
    let regular_resume_ms = config.pre_roll_ms + config.max_overlap_ms + 100;
    let startup_evidence_ms = STARTUP_SIGNAL_LOOKBACK_MS + STARTUP_SIGNAL_MARGIN_MS + 100;
    regular_resume_ms.max(startup_evidence_ms)
}

fn frame_has_signal(samples: &[i16]) -> bool {
    pcm_peak_abs(samples) > MIN_NONZERO_AMPLITUDE as u16
}

fn first_signal_offset(samples: &[i16]) -> Option<usize> {
    samples
        .iter()
        .position(|sample| sample.unsigned_abs() > MIN_NONZERO_AMPLITUDE as u16)
}

fn pcm_peak_abs(samples: &[i16]) -> u16 {
    samples
        .iter()
        .map(|sample| sample.unsigned_abs())
        .max()
        .unwrap_or(0)
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

fn asr_open_error_message(error: &crate::asr::types::AsrError) -> String {
    match error {
        crate::asr::types::AsrError::OpenTimeout => crate::t!("error.asr_open_timeout"),
        _ => crate::t!("error.asr_open"),
    }
}

fn capture_error(error: anyhow::Error) -> HistoryError {
    HistoryError {
        kind: "capture".to_string(),
        msg: format!("{error:#}"),
    }
}

fn log_vad_pause_resume_step<const N: usize>(
    params: &SessionParams,
    recording_id: &str,
    recording_started_instant: Instant,
    step: &'static str,
    fields: [(&'static str, u64); N],
) {
    if !params.vad_trace {
        return;
    }
    let details = fields
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join(" ");
    tracing::info!(
        recording_id,
        step,
        t_ms = instant_elapsed_ms(recording_started_instant),
        details,
        "VadPause resume diagnostic"
    );
}

async fn send_pcm_chunk(
    session: &mut Box<dyn AsrSession>,
    samples: &[i16],
    audio_samples_sent: &mut u64,
) -> Result<(), HistoryError> {
    let started = Instant::now();
    match tokio::time::timeout(ASR_SEND_TIMEOUT, session.send_pcm(samples, false)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            return Err(HistoryError {
                kind: "asr_send".to_string(),
                msg: error.to_string(),
            })
        }
        Err(_) => {
            return Err(HistoryError {
                kind: "asr_send".to_string(),
                msg: "timeout sending PCM to ASR provider".to_string(),
            })
        }
    }
    let elapsed_ms = started.elapsed().as_millis();
    if elapsed_ms >= ASR_SEND_SLOW_WARN_MS {
        tracing::warn!(elapsed_ms, samples = samples.len(), "ASR send_pcm was slow");
    }
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
    use crate::voice::timeline::ms_to_samples;

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

    struct PendingSendSession;

    #[async_trait]
    impl AsrSession for PendingSendSession {
        async fn send_pcm(&mut self, _pcm: &[i16], _is_last: bool) -> Result<(), AsrError> {
            std::future::pending().await
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

    struct AutoDoneCollectingSession {
        sent: Arc<Mutex<Vec<Vec<i16>>>>,
        events: mpsc::Sender<AsrEvent>,
    }

    #[async_trait]
    impl AsrSession for AutoDoneCollectingSession {
        async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<(), AsrError> {
            self.sent.lock().unwrap().push(pcm.to_vec());
            if is_last {
                let _ = self.events.send(AsrEvent::Done).await;
            }
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
            apple_backend_trace: false,
            idle_pause: false,
            open_timeout_ms: 100,
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
                asr_instance: "fake".to_string(),
                chain_summary: "default".to_string(),
            }],
            post_chain: post::PostChain {
                name: "default".to_string(),
                processors: Vec::new(),
            },
            post_timeout_ms: 100,
            start: crate::voice::resume::RecordingStart::Fresh,
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
        let (pcm_tx, pcm_rx) = mpsc::unbounded_channel();
        drop(pcm_tx);
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

        let _started = match join_capture_and_asr(capture, asr, "test", &control).await {
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
    async fn startup_buffers_capture_pcm_until_asr_open() {
        let (pcm_tx, pcm_rx) = mpsc::unbounded_channel();
        let (_event_tx, events) = mpsc::channel(1);
        let capture = async move { Ok(CaptureStream::Cpal(RecordingStream::for_test(pcm_rx))) };
        let asr = async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok::<OpenedSession, AsrError>((Box::new(CollectingSession::default()), events))
        };
        let control = SessionControl::new();
        let started = join_capture_and_asr(capture, asr, "test", &control);
        tokio::pin!(started);

        tokio::task::yield_now().await;
        pcm_tx.send(vec![1, 2, 3]).unwrap();
        pcm_tx.send(vec![4, 5]).unwrap();

        let started = tokio::time::timeout(Duration::from_millis(100), &mut started)
            .await
            .expect("ASR open should complete")
            .expect("startup should succeed");

        assert_eq!(started.startup_pcm, vec![vec![1, 2, 3], vec![4, 5]]);
    }

    #[tokio::test]
    async fn startup_stop_after_capture_preserves_buffer_until_asr_open() {
        let (pcm_tx, pcm_rx) = mpsc::unbounded_channel();
        let (_event_tx, events) = mpsc::channel(1);
        let capture = async move { Ok(CaptureStream::Cpal(RecordingStream::for_test(pcm_rx))) };
        let asr = async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok::<OpenedSession, AsrError>((Box::new(CollectingSession::default()), events))
        };
        let control = SessionControl::new();
        let stop = control.clone();
        let started = join_capture_and_asr(capture, asr, "test", &control);
        tokio::pin!(started);

        tokio::task::yield_now().await;
        pcm_tx.send(vec![1, 2, 3]).unwrap();
        stop.request_stop();
        pcm_tx.send(vec![4, 5]).unwrap();
        drop(pcm_tx);

        let started = tokio::time::timeout(Duration::from_millis(100), &mut started)
            .await
            .expect("ASR open should complete after stop")
            .expect("startup should preserve audio until ASR opens");

        assert_eq!(started.startup_pcm, vec![vec![1, 2, 3], vec![4, 5]]);
    }

    #[tokio::test]
    async fn initial_asr_open_failure_discards_started_capture() {
        let (pcm_tx, pcm_rx) = mpsc::unbounded_channel();
        drop(pcm_tx);
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
    async fn initial_asr_open_times_out() {
        let (_pcm_tx, pcm_rx) = mpsc::unbounded_channel();
        let capture = async move { Ok(CaptureStream::Cpal(RecordingStream::for_test(pcm_rx))) };
        let asr = open_with_timeout(
            async { std::future::pending::<Result<OpenedSession, AsrError>>().await },
            10,
        );
        let control = SessionControl::new();

        let error = match join_capture_and_asr(capture, asr, "test", &control).await {
            Ok(_) => panic!("expected ASR open timeout"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            StartSessionError::Asr(AsrError::OpenTimeout)
        ));
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
    async fn startup_pcm_is_replayed_and_counted_before_live_pcm() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let (event_tx, events) = mpsc::channel(4);
        let (pcm_tx, pcm_rx) = mpsc::unbounded_channel();
        drop(pcm_tx);
        let rec = CaptureStream::Cpal(RecordingStream::for_test(pcm_rx));
        let state = StateStore::new();
        let params = test_session_params(state, None);
        let provider = CountingProvider {
            opens: AtomicUsize::new(0),
        };
        let outcome = run_with_capture_stream(
            &provider,
            params,
            SessionControl::new(),
            rec,
            Some((
                Box::new(AutoDoneCollectingSession {
                    sent: sent.clone(),
                    events: event_tx,
                }),
                events,
            )),
            vec![vec![10, 11, 12], vec![13, 14]],
            post::AppContext::default(),
            "test".to_string(),
            time::OffsetDateTime::now_utc(),
            Instant::now(),
            RecordingMode::Continuous,
            RecordingObserver::start(TraceStart {
                enabled: false,
                recording_id: "test".to_string(),
                provider: "counting".to_string(),
                started_at: time::OffsetDateTime::now_utc().to_string(),
                started_instant: Instant::now(),
            }),
            None,
        )
        .await
        .expect("engine should finish after input EOF");

        assert_eq!(
            sent.lock().unwrap().clone(),
            vec![vec![10, 11, 12], vec![13, 14], Vec::<i16>::new()]
        );
        assert_eq!(outcome.total_audio_samples, 5);
        assert_eq!(outcome.sessions.len(), 1);
        assert_eq!(outcome.sessions[0].audio_samples, 5);
    }

    #[cfg(target_os = "macos")]
    async fn recv_apple_capture_with_timeout(
        capture: &mut AppleCapture,
    ) -> anyhow::Result<Option<Vec<i16>>> {
        tokio::time::timeout(Duration::from_secs(1), capture.recv())
            .await
            .expect("AppleCapture recv timed out")
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn apple_capture_survives_dropped_recv_future() {
        let (apple_tx, apple_rx) = mpsc::channel(8);
        let mut capture = AppleCapture::apple(
            "test",
            crate::voice::apple_source::RunningAppleVpSource::for_test(apple_rx),
        );

        let mut pending_recv = Box::pin(capture.recv());
        tokio::select! {
            result = &mut pending_recv => panic!("AppleOnly recv returned unexpectedly: {result:?}"),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {}
        }
        drop(pending_recv);

        apple_tx.send(Ok(vec![42])).await.unwrap();
        assert_eq!(
            recv_apple_capture_with_timeout(&mut capture).await.unwrap(),
            Some(vec![42])
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn apple_capture_raw_fallback_survives_dropped_recv_future() {
        let (raw_tx, raw_rx) = mpsc::unbounded_channel();
        let raw = RecordingStream::for_test(raw_rx);
        let mut capture = AppleCapture::raw_fallback("test", raw);

        let mut pending_recv = Box::pin(capture.recv());
        tokio::select! {
            result = &mut pending_recv => panic!("RawFallback recv returned unexpectedly: {result:?}"),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {}
        }
        drop(pending_recv);

        raw_tx.send(vec![7]).unwrap();
        assert_eq!(
            recv_apple_capture_with_timeout(&mut capture).await.unwrap(),
            Some(vec![7])
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn apple_capture_propagates_apple_error() {
        let (apple_tx, apple_rx) = mpsc::channel(8);
        let mut capture = AppleCapture::apple(
            "test",
            crate::voice::apple_source::RunningAppleVpSource::for_test(apple_rx),
        );

        apple_tx.send(Ok(vec![10])).await.unwrap();
        apple_tx
            .send(Err(anyhow::anyhow!("scripted apple terminal")))
            .await
            .unwrap();

        assert_eq!(
            recv_apple_capture_with_timeout(&mut capture).await.unwrap(),
            Some(vec![10])
        );
        let error = recv_apple_capture_with_timeout(&mut capture)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("scripted apple terminal"),
            "{error:#}"
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn apple_capture_drain_after_stop_outputs_apple_residual() {
        let (apple_tx, apple_rx) = mpsc::channel(8);
        apple_tx.send(Ok(vec![11])).await.unwrap();
        drop(apple_tx);
        let mut capture = AppleCapture::apple(
            "test",
            crate::voice::apple_source::RunningAppleVpSource::for_test(apple_rx),
        );

        let drained = tokio::time::timeout(Duration::from_secs(1), capture.drain_after_stop())
            .await
            .expect("Apple stop drain timed out")
            .unwrap();

        assert_eq!(drained, vec![vec![11]]);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn apple_capture_finish_does_not_publish_retained_audio() {
        let (_apple_tx, apple_rx) = mpsc::channel(4);
        let mut capture = AppleCapture::apple(
            "test",
            crate::voice::apple_source::RunningAppleVpSource::for_test(apple_rx),
        );

        assert_eq!(capture.finish_audio().await.unwrap(), None);
        assert!(matches!(capture.state, AppleCaptureState::Done));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn apple_capture_stop_after_apple_uses_explicit_stopping_state() {
        let (_apple_tx, apple_rx) = mpsc::channel(4);
        let mut capture = AppleCapture::apple(
            "test",
            crate::voice::apple_source::RunningAppleVpSource::for_test(apple_rx),
        );

        capture.stop();

        assert!(matches!(
            capture.state,
            AppleCaptureState::StoppingApple { .. }
        ));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn apple_capture_raw_fallback_stop_drains_raw() {
        let (raw_tx, raw_rx) = mpsc::unbounded_channel();
        let (raw, _finish_rx) = RecordingStream::for_test_observe(raw_rx);
        let mut capture = AppleCapture::raw_fallback("test", raw);

        raw_tx.send(vec![1, 2, 3]).unwrap();
        capture.stop();
        raw_tx.send(vec![4, 5]).unwrap();
        drop(raw_tx);

        let drained = tokio::time::timeout(Duration::from_millis(50), capture.drain_after_stop())
            .await
            .expect("drain should not wait")
            .unwrap();

        assert_eq!(drained, vec![vec![1, 2, 3], vec![4, 5]]);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "touches microphone/TCC; run manually to verify pure Apple VP capture"]
    async fn apple_capture_smoke_receives_apple_pcm() {
        let mut rec = start_apple_capture_stream("test", false)
            .await
            .expect("Apple capture should start");
        let deadline = TokioInstant::now() + Duration::from_secs(6);
        let mut frames = 0usize;

        loop {
            assert!(
                TokioInstant::now() < deadline,
                "Apple bridge did not switch to Apple PCM after {frames} frames"
            );
            let pcm = tokio::time::timeout(Duration::from_secs(1), rec.recv())
                .await
                .expect("Apple capture recv timed out")
                .expect("Apple capture recv failed");
            if pcm.is_some() {
                frames += 1;
                break;
            }
        }

        rec.stop();
        let _ = rec.drain_after_stop().await.unwrap();
        assert!(frames > 0, "expected at least one PCM frame");
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
        let session = capture
            .into_session(started_at, PartialTextPolicy::DiscardTentative)
            .unwrap();
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
            .into_session(Instant::now(), PartialTextPolicy::DiscardTentative)
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

    #[tokio::test]
    async fn pending_pcm_send_times_out_without_counting_audio() {
        let mut session: Box<dyn AsrSession> = Box::new(PendingSendSession);
        let mut audio_samples_sent = 7;
        let error = send_pcm_chunk(&mut session, &[1, 2, 3], &mut audio_samples_sent)
            .await
            .expect_err("pending PCM delivery must time out");
        assert_eq!(error.kind, "asr_send");
        assert!(error.msg.contains("timeout"), "{}", error.msg);
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
            compute_resume_start_sample(16_000, 4_800, 0, 3_200, 0, None),
            11_200
        );
    }

    #[test]
    fn resume_start_bounded_by_last_sent_minus_max_overlap() {
        assert_eq!(
            compute_resume_start_sample(18_000, 8_000, 20_000, 3_200, 0, None),
            16_800
        );
    }

    #[test]
    fn resume_start_clamped_to_oldest_retained_sample() {
        assert_eq!(
            compute_resume_start_sample(2_000, 4_000, 0, 200, 800, None),
            800
        );
    }

    #[test]
    fn resume_overlap_stays_within_cap() {
        let max_overlap = 200 * 16_000 / 1_000;
        let start = compute_resume_start_sample(10_000, 0, 16_000, max_overlap, 0, None);
        assert!(16_000 - start <= max_overlap);
    }

    #[test]
    fn first_resume_uses_startup_signal_evidence_before_late_vad_trigger() {
        let configured_pre_roll = 300 * 16_000 / 1_000;
        let first_signal = 1_600; // 100ms after recording start.

        let send_start = compute_resume_start_sample(
            13_824,
            configured_pre_roll,
            0,
            200 * 16_000 / 1_000,
            0,
            Some(first_signal),
        );
        assert_eq!(send_start, 0);
    }

    #[test]
    fn startup_signal_evidence_still_respects_overlap_and_oldest_bounds() {
        let configured_pre_roll = 300 * 16_000 / 1_000;
        let first_signal = 1_600;

        assert_eq!(
            compute_resume_start_sample(
                13_824,
                configured_pre_roll,
                20_000,
                200 * 16_000 / 1_000,
                0,
                Some(first_signal),
            ),
            16_800
        );
        assert_eq!(
            compute_resume_start_sample(
                13_824,
                configured_pre_roll,
                0,
                200 * 16_000 / 1_000,
                4_000,
                Some(first_signal),
            ),
            4_000
        );
    }

    #[test]
    fn startup_signal_evidence_retention_survives_late_vad_trigger() {
        let config = crate::config::VoiceVadCfg::default();
        let mut vad = VadPauseState::with_detector(
            &config,
            VadDetector::Scripted {
                frames: std::collections::VecDeque::new(),
                buffered_samples: 0,
                sample_offset: 0,
            },
        )
        .unwrap();

        vad.push_idle_pcm(&vec![0; ms_to_samples(100) as usize]);
        let mut first_voice = vec![0; ms_to_samples(100) as usize];
        first_voice[0] = MIN_NONZERO_AMPLITUDE + 1;
        vad.push_idle_pcm(&first_voice);
        vad.push_idle_pcm(&vec![0; ms_to_samples(664) as usize]);

        let first_signal = vad.first_signal_sample.unwrap();
        let speech_start = ms_to_samples(864);
        let send_start = compute_resume_start_sample(
            speech_start,
            vad.pre_roll_samples,
            0,
            vad.max_overlap_samples,
            vad.timeline.oldest_sample(),
            Some(first_signal),
        );
        let replay = vad.timeline.slice_from(send_start);

        assert_eq!(samples_to_ms(first_signal), 100);
        assert_eq!(send_start, 0);
        assert_eq!(replay.start_sample, 0);
        assert!(!replay.samples.is_empty());
    }

    #[test]
    fn startup_signal_evidence_records_first_non_silent_sample_once() {
        let config = crate::config::VoiceVadCfg::default();
        let mut vad = VadPauseState::with_detector(
            &config,
            VadDetector::Scripted {
                frames: std::collections::VecDeque::new(),
                buffered_samples: 0,
                sample_offset: 0,
            },
        )
        .unwrap();

        vad.push_idle_pcm(&[0, 1, MIN_NONZERO_AMPLITUDE]);
        assert_eq!(vad.first_signal_sample, None);

        vad.push_idle_pcm(&[0, MIN_NONZERO_AMPLITUDE + 1, 40]);
        assert_eq!(vad.first_signal_sample, Some(4));

        vad.push_idle_pcm(&[100, 100]);
        assert_eq!(vad.first_signal_sample, Some(4));
    }

    #[test]
    fn audio_signal_threshold_rejects_zero_and_accepts_noise_floor() {
        assert!(!frame_has_signal(&[0; 480]));
        assert!(!frame_has_signal(&[1, -2, 8, -8]));
        assert!(frame_has_signal(&[0, MIN_NONZERO_AMPLITUDE + 1, 0]));
        assert!(frame_has_signal(&[80, -50, 60]));
    }
}
