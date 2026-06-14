use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::text_stats::TextStats;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub version: u8,
    pub id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub ended_at: OffsetDateTime,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_stats: Option<TextStats>,
    pub status: HistoryStatus,
    pub app: Option<String>,
    pub asr: AsrHistory,
    pub pipeline: Vec<PipelineStepHistory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<HistoryError>,
}

impl HistoryRecord {
    pub fn final_text(&self) -> &str {
        self.pipeline
            .iter()
            .rev()
            .find_map(|step| step.text.as_deref())
            .unwrap_or(&self.asr.raw)
    }

    pub fn text_stats(&self) -> TextStats {
        self.text_stats
            .unwrap_or_else(|| crate::text_stats::compute(self.final_text()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStatus {
    Submitted,
    Canceled,
    Error,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryError {
    pub kind: String,
    pub msg: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsrHistory {
    pub provider: String,
    pub raw: String,
    pub audio_ms: u64,
    pub sessions: Vec<AsrSessionHistory>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsrSessionHistory {
    pub text: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub ended_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineStepHistory {
    pub name: String,
    pub status: PipelineStepStatus,
    pub duration_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStepStatus {
    Ok,
    Error,
    Timeout,
    Skipped,
}

pub fn default_path() -> PathBuf {
    state_dir().join("history.jsonl")
}

pub fn state_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        return PathBuf::from(xdg).join("shuohua");
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state/shuohua")
}

pub fn append_default(record: &HistoryRecord) -> Result<()> {
    append_record(&default_path(), record)
}

pub fn append_record(path: &Path, record: &HistoryRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create history dir {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open history {}", path.display()))?;
    serde_json::to_writer(&mut file, record)
        .with_context(|| format!("serialize history record {}", record.id))?;
    file.write_all(b"\n")
        .with_context(|| format!("write history {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use time::macros::datetime;

    use super::*;

    #[test]
    fn append_writes_one_json_line_with_rfc3339_timestamps() {
        let dir = std::env::temp_dir().join(format!("shuohua-history-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.jsonl");

        let record = HistoryRecord {
            version: 1,
            id: "01HXYZABCDEF0123456789ABCD".to_string(),
            started_at: datetime!(2026-06-13 12:00:00 UTC),
            ended_at: datetime!(2026-06-13 12:00:08 UTC),
            duration_ms: 8000,
            text_stats: Some(crate::text_stats::compute("今天天气真好 我们出去走走")),
            status: HistoryStatus::Submitted,
            app: Some("com.apple.dt.Xcode".to_string()),
            asr: AsrHistory {
                provider: "doubao".to_string(),
                raw: "今天天气真好 我们出去走走".to_string(),
                audio_ms: 5300,
                sessions: vec![AsrSessionHistory {
                    text: "今天天气真好".to_string(),
                    started_at: datetime!(2026-06-13 12:00:00 UTC),
                    ended_at: datetime!(2026-06-13 12:00:03 UTC),
                }],
            },
            pipeline: vec![PipelineStepHistory {
                name: "filler".to_string(),
                status: PipelineStepStatus::Ok,
                duration_ms: 0.3,
                text: Some("今天天气真好 我们出去走走".to_string()),
                error: None,
            }],
            error: None,
        };

        append_record(&path, &record).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(body.lines().count(), 1);
        assert!(body.ends_with('\n'));

        let json: serde_json::Value = serde_json::from_str(body.trim_end()).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["started_at"], "2026-06-13T12:00:00Z");
        assert_eq!(json["ended_at"], "2026-06-13T12:00:08Z");
        assert_eq!(json["text_stats"]["chars"], 13);
        assert_eq!(json["text_stats"]["words"], 12);
        assert_eq!(json["status"], "submitted");
        assert_eq!(
            json["asr"]["sessions"][0]["started_at"],
            "2026-06-13T12:00:00Z"
        );
        assert_eq!(json["pipeline"][0]["status"], "ok");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_text_stats_are_derived_from_final_text() {
        let json = r#"{
            "version": 1,
            "id": "01HXYZABCDEF0123456789ABCD",
            "started_at": "2026-06-13T12:00:00Z",
            "ended_at": "2026-06-13T12:00:08Z",
            "duration_ms": 8000,
            "status": "submitted",
            "app": null,
            "asr": {
                "provider": "doubao",
                "raw": "Hello，你好。",
                "audio_ms": 5300,
                "sessions": []
            },
            "pipeline": []
        }"#;

        let record: HistoryRecord = serde_json::from_str(json).unwrap();

        assert_eq!(record.text_stats, None);
        assert_eq!(record.text_stats().chars, 9);
        assert_eq!(record.text_stats().words, 5);
    }
}
