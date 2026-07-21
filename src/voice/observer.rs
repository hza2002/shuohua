use std::time::Instant;

use crate::asr::types::{AsrError, AsrEvent};
use crate::voice::capture;

#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "dev"), allow(dead_code))]
pub struct TraceStart {
    pub enabled: bool,
    pub recording_id: String,
    pub provider: String,
    pub started_at: String,
    pub started_instant: Instant,
}

#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "dev"), allow(dead_code))]
pub enum SessionPhase {
    Start {
        index: u32,
        start_ms: u64,
    },
    FinalizeStart {
        index: u32,
        t_ms: u64,
    },
    Done {
        index: u32,
        start_ms: u64,
        end_ms: u64,
        audio_ms: u64,
    },
    OpenError {
        index: u32,
        t_ms: u64,
        message: String,
    },
}

#[cfg(feature = "dev")]
mod imp {
    use super::{AsrEvent, SessionPhase, TraceStart};
    use crate::voice::silero::{SileroConfig, SileroVad};
    use crate::voice::vad::{VadFrame, VadPolicy};
    use anyhow::{Context, Result};
    use serde_json::json;
    use std::fs::{File, OpenOptions};
    use std::io::{BufWriter, Write};
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    const SAMPLE_RATE: u64 = 16_000;
    const LEVEL_WINDOW_SAMPLES: u64 = SAMPLE_RATE / 20;
    const SIGNAL_AMPLITUDE_THRESHOLD: u16 = 8;
    const VAD_THRESHOLD: f32 = 0.5;
    const PRE_ROLL_MS: u64 = 300;

    pub struct RecordingObserver {
        inner: Option<TraceInner>,
    }

    struct TraceInner {
        writer: BufWriter<File>,
        vad: SileroTrace,
        level: AudioLevelTrace,
        started_instant: Instant,
    }

    struct AudioLevelTrace {
        window_sum_squares: f64,
        window_samples: u64,
        windows: u64,
        max_rms: f32,
        max_peak: f32,
        window_peak_abs: u16,
        peak_abs: u16,
        clipped: bool,
    }

    struct SileroTrace {
        detector: SileroVad,
        state: TraceVadState,
        voiced_frames: u32,
        first_voiced_start_ms: Option<u64>,
        silence_ms: u64,
        active_start_ms: Option<u64>,
        active_ms: u64,
        sessions: u32,
        policy: VadPolicy,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TraceVadState {
        Silence,
        Speech,
    }

    impl RecordingObserver {
        pub fn start(start: TraceStart) -> Self {
            match Self::start_in_dir(&default_trace_dir(), start) {
                Ok(trace) => trace,
                Err(e) => {
                    tracing::warn!(error = ?e, "dev voice trace disabled");
                    Self { inner: None }
                }
            }
        }

        pub fn start_in_dir(base: &Path, start: TraceStart) -> Result<Self> {
            if !start.enabled {
                return Ok(Self { inner: None });
            }
            std::fs::create_dir_all(base)
                .with_context(|| format!("create trace dir {}", base.display()))?;
            let path = base.join(format!("{}.jsonl", start.recording_id));
            let file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&path)
                .with_context(|| format!("open trace {}", path.display()))?;
            let mut inner = TraceInner {
                writer: BufWriter::new(file),
                vad: SileroTrace::new()?,
                level: AudioLevelTrace::default(),
                started_instant: start.started_instant,
            };
            inner.write(json!({
                "event": "recording_start",
                "recording_id": start.recording_id,
                "provider": start.provider,
                "started_at": start.started_at,
                "vad": {
                    "backend": "silero",
                    "threshold": VAD_THRESHOLD,
                    "frame_ms": inner.vad.policy.frame_ms,
                    "min_start_voiced_frames": inner.vad.policy.min_start_voiced_frames,
                    "pause_silence_ms": inner.vad.policy.pause_silence_ms,
                    "pre_roll_ms": PRE_ROLL_MS,
                }
            }));
            Ok(Self { inner: Some(inner) })
        }

        pub fn on_pcm(&mut self, samples: &[i16]) {
            let Some(inner) = self.inner.as_mut() else {
                return;
            };
            inner.level.accept(samples);
            let events = inner.vad.accept(samples);
            for event in events {
                inner.write(event);
            }
        }

