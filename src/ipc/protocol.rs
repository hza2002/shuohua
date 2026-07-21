use serde::{Deserialize, Serialize};

use crate::history::{
    AggregateStats, AnalyticsPeriod, AnalyticsSnapshot, CleanupFilter, CleanupPreview,
    CleanupResult, HistoryRecord, HistoryStatsSnapshot,
};
use crate::state::{AudioMeter, SessionMeta, SessionPhase};

pub const PROTO_VERSION: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireState {
    Idle,
    Recording,
    Stopping,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Command {
    Subscribe,
    StartRecording,
    StopRecording,
    CancelRecording,
    ReloadConfig,
    GetHistory {
        #[serde(default = "default_history_limit")]
        limit: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
    },
    GetHistoryStats,
    GetHistoryAnalytics {
        period: AnalyticsPeriod,
        anchor: String,
    },
    DeleteAudio {
        id: String,
    },
    DeleteHistory {
        id: String,
    },
    PreviewHistoryCleanup {
        filter: CleanupFilter,
    },
    ExecuteHistoryCleanup {
        filter: CleanupFilter,
        ids: Vec<String>,
    },
    DaemonStatus,
    Shutdown,
}

fn default_history_limit() -> usize {
    50
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Snapshot {
        proto_version: u8,
        state: WireState,
        recording: Option<String>,
        started_at: Option<String>,
        app: Option<String>,
        app_name: Option<String>,
        dur_ms: u64,
        words: u32,
        segments: Vec<String>,
        partial: String,
    },
    StateChanged {
        state: WireState,
        recording_id: Option<String>,
        started_at: Option<String>,
    },
    AppChanged {
        app: Option<String>,
        app_name: Option<String>,
    },
    StatsChanged {
        dur_ms: u64,
        words: u32,
    },
    Partial {
        recording_id: String,
        text: String,
    },
    Segment {
        recording_id: String,
        text: String,
    },
    PipelineStep {
        recording_id: String,
        name: String,
        status: String,
        duration_ms: f64,
        text: Option<String>,
        error: Option<String>,
    },
    AudioMeter {
        recording_id: String,
        meter: AudioMeter,
    },
    SessionMeta {
        recording_id: String,
        meta: SessionMeta,
    },
    SessionPhase {
        recording_id: String,
        phase: SessionPhase,
    },
    HistoryAppended {
        record: Box<HistoryRecord>,
    },
    History {
        records: Vec<HistoryRecord>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        matched: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stats: Option<AggregateStats>,
    },
    HistoryStats {
        snapshot: HistoryStatsSnapshot,
    },
    HistoryAnalytics {
        snapshot: AnalyticsSnapshot,
    },
    HistoryChanged,
    AudioDeleted {
        id: String,
        deleted: bool,
    },
    HistoryDeleted {
        id: String,
        record_deleted: bool,
        audio_deleted: bool,
        audio_error: Option<String>,
    },
    HistoryCleanupPreview {
        preview: CleanupPreview,
    },
    HistoryCleanupDone {
        result: CleanupResult,
    },
    DaemonStatus {
        pid: u32,
        uptime_ms: u64,
        state: WireState,
        recording_id: Option<String>,
    },
    ConfigReloaded {
        path: String,
    },
    Error {
        recording_id: Option<String>,
        kind: String,
        msg: String,
    },
}

pub fn encode_command(command: &Command) -> serde_json::Result<String> {
    encode_line(command)
}

pub fn decode_command(line: &str) -> serde_json::Result<Command> {
    serde_json::from_str(line.trim_end())
}

pub fn encode_event(event: &Event) -> serde_json::Result<String> {
    encode_line(event)
}

pub fn decode_event(line: &str) -> serde_json::Result<Event> {
    serde_json::from_str(line.trim_end())
}

fn encode_line<T: Serialize>(value: &T) -> serde_json::Result<String> {
    let mut line = serde_json::to_string(value)?;
    line.push('\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_round_trips_line_delimited_json() {
        let command = Command::GetHistory {
            limit: 50,
            before: Some("2026-06-12T22:00:00Z".to_string()),
            before_id: Some("01HXYZ".to_string()),
            query: Some("rust".to_string()),
        };

        let line = encode_command(&command).unwrap();

        assert!(line.ends_with('\n'));
        assert_eq!(decode_command(&line).unwrap(), command);
    }

    #[test]
    fn shutdown_command_round_trips() {
        let line = encode_command(&Command::Shutdown).unwrap();

        assert!(line.contains(r#""op":"shutdown""#));
        assert_eq!(decode_command(&line).unwrap(), Command::Shutdown);
    }

    #[test]
    fn history_service_commands_round_trip_without_bumping_proto_version() {
        assert_eq!(PROTO_VERSION, 2);
        let commands = vec![
            Command::GetHistoryStats,
            Command::GetHistoryAnalytics {
                period: AnalyticsPeriod::Month,
                anchor: "2026-06".to_string(),
            },
            Command::DeleteAudio {
                id: "01HAUDIO".to_string(),
            },
            Command::DeleteHistory {
                id: "01HHISTORY".to_string(),
            },
        ];

        for command in commands {
            let line = encode_command(&command).unwrap();
            assert_eq!(decode_command(&line).unwrap(), command);
        }
    }

    #[test]
    fn history_service_events_round_trip() {
        let stats = HistoryStatsSnapshot {
            status: crate::history::HistoryStatsStatus::Ready,
            total: crate::history::AggregateStats {
                records: 1,
                words: 2,
                duration_ms: 3,
                asr_duration_ms: 4,
                asr_audio_ms: 5,
            },
            current_month: crate::history::AggregateStats::default(),
            today: crate::history::AggregateStats::default(),
            error: None,
        };
        let analytics = AnalyticsSnapshot {
            status: crate::history::HistoryStatsStatus::Ready,
            period: AnalyticsPeriod::Month,
            anchor: "2026-06".to_string(),
            points: Vec::new(),
            error: None,
        };
        let events = vec![
            Event::History {
                records: Vec::new(),
                matched: Some(23),
                stats: Some(crate::history::AggregateStats {
                    records: 23,
                    words: 45,
                    duration_ms: 67,
                    asr_duration_ms: 60,
                    asr_audio_ms: 50,
                }),
            },
            Event::HistoryAppended {
                record: Box::new(HistoryRecord {
                    version: 1,
                    id: "01HLEGACY".to_string(),
                    started_at: time::OffsetDateTime::UNIX_EPOCH,
                    ended_at: time::OffsetDateTime::UNIX_EPOCH,
                    duration_ms: 0,
                    status: crate::history::HistoryStatus::Submitted,
                    app: None,
                    text: String::new(),
                    text_stats: crate::text_stats::TextStats { words: 0 },
                    asr: crate::history::AsrHistory {
                        provider: "test".to_string(),
                        text: String::new(),
                        duration_ms: 0,
                        audio_ms: 0,
                        sessions: Vec::new(),
                    },
                    pipeline: Vec::new(),
                    error: None,
                }),
            },
            Event::HistoryStats { snapshot: stats },
            Event::HistoryAnalytics {
                snapshot: analytics,
            },
            Event::HistoryChanged,
            Event::AudioDeleted {
                id: "01HAUDIO".to_string(),
                deleted: true,
            },
            Event::HistoryDeleted {
                id: "01HHISTORY".to_string(),
                record_deleted: true,
                audio_deleted: false,
                audio_error: Some("missing".to_string()),
            },
        ];

        for event in events {
            let line = encode_event(&event).unwrap();
            assert_eq!(decode_event(&line).unwrap(), event);
        }
    }

    #[test]
    fn history_cleanup_commands_round_trip() {
        let filter = crate::history::CleanupFilter {
            scope: crate::history::CleanupScope::AudioOnly,
            window: crate::history::CleanupWindow::OlderThanDays(30),
        };
        let record_filter = crate::history::CleanupFilter {
            scope: crate::history::CleanupScope::RecordAndAudio,
            window: crate::history::CleanupWindow::All,
        };
        let commands = vec![
            Command::PreviewHistoryCleanup { filter },
            Command::ExecuteHistoryCleanup {
                filter,
                ids: vec!["01HAUDIO".to_string()],
            },
            Command::ExecuteHistoryCleanup {
                filter: record_filter,
                ids: vec!["01HHISTORY".to_string()],
            },
        ];
        for command in commands {
            let line = encode_command(&command).unwrap();
            assert_eq!(decode_command(&line).unwrap(), command);
        }
    }

    #[test]
    fn history_cleanup_events_round_trip() {
        let filter = crate::history::CleanupFilter {
            scope: crate::history::CleanupScope::AudioOnly,
            window: crate::history::CleanupWindow::All,
        };
        let preview_event = Event::HistoryCleanupPreview {
            preview: crate::history::CleanupPreview {
                filter,
                ids: vec!["01HAUDIO".to_string()],
                audio_bytes: 333_447_168,
                audio_ms: 5_300_000,
                oldest: None,
                newest: None,
                warnings: Vec::new(),
            },
        };
        let done_event = Event::HistoryCleanupDone {
            result: crate::history::CleanupResult {
                requested: 42,
                deleted: 41,
                missing: 1,
                errors: Vec::new(),
            },
        };
        for event in [preview_event, done_event] {
            let line = encode_event(&event).unwrap();
            assert_eq!(decode_event(&line).unwrap(), event);
        }
    }

    #[test]
    fn segment_event_round_trips_with_tagged_schema() {
        let event = Event::Segment {
            recording_id: "01HXYZ".to_string(),
            text: "hello".to_string(),
        };

        let line = encode_event(&event).unwrap();

        assert!(line.contains(r#""event":"segment""#));
        assert_eq!(decode_event(&line).unwrap(), event);
    }

    #[test]
    fn audio_meter_event_round_trips() {
        let event = Event::AudioMeter {
            recording_id: "01HXYZ".to_string(),
            meter: AudioMeter {
                rms: 0.25,
                peak: 0.75,
                clipped: false,
                vad_probability: Some(0.8),
                vad_speech: Some(true),
            },
        };

        let line = encode_event(&event).unwrap();

        assert!(line.contains(r#""event":"audio_meter""#));
        assert_eq!(decode_event(&line).unwrap(), event);
    }

    #[test]
    fn session_meta_event_round_trips() {
        let event = Event::SessionMeta {
            recording_id: "01HXYZ".to_string(),
            meta: SessionMeta {
                provider: "doubao".to_string(),
                chain: "zh_filter → deepseek".to_string(),
                vad: "silero".to_string(),
                hotwords: 3,
            },
        };

        let line = encode_event(&event).unwrap();

        assert!(line.contains(r#""event":"session_meta""#));
        assert_eq!(decode_event(&line).unwrap(), event);
    }

    #[test]
    fn session_phase_event_round_trips() {
        let event = Event::SessionPhase {
            recording_id: "01HXYZ".to_string(),
            phase: SessionPhase::Idle,
        };

        let line = encode_event(&event).unwrap();

        assert!(line.contains(r#""event":"session_phase""#));
        assert_eq!(decode_event(&line).unwrap(), event);
    }
}
