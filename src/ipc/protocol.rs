use serde::{Deserialize, Serialize};

use crate::state::history::HistoryRecord;
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
        query: Option<String>,
    },
    DaemonStatus,
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
        stats: Stats,
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

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stats {
    pub history_count: usize,
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
            query: Some("rust".to_string()),
        };

        let line = encode_command(&command).unwrap();

        assert!(line.ends_with('\n'));
        assert_eq!(decode_command(&line).unwrap(), command);
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
                chain: "rule:filler → llm:deepseek".to_string(),
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