        pub fn on_provider_opened(&mut self, t_ms: u64) {
            self.write(json!({"event": "provider_opened", "t_ms": t_ms}));
        }

        pub fn on_asr_event(&mut self, t_ms: u64, event: &AsrEvent) {
            let started_instant = self
                .inner
                .as_ref()
                .map(|inner| inner.started_instant)
                .unwrap_or_else(Instant::now);
            match event {
                AsrEvent::Partial { text, seq } => {
                    self.write(json!({
                        "event": "asr_partial",
                        "t_ms": t_ms,
                        "seq": seq,
                        "text": text,
                    }));
                }
                AsrEvent::Segment {
                    text,
                    started_at,
                    ended_at,
                } => {
                    self.write(json!({
                        "event": "asr_segment",
                        "t_ms": t_ms,
                        "text": text,
                        "start_ms": instant_to_ms(started_instant, *started_at),
                        "end_ms": instant_to_ms(started_instant, *ended_at),
                    }));
                }
                AsrEvent::Final { text } => {
                    self.write(json!({
                        "event": "asr_final",
                        "t_ms": t_ms,
                        "text": text,
                    }));
                }
                AsrEvent::Error { err } => {
                    self.write(json!({
                        "event": "asr_error",
                        "t_ms": t_ms,
                        "message": err.to_string(),
                    }));
                }
                AsrEvent::Done => {
                    self.write(json!({"event": "asr_done", "t_ms": t_ms}));
                }
            }
        }

        pub fn on_session(&mut self, phase: SessionPhase) {
            match phase {
                SessionPhase::Start { index, start_ms } => {
                    self.write(json!({
                        "event": "session_start",
                        "session_index": index,
                        "start_ms": start_ms,
                    }));
                }
                SessionPhase::FinalizeStart { index, t_ms } => {
                    self.write(json!({
                        "event": "session_finalize_start",
                        "session_index": index,
                        "t_ms": t_ms,
                    }));
                }
                SessionPhase::Done {
                    index,
                    start_ms,
                    end_ms,
                    audio_ms,
                } => {
                    self.write(json!({
                        "event": "session_done",
                        "session_index": index,
                        "start_ms": start_ms,
                        "end_ms": end_ms,
                        "audio_ms": audio_ms,
                    }));
                }
                SessionPhase::OpenError {
                    index,
                    t_ms,
                    message,
                } => {
                    self.write(json!({
                        "event": "session_open_error",
                        "session_index": index,
                        "t_ms": t_ms,
                        "message": message,
                    }));
                }
            }
        }

        pub fn on_finish(&mut self, status: &str, audio_ms: u64) {
            let Some(inner) = self.inner.as_mut() else {
                return;
            };
            let summary = inner.vad.finish(audio_ms);
            let level = inner.level.finish();
            inner.write(json!({
                "event": "recording_end",
                "status": status,
                "audio_ms": audio_ms,
                "vad": {
                    "active_ms": summary.active_ms,
                    "saved_ms": audio_ms.saturating_sub(summary.active_ms),
                    "sessions": summary.sessions,
                },
                "audio_level": {
                    "windows": level.windows,
                    "max_rms": level.max_rms,
                    "max_peak": level.max_peak,
                    "clipped": level.clipped,
                    "has_signal": level.has_signal,
                }
            }));
            let _ = inner.writer.flush();
        }

        fn write(&mut self, value: serde_json::Value) {
            if let Some(inner) = self.inner.as_mut() {
                inner.write(value);
            }
        }
    }

    struct VadSummary {
        active_ms: u64,
        sessions: u32,
    }

    struct AudioLevelSummary {
        windows: u64,
        max_rms: f32,
        max_peak: f32,
        clipped: bool,
        has_signal: bool,
    }

    impl AudioLevelTrace {
        fn accept(&mut self, samples: &[i16]) {
            for &sample in samples {
                let abs = sample.unsigned_abs();
                self.peak_abs = self.peak_abs.max(abs);
                self.window_peak_abs = self.window_peak_abs.max(abs);
                self.clipped |= abs >= i16::MAX as u16;
                let normalized = f64::from(sample) / f64::from(i16::MAX);
                self.window_sum_squares += normalized * normalized;
                self.window_samples += 1;
                if self.window_samples >= LEVEL_WINDOW_SAMPLES {
                    self.finish_window();
                }
            }
        }

