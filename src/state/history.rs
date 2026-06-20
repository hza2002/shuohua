use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, UtcOffset};

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
    pub status: HistoryStatus,
    pub app: Option<String>,
    pub text: String,
    pub text_stats: TextStats,
    pub asr: AsrHistory,
    pub pipeline: Vec<PipelineStepHistory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<HistoryError>,
}

impl HistoryRecord {
    pub fn text_stats(&self) -> TextStats {
        self.text_stats
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
    pub text: String,
    /// ASR 工作窗口（毫秒）= 首 session.started_at → 末 session.ended_at。
    /// 这是"如果不开 idle_pause、走单 session 会喂出去多少音频"的真实基线。
    /// `duration_ms - audio_ms` 应按有符号数解释：正数是净省下的静音，
    /// 负数是 resume overlap 带来的重复发送开销。空 sessions = 0。
    #[serde(default)]
    pub duration_ms: u64,
    /// 实际喂给 provider 的音频时长（毫秒）= Σ sessions[].audio_ms。
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
    pub audio_ms: u64,
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

pub fn history_dir() -> PathBuf {
    state_dir().join("history")
}

pub fn state_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        return PathBuf::from(xdg).join("shuohua");
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state/shuohua")
}

pub fn append_default(record: &HistoryRecord) -> Result<()> {
    append_record(&path_for_month(record.started_at), record)
}

pub fn path_for_month(now: OffsetDateTime) -> PathBuf {
    path_for_month_in_dir(&history_dir(), now)
}

pub fn path_for_month_in_dir(dir: &Path, now: OffsetDateTime) -> PathBuf {
    let local = now.to_offset(local_offset());
    let name = format!("{:04}-{:02}.jsonl", local.year(), u8::from(local.month()));
    dir.join(name)
}

pub fn monthly_history_files_in_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    match fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                if path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(is_monthly_history_file)
                {
                    files.push(path);
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("read history dir {}", dir.display())),
    }
    files.sort();
    files.reverse();
    Ok(files)
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

fn local_offset() -> UtcOffset {
    UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
}

fn is_monthly_history_file(name: &str) -> bool {
    let Some((stem, "jsonl")) = name.rsplit_once('.') else {
        return false;
    };
    let bytes = stem.as_bytes();
    if bytes.len() != 7 || bytes[4] != b'-' {
        return false;
    }
    if !bytes[..4].iter().all(u8::is_ascii_digit) || !bytes[5..].iter().all(u8::is_ascii_digit) {
        return false;
    }
    matches!(
        &stem[5..],
        "01" | "02" | "03" | "04" | "05" | "06" | "07" | "08" | "09" | "10" | "11" | "12"
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use time::macros::datetime;

    use super::*;

    fn sample_record() -> HistoryRecord {
        HistoryRecord {
            version: 1,
            id: "01HXYZABCDEF0123456789ABCD".to_string(),
            started_at: datetime!(2026-06-13 12:00:00 UTC),
            ended_at: datetime!(2026-06-13 12:00:08 UTC),
            duration_ms: 8000,
            status: HistoryStatus::Submitted,
            app: Some("com.apple.dt.Xcode".to_string()),
            text: "今天天气真好，我们出去走走。".to_string(),
            text_stats: crate::text_stats::compute("今天天气真好，我们出去走走。"),
            asr: AsrHistory {
                provider: "doubao".to_string(),
                text: "今天天气真好 我们出去走走".to_string(),
                duration_ms: 5300,
                audio_ms: 5300,
                sessions: vec![AsrSessionHistory {
                    text: "今天天气真好 我们出去走走".to_string(),
                    started_at: datetime!(2026-06-13 12:00:00 UTC),
                    ended_at: datetime!(2026-06-13 12:00:05 UTC),
                    audio_ms: 5300,
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
        }
    }

    #[test]
    fn append_writes_one_json_line_with_v1_schema() {
        let dir = std::env::temp_dir().join(format!("shuohua-history-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("2026-06.jsonl");

        let record = sample_record();
        append_record(&path, &record).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(body.lines().count(), 1);
        assert!(body.ends_with('\n'));

        let json: serde_json::Value = serde_json::from_str(body.trim_end()).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["text"], "今天天气真好，我们出去走走。");
        assert!(json["text_stats"]["words"].as_u64().unwrap() > 0);
        assert!(json["text_stats"].get("chars").is_none());
        assert_eq!(json["asr"]["text"], "今天天气真好 我们出去走走");
        assert!(json["asr"].get("raw").is_none());
        assert_eq!(json["asr"]["audio_ms"], 5300);
        assert_eq!(json["asr"]["duration_ms"], 5300);
        assert_eq!(json["asr"]["sessions"][0]["audio_ms"], 5300);
        assert_eq!(
            json["asr"]["sessions"][0]["started_at"],
            "2026-06-13T12:00:00Z"
        );
        assert!(
            json.get("error").is_none(),
            "error should be omitted when status=submitted"
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn path_for_month_uses_year_month_jsonl_name() {
        let dir = PathBuf::from("/tmp/shuohua-history-test");
        let path = path_for_month_in_dir(&dir, datetime!(2026-06-13 12:00:00 UTC));
        let file_name = path.file_name().unwrap().to_string_lossy();
        assert!(file_name.ends_with(".jsonl"));
        assert_eq!(file_name.len(), "2026-06.jsonl".len());
    }

    #[test]
    fn monthly_history_files_returns_newest_files_first() {
        let dir = std::env::temp_dir().join(format!("shuohua-history-list-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("2026-05.jsonl"), "").unwrap();
        fs::write(dir.join("2026-07.jsonl"), "").unwrap();
        fs::write(dir.join("notes.txt"), "").unwrap();

        let files = monthly_history_files_in_dir(&dir).unwrap();
        let names: Vec<_> = files
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["2026-07.jsonl", "2026-05.jsonl"]);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn monthly_history_files_ignores_non_monthly_jsonl_files() {
        let dir = std::env::temp_dir().join(format!("shuohua-history-list-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("2026-05.jsonl"), "").unwrap();
        fs::write(dir.join("backup.jsonl"), "").unwrap();
        fs::write(dir.join("2026-5.jsonl"), "").unwrap();
        fs::write(dir.join("2026-13.jsonl"), "").unwrap();

        let files = monthly_history_files_in_dir(&dir).unwrap();
        let names: Vec<_> = files
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["2026-05.jsonl"]);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn error_field_serialized_when_status_not_submitted() {
        let mut record = sample_record();
        record.status = HistoryStatus::Error;
        record.error = Some(HistoryError {
            kind: "asr_timeout".to_string(),
            msg: "no done within 5s".to_string(),
        });
        let s = serde_json::to_string(&record).unwrap();
        let json: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(json["error"]["kind"], "asr_timeout");
    }

    #[test]
    fn canceled_status_may_omit_error_field() {
        let mut record = sample_record();
        record.status = HistoryStatus::Canceled;
        record.error = None;

        let s = serde_json::to_string(&record).unwrap();
        let json: serde_json::Value = serde_json::from_str(&s).unwrap();

        assert_eq!(json["status"], "canceled");
        assert!(json.get("error").is_none());
    }

    #[test]
    fn record_round_trips_through_serde() {
        let record = sample_record();
        let s = serde_json::to_string(&record).unwrap();
        let json: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(json["version"], 1);
        let back: HistoryRecord = serde_json::from_str(&s).unwrap();
        assert_eq!(record, back);
    }
}
