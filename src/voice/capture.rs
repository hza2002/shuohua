//! Capture model for voice ASR sessions: per-segment / per-session 数据 +
//! recording timeline 的时间换算。
//!
//! 纯数据 + 算术。不持 overlay / state / I/O。

use std::time::Instant;

#[derive(Debug, Clone)]
pub(crate) struct SegmentCapture {
    pub text: String,
    #[allow(dead_code)]
    pub started_at: Instant,
    #[allow(dead_code)]
    pub ended_at: Instant,
}

/// 一次 ASR provider session 在 recording timeline 上的捕获。
/// 一条 recording 携带 1..N 个 session（参见 SCHEMA §2.2）。
#[derive(Debug, Clone)]
pub(crate) struct SessionCapture {
    pub started_at: Instant,
    pub ended_at: Instant,
    pub audio_samples: u64,
    pub segments: Vec<SegmentCapture>,
    pub final_text: Option<String>,
}

pub(crate) fn samples_to_ms(samples: u64) -> u64 {
    samples.saturating_mul(1000) / 16_000
}

pub(crate) fn instant_to_datetime(
    recording_started_at: time::OffsetDateTime,
    recording_started_instant: Instant,
    instant: Instant,
) -> time::OffsetDateTime {
    let delta = instant.saturating_duration_since(recording_started_instant);
    recording_started_at + time::Duration::milliseconds(delta.as_millis() as i64)
}

pub(crate) fn session_text_from_parts(
    segments: &[SegmentCapture],
    final_text: Option<&str>,
) -> String {
    if let Some(text) = final_text.filter(|s| !s.is_empty()) {
        return text.to_string();
    }
    segments.iter().map(|s| s.text.as_str()).collect()
}

pub(crate) fn session_text(session: &SessionCapture) -> String {
    session_text_from_parts(&session.segments, session.final_text.as_deref())
}

pub(crate) fn session_texts(sessions: &[SessionCapture]) -> Vec<String> {
    sessions
        .iter()
        .map(session_text)
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn segment(base: Instant, text: &str, start_ms: u64, end_ms: u64) -> SegmentCapture {
        SegmentCapture {
            text: text.to_string(),
            started_at: base + Duration::from_millis(start_ms),
            ended_at: base + Duration::from_millis(end_ms),
        }
    }

    #[test]
    fn session_text_prefers_final_text_over_segment_concat() {
        let base = Instant::now();
        let session = SessionCapture {
            started_at: base,
            ended_at: base + Duration::from_millis(900),
            audio_samples: 16_000 * 900 / 1_000,
            segments: vec![segment(base, "a", 0, 100), segment(base, "b", 100, 200)],
            final_text: Some("ab.".to_string()),
        };
        assert_eq!(session_text(&session), "ab.");
    }
}