        fn finish(&mut self) -> AudioLevelSummary {
            if self.window_samples > 0 {
                self.finish_window();
            }
            AudioLevelSummary {
                windows: self.windows,
                max_rms: self.max_rms,
                max_peak: self.max_peak,
                clipped: self.clipped,
                has_signal: self.peak_abs > SIGNAL_AMPLITUDE_THRESHOLD,
            }
        }

        fn finish_window(&mut self) {
            let rms = if self.window_samples == 0 {
                0.0
            } else {
                (self.window_sum_squares / self.window_samples as f64).sqrt() as f32
            };
            self.max_rms = self.max_rms.max(rms.clamp(0.0, 1.0));
            self.max_peak = self
                .max_peak
                .max((self.window_peak_abs as f32 / i16::MAX as f32).clamp(0.0, 1.0));
            self.windows += 1;
            self.window_sum_squares = 0.0;
            self.window_samples = 0;
            self.window_peak_abs = 0;
        }
    }

    impl Default for AudioLevelTrace {
        fn default() -> Self {
            Self {
                window_sum_squares: 0.0,
                window_samples: 0,
                windows: 0,
                max_rms: 0.0,
                max_peak: 0.0,
                window_peak_abs: 0,
                peak_abs: 0,
                clipped: false,
            }
        }
    }

    impl SileroTrace {
        fn new() -> Result<Self> {
            let policy = VadPolicy::default();
            let detector = SileroVad::new(SileroConfig {
                threshold: VAD_THRESHOLD,
            })
            .map_err(|e| anyhow::anyhow!("create Silero VAD: {e}"))?;
            Ok(Self {
                detector,
                state: TraceVadState::Silence,
                voiced_frames: 0,
                first_voiced_start_ms: None,
                silence_ms: 0,
                active_start_ms: None,
                active_ms: 0,
                sessions: 0,
                policy,
            })
        }

        fn accept(&mut self, samples: &[i16]) -> Vec<serde_json::Value> {
            let mut out = Vec::new();
            for frame in self.detector.accept(samples) {
                let start_ms = samples_to_ms(frame.start_sample);
                let end_ms =
                    samples_to_ms(frame.start_sample + SileroConfig::frame_samples() as u64);
                let speech = matches!(frame.frame, VadFrame::Speech);
                out.push(json!({
                    "event": "vad_frame",
                    "start_ms": start_ms,
                    "end_ms": end_ms,
                    "probability": frame.probability,
                    "speech": speech,
                }));
                if let Some(event) = self.accept_vad_frame(frame.frame, start_ms, end_ms) {
                    out.push(event);
                }
            }
            out
        }

        fn accept_vad_frame(
            &mut self,
            frame: VadFrame,
            start_ms: u64,
            end_ms: u64,
        ) -> Option<serde_json::Value> {
            match (self.state, frame) {
                (TraceVadState::Silence, VadFrame::Speech) => {
                    if self.voiced_frames == 0 {
                        self.first_voiced_start_ms = Some(start_ms);
                    }
                    self.voiced_frames += 1;
                    self.silence_ms = 0;
                    if self.voiced_frames >= self.policy.min_start_voiced_frames {
                        let active_start = self
                            .first_voiced_start_ms
                            .unwrap_or(start_ms)
                            .saturating_sub(PRE_ROLL_MS);
                        self.state = TraceVadState::Speech;
                        self.voiced_frames = 0;
                        self.first_voiced_start_ms = None;
                        self.active_start_ms = Some(active_start);
                        self.sessions += 1;
                        Some(json!({
                            "event": "vad_transition",
                            "kind": "resume",
                            "at_ms": active_start,
                            "detected_at_ms": end_ms,
                        }))
                    } else {
                        None
                    }
                }
                (TraceVadState::Silence, VadFrame::Silence) => {
                    self.voiced_frames = 0;
                    self.first_voiced_start_ms = None;
                    None
                }
                (TraceVadState::Speech, VadFrame::Speech) => {
                    self.silence_ms = 0;
                    None
                }
                (TraceVadState::Speech, VadFrame::Silence) => {
                    self.silence_ms += self.policy.frame_ms as u64;
                    if self.silence_ms >= self.policy.pause_silence_ms as u64 {
                        self.state = TraceVadState::Silence;
                        self.silence_ms = 0;
                        if let Some(start) = self.active_start_ms.take() {
                            self.active_ms += end_ms.saturating_sub(start);
                        }
                        Some(json!({
                            "event": "vad_transition",
                            "kind": "pause",
                            "at_ms": end_ms,
                        }))
                    } else {
                        None
                    }
                }
            }
        }

