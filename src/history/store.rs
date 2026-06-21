use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use time::{OffsetDateTime, UtcOffset};

use crate::history::HistoryRecord;
use crate::paths::StateDirs;

pub fn history_dir() -> PathBuf {
    StateDirs::discover().history()
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
    let mut line = serde_json::to_vec(record)
        .with_context(|| format!("serialize history record {}", record.id))?;
    line.push(b'\n');

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create history dir {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open history {}", path.display()))?;
    file.write_all(&line)
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
    use crate::history::{
        AsrHistory, AsrSessionHistory, HistoryError, HistoryStatus, PipelineStepHistory,
        PipelineStepStatus,
    };

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
    fn empty_status_serializes_as_empty_without_error() {
        let mut record = sample_record();
        record.status = HistoryStatus::Empty;
        record.text.clear();
        record.asr.text.clear();
        record.error = None;

        let s = serde_json::to_string(&record).unwrap();
        let json: serde_json::Value = serde_json::from_str(&s).unwrap();

        assert_eq!(json["status"], "empty");
        assert_eq!(json["text"], "");
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
