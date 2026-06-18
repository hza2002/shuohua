//! HistoryRecord 构造与落盘。
//!
//! 从 [`SessionCapture`] + post pipeline + status 构造 schema v2 record，
//! append 到 monthly history JSONL。
//!
//! 输入数据全部来自 [`crate::voice::capture`] 和 orchestration 状态；本模块不
//! 触碰 overlay / state 广播 / dispatch。

use std::time::Instant;

use crate::post::{self, PipelineStepStatus};
use crate::state::history::{
    self, AsrHistory, AsrSessionHistory, HistoryError, HistoryRecord, HistoryStatus,
    PipelineStepHistory, PipelineStepStatus as HistoryPipelineStepStatus,
};
use crate::voice::capture::{instant_to_datetime, samples_to_ms, session_text, SessionCapture};

pub(crate) struct HistoryInput {
    pub id: String,
    pub provider: String,
    pub started_at: time::OffsetDateTime,
    pub ended_at: time::OffsetDateTime,
    pub started_instant: Instant,
    pub asr_text: String,
    pub final_text: String,
    pub sessions: Vec<SessionCapture>,
    pub pipeline: Vec<PipelineStepHistory>,
    pub app: Option<String>,
    pub status: HistoryStatus,
    pub error: Option<HistoryError>,
}

pub(crate) fn build_record(input: HistoryInput) -> HistoryRecord {
    let audio_ms: u64 = input
        .sessions
        .iter()
        .map(|s| samples_to_ms(s.audio_samples))
        .sum();
    let all_sessions_empty = input
        .sessions
        .iter()
        .all(|s| s.segments.is_empty() && s.audio_samples == 0);
    let sessions = if all_sessions_empty && input.asr_text.is_empty() {
        Vec::new()
    } else {
        build_asr_sessions(&input.sessions, input.started_at, input.started_instant)
    };

    // ASR 工作窗口（首段 started_at → 末段 ended_at）。空 sessions 直接 0。
    let asr_duration_ms = match (sessions.first(), sessions.last()) {
        (Some(first), Some(last)) => (last.ended_at - first.started_at)
            .whole_milliseconds()
            .max(0) as u64,
        _ => 0,
    };

    HistoryRecord {
        version: 2,
        id: input.id,
        started_at: input.started_at,
        ended_at: input.ended_at,
        duration_ms: (input.ended_at - input.started_at)
            .whole_milliseconds()
            .max(0) as u64,
        status: input.status,
        app: input.app,
        text: input.final_text.clone(),
        text_stats: crate::text_stats::compute(&input.final_text),
        asr: AsrHistory {
            provider: input.provider,
            text: input.asr_text,
            duration_ms: asr_duration_ms,
            audio_ms,
            sessions,
        },
        pipeline: input.pipeline,
        error: input.error,
    }
}

pub(crate) fn append_history(input: HistoryInput) -> Option<HistoryRecord> {
    let record = build_record(input);
    if let Err(e) = history::append_default(&record) {
        tracing::error!(
            recording_id = %record.id,
            error = ?e,
            "history append failed"
        );
        return None;
    }
    tracing::info!(
        recording_id = %record.id,
        status = ?record.status,
        provider = %record.asr.provider,
        audio_ms = record.asr.audio_ms,
        session_count = record.asr.sessions.len(),
        pipeline_steps = record.pipeline.len(),
        "recording ended"
    );
    Some(record)
}

fn build_asr_sessions(
    sessions: &[SessionCapture],
    recording_started_at: time::OffsetDateTime,
    recording_started_instant: Instant,
) -> Vec<AsrSessionHistory> {
    sessions
        .iter()
        .map(|s| AsrSessionHistory {
            text: session_text(s),
            started_at: instant_to_datetime(
                recording_started_at,
                recording_started_instant,
                s.started_at,
            ),
            ended_at: instant_to_datetime(
                recording_started_at,
                recording_started_instant,
                s.ended_at,
            ),
            audio_ms: samples_to_ms(s.audio_samples),
        })
        .collect()
}