        fn finish(&mut self, audio_ms: u64) -> VadSummary {
            if let Some(start) = self.active_start_ms.take() {
                self.active_ms += audio_ms.saturating_sub(start);
            }
            VadSummary {
                active_ms: self.active_ms.min(audio_ms),
                sessions: self.sessions,
            }
        }
    }

    impl TraceInner {
        fn write(&mut self, value: serde_json::Value) {
            if serde_json::to_writer(&mut self.writer, &value).is_ok() {
                let _ = self.writer.write_all(b"\n");
            }
        }
    }

    fn samples_to_ms(samples: u64) -> u64 {
        samples.saturating_mul(1000) / SAMPLE_RATE
    }

    fn instant_to_ms(base: Instant, instant: Instant) -> u64 {
        instant
            .saturating_duration_since(base)
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    fn default_trace_dir() -> PathBuf {
        crate::paths::StateDirs::discover().traces()
    }
}

#[cfg(not(feature = "dev"))]
mod imp {
    use super::{AsrEvent, SessionPhase, TraceStart};
    use std::path::Path;

    #[derive(Debug, Clone)]
    pub struct RecordingObserver;

    impl RecordingObserver {
        #[inline(always)]
        pub fn start(_start: TraceStart) -> Self {
            Self
        }

        #[allow(dead_code)]
        #[inline(always)]
        pub fn start_in_dir(_base: &Path, _start: TraceStart) -> anyhow::Result<Self> {
            Ok(Self)
        }

        #[inline(always)]
        pub fn on_pcm(&mut self, _samples: &[i16]) {}

        #[inline(always)]
        pub fn on_provider_opened(&mut self, _t_ms: u64) {}

        #[inline(always)]
        pub fn on_asr_event(&mut self, _t_ms: u64, _event: &AsrEvent) {}

        #[inline(always)]
        pub fn on_session(&mut self, _phase: SessionPhase) {}

        #[inline(always)]
        pub fn on_finish(&mut self, _status: &str, _audio_ms: u64) {}
    }
}

pub use imp::RecordingObserver;

#[inline]
pub(crate) fn instant_elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

#[inline]
pub(crate) fn observe_asr_event(trace: &mut RecordingObserver, started: Instant, event: &AsrEvent) {
    trace.on_asr_event(instant_elapsed_ms(started), event);
}

#[inline]
pub(crate) fn observe_asr_event_at(trace: &mut RecordingObserver, t_ms: u64, event: &AsrEvent) {
    trace.on_asr_event(t_ms, event);
}

#[inline]
pub(crate) fn observe_asr_error(trace: &mut RecordingObserver, started: Instant, err: AsrError) {
    observe_asr_event(trace, started, &AsrEvent::Error { err });
}

#[inline]
pub(crate) fn observe_pcm(trace: &mut RecordingObserver, samples: &[i16]) {
    trace.on_pcm(samples);
}

#[inline]
pub(crate) fn observe_provider_opened(trace: &mut RecordingObserver, started: Instant) {
    trace.on_provider_opened(instant_elapsed_ms(started));
}

#[inline]
pub(crate) fn observe_session(trace: &mut RecordingObserver, phase: SessionPhase) {
    trace.on_session(phase);
}

#[inline]
pub(crate) fn observe_finish(trace: &mut RecordingObserver, status: &str, audio_samples: u64) {
    trace.on_finish(status, capture::samples_to_ms(audio_samples));
}

#[inline]
pub(crate) fn observe_finish_ms(trace: &mut RecordingObserver, status: &str, audio_ms: u64) {
    trace.on_finish(status, audio_ms);
}

#[cfg(all(test, feature = "dev"))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn writes_jsonl_trace_events_and_summary() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-trace-test-{}", ulid::Ulid::generate()));
        fs::create_dir_all(&dir).unwrap();

