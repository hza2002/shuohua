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
    pub partial_text: Option<String>,
}

pub(crate) fn samples_to_ms(samples: u64) -> u64 {
    // History stores integer milliseconds. Converting a sample count separately
    // from its start/end timestamps can differ by at most 1ms due to flooring;
    // docs/schema.md explicitly treats that as harmless quantization.
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
    partial_text: Option<&str>,
) -> String {
    if let Some(text) = final_text.filter(|s| !s.is_empty()) {
        return text.to_string();
    }
    let mut text: String = segments.iter().map(|s| s.text.as_str()).collect();
    if let Some(partial) = partial_text.filter(|s| !s.is_empty()) {
        text.push_str(partial);
    }
    text
}

pub(crate) fn session_text(session: &SessionCapture) -> String {
    session_text_from_parts(
        &session.segments,
        session.final_text.as_deref(),
        session.partial_text.as_deref(),
    )
}

pub(crate) fn session_texts(sessions: &[SessionCapture]) -> Vec<String> {
    sessions
        .iter()
        .map(session_text)
        .filter(|s| !s.is_empty())
        .collect()
}

/// 这次录音是否有值得留存的内容（识别文本，或喂过音频的 session）。
///
/// 取消时用它统一决定：有内容则保留 history + retained audio（可能是误触，
/// 让用户从 TUI 找回文本和音频）；无内容（toggle 后立即取消、啥也没说）则
/// 两者都不留，避免产生 TUI 无法关联的孤儿音频文件。
pub(crate) fn has_archivable_content(sessions: &[SessionCapture]) -> bool {
    !session_texts(sessions).is_empty() || sessions.iter().any(|s| s.audio_samples > 0)
}

/// resume（带 seed）录音的留存判据：必须有**新的** ASR 文本才算——普通录音
/// 「喂过音频或有文本」即可，但 resume 若只有音频、没识别出新文本，只是一次没
/// 接上话的尝试；这时写 history / 留音频会盖掉它想续写的那条可恢复记录（见
/// voice.md）。engine（retained audio）与 finish（history）共用，保持两者一致。
pub(crate) fn has_archivable_content_for(sessions: &[SessionCapture], is_resume: bool) -> bool {
    if is_resume {
        return !session_texts(sessions).is_empty();
    }
    has_archivable_content(sessions)
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
            partial_text: Some("ignored".to_string()),
        };
        assert_eq!(session_text(&session), "ab.");
    }

    #[test]
    fn session_text_uses_partial_when_final_is_absent() {
        let base = Instant::now();
        let session = SessionCapture {
            started_at: base,
            ended_at: base + Duration::from_millis(900),
            audio_samples: 16_000 * 900 / 1_000,
            segments: vec![segment(base, "a", 0, 100)],
            final_text: None,
            partial_text: Some("b".to_string()),
        };
        assert_eq!(session_text(&session), "ab");
    }

    #[test]
    fn has_archivable_content_distinguishes_content_from_noise() {
        let base = Instant::now();
        // 无 session：toggle 后立即取消 → 无内容
        assert!(!has_archivable_content(&[]));

        // 喂过音频但没识别出文本（可能误触，音频可找回）→ 有内容
        let audio_only = SessionCapture {
            started_at: base,
            ended_at: base,
            audio_samples: 16_000,
            segments: vec![],
            final_text: None,
            partial_text: None,
        };
        assert!(has_archivable_content(std::slice::from_ref(&audio_only)));

        // 有识别文本 → 有内容
        let with_text = SessionCapture {
            started_at: base,
            ended_at: base,
            audio_samples: 0,
            segments: vec![segment(base, "hi", 0, 100)],
            final_text: None,
            partial_text: None,
        };
        assert!(has_archivable_content(std::slice::from_ref(&with_text)));
    }
}
