#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "dev-vad-trace"), allow(dead_code))]
pub struct TraceStart {
    pub enabled: bool,
    pub recording_id: String,
    pub provider: String,
    pub started_at: String,
}

#[cfg(feature = "dev-vad-trace")]
mod imp {
    use super::TraceStart;
    use crate::voice::vad::{VadFrame, VadPolicy};
    use anyhow::{Context, Result};
    use serde_json::json;
    use std::fs::{File, OpenOptions};
    use std::io::{BufWriter, Write};
    use std::path::{Path, PathBuf};

    const SAMPLE_RATE: u64 = 16_000;
    const SILERO_CHUNK_SAMPLES: usize = 512;
    const VAD_THRESHOLD: f32 = 0.5;
    const PRE_ROLL_MS: u64 = 300;

    pub struct TraceRecorder {
        inner: Option<TraceInner>,
    }

    struct TraceInner {
        writer: BufWriter<File>,
        vad: SileroTrace,
    }

    struct SileroTrace {
        detector: voice_activity_detector::VoiceActivityDetector,
        buffer: Vec<i16>,
        sample_offset: u64,
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

    impl TraceRecorder {
        pub fn start(start: TraceStart) -> Self {
            match Self::start_in_dir(&default_trace_dir(), start) {
                Ok(trace) => trace,
                Err(e) => {
                    eprintln!("[trace] disabled: {e:#}");
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

        pub fn pcm_frame(&mut self, samples: &[i16]) {
            let Some(inner) = self.inner.as_mut() else {
                return;
            };
            let events = inner.vad.accept(samples);
            for event in events {
                inner.write(event);
            }
        }

        pub fn provider_opened(&mut self, t_ms: u64) {
            self.write(json!({"event": "provider_opened", "t_ms": t_ms}));
        }

        pub fn asr_partial(&mut self, t_ms: u64, seq: u64, text: &str) {
            self.write(json!({
                "event": "asr_partial",
                "t_ms": t_ms,
                "seq": seq,
                "text": text,
            }));
        }

        pub fn asr_segment(&mut self, t_ms: u64, text: &str, start_ms: u64, end_ms: u64) {
            self.write(json!({
                "event": "asr_segment",
                "t_ms": t_ms,
                "text": text,
                "start_ms": start_ms,
                "end_ms": end_ms,
            }));
        }

        pub fn asr_error(&mut self, t_ms: u64, message: &str) {
            self.write(json!({
                "event": "asr_error",
                "t_ms": t_ms,
                "message": message,
            }));
        }

        pub fn asr_done(&mut self, t_ms: u64) {
            self.write(json!({"event": "asr_done", "t_ms": t_ms}));
        }

        pub fn finish(&mut self, status: &str, audio_ms: u64) {
            let Some(inner) = self.inner.as_mut() else {
                return;
            };
            let summary = inner.vad.finish(audio_ms);
            inner.write(json!({
                "event": "recording_end",
                "status": status,
                "audio_ms": audio_ms,
                "vad": {
                    "active_ms": summary.active_ms,
                    "saved_ms": audio_ms.saturating_sub(summary.active_ms),
                    "sessions": summary.sessions,
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

    impl SileroTrace {
        fn new() -> Result<Self> {
            let policy = VadPolicy::default();
            let detector = voice_activity_detector::VoiceActivityDetector::builder()
                .sample_rate(SAMPLE_RATE as i64)
                .chunk_size(SILERO_CHUNK_SAMPLES)
                .build()
                .map_err(|e| anyhow::anyhow!("create Silero VAD: {e}"))?;
            Ok(Self {
                detector,
                buffer: Vec::with_capacity(SILERO_CHUNK_SAMPLES),
                sample_offset: 0,
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
            self.buffer.extend_from_slice(samples);
            while self.buffer.len() >= SILERO_CHUNK_SAMPLES {
                let chunk: Vec<i16> = self.buffer.drain(..SILERO_CHUNK_SAMPLES).collect();
                let start_sample = self.sample_offset;
                self.sample_offset += SILERO_CHUNK_SAMPLES as u64;
                let start_ms = samples_to_ms(start_sample);
                let end_ms = samples_to_ms(self.sample_offset);
                let probability = self.detector.predict(chunk.iter().copied());
                let speech = probability >= VAD_THRESHOLD;
                out.push(json!({
                    "event": "vad_frame",
                    "start_ms": start_ms,
                    "end_ms": end_ms,
                    "probability": probability,
                    "speech": speech,
                }));
                if let Some(event) = self.accept_vad_frame(
                    if speech {
                        VadFrame::Speech
                    } else {
                        VadFrame::Silence
                    },
                    start_ms,
                    end_ms,
                ) {
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

    fn default_trace_dir() -> PathBuf {
        crate::state::history::state_dir().join("traces")
    }
}

#[cfg(not(feature = "dev-vad-trace"))]
mod imp {
    use super::TraceStart;
    use std::path::Path;

    #[derive(Debug, Clone)]
    pub struct TraceRecorder;

    impl TraceRecorder {
        pub fn start(_start: TraceStart) -> Self {
            Self
        }

        #[allow(dead_code)]
        pub fn start_in_dir(_base: &Path, _start: TraceStart) -> anyhow::Result<Self> {
            Ok(Self)
        }

        pub fn pcm_frame(&mut self, _samples: &[i16]) {}
        pub fn provider_opened(&mut self, _t_ms: u64) {}
        pub fn asr_partial(&mut self, _t_ms: u64, _seq: u64, _text: &str) {}
        pub fn asr_segment(&mut self, _t_ms: u64, _text: &str, _start_ms: u64, _end_ms: u64) {}
        pub fn asr_error(&mut self, _t_ms: u64, _message: &str) {}
        pub fn asr_done(&mut self, _t_ms: u64) {}
        pub fn finish(&mut self, _status: &str, _audio_ms: u64) {}
    }
}

pub use imp::TraceRecorder;

#[cfg(all(test, feature = "dev-vad-trace"))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn writes_jsonl_trace_events_and_summary() {
        let dir = std::env::temp_dir().join(format!("shuohua-trace-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();

        let mut trace = TraceRecorder::start_in_dir(
            &dir,
            TraceStart {
                enabled: true,
                recording_id: "01TRACE".to_string(),
                provider: "doubao".to_string(),
                started_at: "2026-06-16T00:00:00Z".to_string(),
            },
        )
        .unwrap();
        trace.asr_partial(10, 1, "测");
        trace.asr_segment(20, "测试", 0, 500);
        trace.finish("submitted", 1600);

        let body = fs::read_to_string(dir.join("01TRACE.jsonl")).unwrap();
        assert!(body.contains(r#""event":"recording_start""#));
        assert!(body.contains(r#""event":"asr_partial""#));
        assert!(body.contains(r#""event":"asr_segment""#));
        assert!(body.contains(r#""event":"recording_end""#));
        assert!(body.contains(r#""audio_ms":1600"#));

        let _ = fs::remove_dir_all(dir);
    }
}