impl From<post::PipelineStep> for PipelineStepHistory {
    fn from(step: post::PipelineStep) -> Self {
        Self {
            name: step.name,
            status: match step.status {
                PipelineStepStatus::Ok => HistoryPipelineStepStatus::Ok,
                PipelineStepStatus::Error => HistoryPipelineStepStatus::Error,
                PipelineStepStatus::Timeout => HistoryPipelineStepStatus::Timeout,
                PipelineStepStatus::Skipped => HistoryPipelineStepStatus::Skipped,
            },
            duration_ms: step.duration_ms,
            text: step.text,
            error: step.error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::capture::SegmentCapture;
    use std::time::{Duration, Instant};
    use time::OffsetDateTime;

    fn segment(base: Instant, text: &str, start_ms: u64, end_ms: u64) -> SegmentCapture {
        SegmentCapture {
            text: text.to_string(),
            started_at: base + Duration::from_millis(start_ms),
            ended_at: base + Duration::from_millis(end_ms),
        }
    }

    #[test]
    fn single_session_collapses_multiple_segments_into_one_entry() {
        let recording_start = OffsetDateTime::from_unix_timestamp(1_750_000_000).unwrap();
        let base = Instant::now();
        let segments = vec![
            segment(base, "alpha ", 0, 500),
            segment(base, "beta ", 600, 1_000),
            segment(base, "gamma", 1_100, 1_500),
        ];
        let sessions = vec![SessionCapture {
            started_at: base,
            ended_at: base + Duration::from_millis(1_500),
            audio_samples: 16_000 * 1_500 / 1_000,
            segments,
            final_text: None,
        }];
        let input = HistoryInput {
            id: "01HXYZABCDEF0123456789ABCD".to_string(),
            provider: "fake".to_string(),
            started_at: recording_start,
            ended_at: recording_start + time::Duration::milliseconds(2_000),
            started_instant: base,
            asr_text: "alpha beta gamma".to_string(),
            final_text: "alpha beta gamma.".to_string(),
            sessions,
            pipeline: Vec::new(),
            app: None,
            status: HistoryStatus::Submitted,
            error: None,
        };
        let record = build_record(input);
        assert_eq!(record.version, 2);
        assert_eq!(record.text, "alpha beta gamma.");
        assert_eq!(record.asr.text, "alpha beta gamma");
        assert_eq!(record.asr.audio_ms, 1_500);
        assert_eq!(record.asr.sessions.len(), 1);
        let session = &record.asr.sessions[0];
        assert_eq!(session.text, "alpha beta gamma");
        assert_eq!(session.audio_ms, 1_500);
        assert!(session.started_at <= session.ended_at);
        assert!(session.started_at >= record.started_at);
        assert!(session.ended_at <= record.ended_at);
    }

    #[test]
    fn empty_sessions_and_empty_asr_text_produce_no_sessions() {
        let recording_start = OffsetDateTime::from_unix_timestamp(1_750_000_000).unwrap();
        let base = Instant::now();
        let input = HistoryInput {
            id: "01HXYZABCDEF0123456789ABCD".to_string(),
            provider: "fake".to_string(),
            started_at: recording_start,
            ended_at: recording_start + time::Duration::milliseconds(500),
            started_instant: base,
            asr_text: String::new(),
            final_text: String::new(),
            sessions: Vec::new(),
            pipeline: Vec::new(),
            app: None,
            status: HistoryStatus::Canceled,
            error: None,
        };
        let record = build_record(input);
        assert!(record.asr.sessions.is_empty());
        assert_eq!(record.asr.text, "");
    }

    #[test]
    fn multi_session_history_sums_audio_ms_and_preserves_session_count() {
        let recording_start = OffsetDateTime::from_unix_timestamp(1_750_000_000).unwrap();
        let base = Instant::now();
        let sessions = vec![
            SessionCapture {
                started_at: base,
                ended_at: base + Duration::from_millis(800),
                audio_samples: 16_000 * 800 / 1_000,
                segments: vec![segment(base, "hello ", 0, 600)],
                final_text: None,
            },
            SessionCapture {
                started_at: base + Duration::from_millis(2_000),
                ended_at: base + Duration::from_millis(2_900),
                audio_samples: 16_000 * 900 / 1_000,
                segments: vec![segment(base, "world", 2_100, 2_800)],
                final_text: None,
            },
        ];
        let input = HistoryInput {
            id: "01HXYZABCDEF0123456789ABCD".to_string(),
            provider: "fake".to_string(),
            started_at: recording_start,
            ended_at: recording_start + time::Duration::milliseconds(3_000),
            started_instant: base,
            asr_text: "hello world".to_string(),
            final_text: "hello world.".to_string(),
            sessions,
            pipeline: Vec::new(),
            app: None,
            status: HistoryStatus::Submitted,
            error: None,
        };
        let record = build_record(input);
        assert_eq!(record.asr.sessions.len(), 2);
        assert_eq!(record.asr.sessions[0].text, "hello ");
        assert_eq!(record.asr.sessions[1].text, "world");
        assert_eq!(record.asr.sessions[0].audio_ms, 800);
        assert_eq!(record.asr.sessions[1].audio_ms, 900);
        assert_eq!(record.asr.audio_ms, 800 + 900);

        let asr_duration_ms = (record.asr.sessions.last().unwrap().ended_at
            - record.asr.sessions.first().unwrap().started_at)
            .whole_milliseconds() as u64;
        assert_eq!(asr_duration_ms, 2_900);
        assert_eq!(record.asr.duration_ms, 2_900);
        assert_eq!(
            record.asr.duration_ms - record.asr.audio_ms,
            2_900 - (800 + 900)
        );
    }

    #[test]
    fn asr_duration_ms_is_zero_when_no_sessions() {
        let recording_start = OffsetDateTime::from_unix_timestamp(1_750_000_000).unwrap();
        let base = Instant::now();
        let input = HistoryInput {
            id: "01HXYZABCDEF0123456789ABCD".to_string(),
            provider: "fake".to_string(),
            started_at: recording_start,
            ended_at: recording_start + time::Duration::milliseconds(500),
            started_instant: base,
            asr_text: String::new(),
            final_text: String::new(),
            sessions: Vec::new(),
            pipeline: Vec::new(),
            app: None,
            status: HistoryStatus::Canceled,
            error: None,
        };
        let record = build_record(input);
        assert!(record.asr.sessions.is_empty());
        assert_eq!(record.asr.duration_ms, 0);
        assert_eq!(record.asr.audio_ms, 0);
    }

    #[test]
    fn overlapping_session_instants_are_preserved() {
        let recording_start = OffsetDateTime::from_unix_timestamp(1_750_000_000).unwrap();
        let base = Instant::now();
        let sessions = vec![
            SessionCapture {
                started_at: base,
                ended_at: base + Duration::from_millis(800),
                audio_samples: 16_000 * 800 / 1_000,
                segments: vec![segment(base, "a", 0, 700)],
                final_text: None,
            },
            SessionCapture {
                started_at: base + Duration::from_millis(700),
                ended_at: base + Duration::from_millis(1_500),
                audio_samples: 16_000 * 800 / 1_000,
                segments: vec![segment(base, "b", 800, 1_400)],
                final_text: None,
            },
        ];
        let input = HistoryInput {
            id: "01HXYZABCDEF0123456789ABCD".to_string(),
            provider: "fake".to_string(),
            started_at: recording_start,
            ended_at: recording_start + time::Duration::milliseconds(2_000),
            started_instant: base,
            asr_text: "ab".to_string(),
            final_text: "ab".to_string(),
            sessions,
            pipeline: Vec::new(),
            app: None,
            status: HistoryStatus::Submitted,
            error: None,
        };
        let record = build_record(input);
        assert_eq!(record.asr.sessions.len(), 2);
        assert!(record.asr.sessions[1].started_at < record.asr.sessions[0].ended_at);
    }
}