        let mut trace = RecordingObserver::start_in_dir(
            &dir,
            TraceStart {
                enabled: true,
                recording_id: "01TRACE".to_string(),
                provider: "doubao".to_string(),
                started_at: "2026-06-16T00:00:00Z".to_string(),
                started_instant: Instant::now(),
            },
        )
        .unwrap();
        let started_instant = Instant::now();
        trace.on_asr_event(
            10,
            &AsrEvent::Partial {
                text: "测".to_string(),
                seq: 1,
            },
        );
        trace.on_asr_event(
            20,
            &AsrEvent::Segment {
                text: "测试".to_string(),
                started_at: started_instant,
                ended_at: started_instant + std::time::Duration::from_millis(500),
            },
        );
        trace.on_finish("submitted", 1600);

        let body = fs::read_to_string(dir.join("01TRACE.jsonl")).unwrap();
        assert!(body.contains(r#""event":"recording_start""#));
        assert!(body.contains(r#""event":"asr_partial""#));
        assert!(body.contains(r#""event":"asr_segment""#));
        assert!(body.contains(r#""event":"recording_end""#));
        assert!(body.contains(r#""audio_ms":1600"#));
        assert!(body.contains(r#""audio_level""#));
        assert!(body.contains(r#""has_signal":false"#));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn summarizes_capture_audio_levels_in_trace_end() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-trace-level-{}", ulid::Ulid::generate()));
        fs::create_dir_all(&dir).unwrap();

        let mut trace = RecordingObserver::start_in_dir(
            &dir,
            TraceStart {
                enabled: true,
                recording_id: "01LEVEL".to_string(),
                provider: "apple".to_string(),
                started_at: "2026-06-16T00:00:00Z".to_string(),
                started_instant: Instant::now(),
            },
        )
        .unwrap();
        trace.on_pcm(&vec![i16::MAX / 4; 1600]);
        trace.on_finish("submitted", 100);

        let body = fs::read_to_string(dir.join("01LEVEL.jsonl")).unwrap();
        assert!(body.contains(r#""audio_level""#));
        assert!(body.contains(r#""windows":2"#));
        assert!(body.contains(r#""has_signal":true"#));
        assert!(body.contains(r#""clipped":false"#));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn writes_session_boundary_events() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-trace-sess-{}", ulid::Ulid::generate()));
        fs::create_dir_all(&dir).unwrap();

        let mut trace = RecordingObserver::start_in_dir(
            &dir,
            TraceStart {
                enabled: true,
                recording_id: "01SESS".to_string(),
                provider: "doubao".to_string(),
                started_at: "2026-06-16T00:00:00Z".to_string(),
                started_instant: Instant::now(),
            },
        )
        .unwrap();
        trace.on_session(SessionPhase::Start {
            index: 0,
            start_ms: 0,
        });
        trace.on_session(SessionPhase::FinalizeStart {
            index: 0,
            t_ms: 1234,
        });
        trace.on_session(SessionPhase::Done {
            index: 0,
            start_ms: 0,
            end_ms: 1534,
            audio_ms: 1534,
        });
        trace.on_session(SessionPhase::OpenError {
            index: 1,
            t_ms: 2000,
            message: "boom".to_string(),
        });
        trace.on_finish("error", 1534);

        let body = fs::read_to_string(dir.join("01SESS.jsonl")).unwrap();
        assert!(body.contains(r#""event":"session_start""#));
        assert!(body.contains(r#""event":"session_finalize_start""#));
        assert!(body.contains(r#""event":"session_done""#));
        assert!(body.contains(r#""audio_ms":1534"#));
        assert!(body.contains(r#""event":"session_open_error""#));
        assert!(body.contains(r#""message":"boom""#));

        let _ = fs::remove_dir_all(dir);
    }
}
