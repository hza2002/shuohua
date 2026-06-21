use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use time::{OffsetDateTime, UtcOffset};

use crate::history::HistoryRecord;
use crate::paths::StateDirs;

pub fn history_dir() -> PathBuf {
    StateDirs::discover().history()
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

pub(crate) fn delete_record_in_dir(
    dir: &Path,
    id: &str,
    local_offset: UtcOffset,
    before_rename: impl FnOnce(),
) -> Result<RecordDeleteOutcome> {
    crate::history::assets::validate_recording_id(id)?;
    let likely = likely_path_for_id_in_dir(dir, id, local_offset)?;
    let mut candidates = Vec::new();
    candidates.push(likely.clone());
    for path in monthly_history_files_in_dir(dir)? {
        if path != likely {
            candidates.push(path);
        }
    }

    for path in candidates {
        if !path.exists() {
            continue;
        }
        let before = fingerprint_delete_source(&path)?;
        let read = read_records_for_delete(&path, local_offset)?;
        let Some(position) = read.records.iter().position(|record| record.id == id) else {
            continue;
        };
        let mut retained = read.records;
        retained.remove(position);
        let tmp = unique_delete_temp_path(&path);
        let rewrite_result = rewrite_records_to_temp(&tmp, &retained).and_then(|()| {
            before_rename();
            let after = fingerprint_delete_source(&path)?;
            if before != after {
                bail!(
                    "history source changed before delete rename: {}",
                    path.display()
                );
            }
            fs::rename(&tmp, &path).with_context(|| format!("replace history {}", path.display()))
        });
        if let Err(error) = rewrite_result {
            let _ = fs::remove_file(&tmp);
            return Err(error);
        }
        return Ok(RecordDeleteOutcome { deleted: true });
    }

    Ok(RecordDeleteOutcome { deleted: false })
}

#[derive(Debug)]
pub(crate) struct RecordDeleteOutcome {
    pub deleted: bool,
}

fn local_offset() -> UtcOffset {
    UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
}

fn likely_path_for_id_in_dir(dir: &Path, id: &str, local_offset: UtcOffset) -> Result<PathBuf> {
    let ulid = crate::history::assets::validate_recording_id(id)?;
    let started_at = system_time_to_offset_datetime(ulid.datetime())?;
    let local = started_at.to_offset(local_offset);
    Ok(dir.join(format!(
        "{:04}-{:02}.jsonl",
        local.year(),
        u8::from(local.month())
    )))
}

fn read_records_for_delete(path: &Path, local_offset: UtcOffset) -> Result<DeleteShardRead> {
    let body =
        fs::read_to_string(path).with_context(|| format!("read history {}", path.display()))?;
    let month_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid history filename {}", path.display()))?;
    let (year, month) = parse_month_file_name(month_name)?;
    let mut records = Vec::new();
    for (index, line) in body.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: HistoryRecord = serde_json::from_str(line)
            .with_context(|| format!("parse history line in {}", path.display()))?;
        let local = record.started_at.to_offset(local_offset);
        if local.year() != year || u8::from(local.month()) != month {
            bail!(
                "history record month mismatch at {}:{}",
                path.display(),
                index + 1
            );
        }
        records.push(record);
    }
    Ok(DeleteShardRead { records })
}

struct DeleteShardRead {
    records: Vec<HistoryRecord>,
}

fn rewrite_records_to_temp(path: &Path, records: &[HistoryRecord]) -> Result<()> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .with_context(|| format!("create temp history {}", path.display()))?;
    for record in records {
        let mut line = serde_json::to_vec(record)
            .with_context(|| format!("serialize history record {}", record.id))?;
        line.push(b'\n');
        file.write_all(&line)
            .with_context(|| format!("write temp history {}", path.display()))?;
    }
    file.flush()
        .with_context(|| format!("flush temp history {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("sync temp history {}", path.display()))?;
    Ok(())
}

fn unique_delete_temp_path(source: &Path) -> PathBuf {
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("history.jsonl");
    source.with_file_name(format!("{name}.tmp-delete-{}", ulid::Ulid::new()))
}

fn parse_month_file_name(name: &str) -> Result<(i32, u8)> {
    if !is_monthly_history_file(name) {
        bail!("invalid monthly history filename: {name}");
    }
    let year = name[..4].parse().context("parse history year")?;
    let month = name[5..7].parse().context("parse history month")?;
    Ok((year, month))
}

fn system_time_to_offset_datetime(value: std::time::SystemTime) -> Result<OffsetDateTime> {
    let duration = value
        .duration_since(std::time::UNIX_EPOCH)
        .context("ulid timestamp before unix epoch")?;
    let seconds = i64::try_from(duration.as_secs()).context("ulid timestamp too large")?;
    OffsetDateTime::from_unix_timestamp(seconds)
        .context("convert ulid timestamp")?
        .replace_nanosecond(duration.subsec_nanos())
        .context("convert ulid timestamp nanos")
}

#[cfg(unix)]
fn fingerprint_delete_source(path: &Path) -> Result<DeleteSourceFingerprint> {
    use std::os::unix::fs::MetadataExt;

    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("stat history {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("history file must not be a symlink: {}", path.display());
    }
    if !metadata.file_type().is_file() {
        bail!("history path must be a regular file: {}", path.display());
    }
    Ok(DeleteSourceFingerprint {
        dev: metadata.dev(),
        ino: metadata.ino(),
        mtime_sec: metadata.mtime(),
        mtime_nsec: metadata.mtime_nsec(),
        ctime_sec: metadata.ctime(),
        ctime_nsec: metadata.ctime_nsec(),
        len: metadata.len(),
    })
}

#[cfg(not(unix))]
fn fingerprint_delete_source(path: &Path) -> Result<DeleteSourceFingerprint> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("stat history {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("history file must not be a symlink: {}", path.display());
    }
    if !metadata.file_type().is_file() {
        bail!("history path must be a regular file: {}", path.display());
    }
    let modified = metadata.modified()?;
    let changed = metadata.created().unwrap_or(modified);
    Ok(DeleteSourceFingerprint {
        dev: 0,
        ino: 0,
        mtime_sec: system_time_secs(modified),
        mtime_nsec: system_time_nanos(modified),
        ctime_sec: system_time_secs(changed),
        ctime_nsec: system_time_nanos(changed),
        len: metadata.len(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeleteSourceFingerprint {
    dev: u64,
    ino: u64,
    mtime_sec: i64,
    mtime_nsec: i64,
    ctime_sec: i64,
    ctime_nsec: i64,
    len: u64,
}

#[cfg(not(unix))]
fn system_time_secs(value: std::time::SystemTime) -> i64 {
    value
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(not(unix))]
fn system_time_nanos(value: std::time::SystemTime) -> i64 {
    value
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::from(duration.subsec_nanos()))
        .unwrap_or(0)
}

pub(crate) fn is_monthly_history_file(name: &str) -> bool {
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

    mod delete {
        use std::fs;
        use std::io::Write;
        use std::path::{Path, PathBuf};
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };

        use time::macros::{datetime, offset};

        use crate::history::stats::tests_support::{record, TestHooks};
        use crate::history::{
            store::path_for_month_in_dir, HistoryEvent, HistoryQuery, HistoryService,
            HistoryStatsStatus,
        };

        #[test]
        fn audio_only_delete_preserves_history() {
            let history_dir = temp_history_dir("audio-only");
            let id = ulid::Ulid::new().to_string();
            write_line(
                &history_dir,
                record(&id, datetime!(2026-06-01 00:00:00 UTC), "one"),
            );
            let audio = write_audio(&history_dir, &id, "flac");
            let before = fs::read_to_string(history_dir.join("2026-06.jsonl")).unwrap();
            let service = HistoryService::with_test_hooks(
                history_dir.clone(),
                offset!(+0),
                TestHooks::default(),
            );

            let result = service.delete_audio(&id).unwrap();

            assert_eq!(result.id, id);
            assert!(result.deleted);
            assert!(!audio.exists());
            assert_eq!(
                fs::read_to_string(history_dir.join("2026-06.jsonl")).unwrap(),
                before
            );
            assert_eq!(service.stats().total.records, 1);
            let _ = fs::remove_dir_all(state_dir_for_history(&history_dir));
        }

        #[test]
        fn history_delete_removes_record_and_audio() {
            let history_dir = temp_history_dir("history-delete");
            let delete_id = ulid::Ulid::new().to_string();
            let keep_id = ulid::Ulid::new().to_string();
            write_line(
                &history_dir,
                record(&delete_id, datetime!(2026-06-01 00:00:00 UTC), "delete"),
            );
            write_line(
                &history_dir,
                record(&keep_id, datetime!(2026-06-01 00:00:01 UTC), "keep"),
            );
            let audio = write_audio(&history_dir, &delete_id, "m4a");
            let service = HistoryService::with_test_hooks(
                history_dir.clone(),
                offset!(+0),
                TestHooks::default(),
            );
            assert_eq!(service.stats().total.records, 2);
            let mut rx = service.subscribe();

            let result = service.delete(&delete_id).unwrap();
            let records = service.page(HistoryQuery::default()).unwrap();

            assert_eq!(result.id, delete_id);
            assert!(result.record_deleted);
            assert!(result.audio_deleted);
            assert!(result.audio_error.is_none());
            assert!(!audio.exists());
            assert_ids(&records, &[&keep_id]);
            assert_eq!(service.stats().total.records, 1);
            assert_eq!(rx.try_recv().unwrap(), HistoryEvent::Changed);
            let _ = fs::remove_dir_all(state_dir_for_history(&history_dir));
        }

        #[test]
        fn missing_history_still_deletes_orphan_audio() {
            let history_dir = temp_history_dir("orphan-audio");
            let id = ulid::Ulid::new().to_string();
            let audio = write_audio(&history_dir, &id, "flac");
            let service = HistoryService::with_test_hooks(
                history_dir.clone(),
                offset!(+0),
                TestHooks::default(),
            );

            let result = service.delete(&id).unwrap();

            assert_eq!(result.id, id);
            assert!(!result.record_deleted);
            assert!(result.audio_deleted);
            assert!(result.audio_error.is_none());
            assert!(!audio.exists());
            let _ = fs::remove_dir_all(state_dir_for_history(&history_dir));
        }

        #[test]
        fn source_change_before_rename_aborts() {
            let history_dir = temp_history_dir("source-change");
            let id = ulid::Ulid::new().to_string();
            write_line(
                &history_dir,
                record(&id, datetime!(2026-06-01 00:00:00 UTC), "one"),
            );
            let path = history_dir.join("2026-06.jsonl");
            let audio = write_audio(&history_dir, &id, "flac");
            let changed = Arc::new(AtomicBool::new(false));
            let hooks = TestHooks::default().with_before_history_delete_rename({
                let path = path.clone();
                let changed = Arc::clone(&changed);
                move || {
                    if !changed.swap(true, Ordering::SeqCst) {
                        fs::OpenOptions::new()
                            .append(true)
                            .open(&path)
                            .unwrap()
                            .write_all(b"\n")
                            .unwrap();
                    }
                }
            });
            let service = HistoryService::with_test_hooks(history_dir.clone(), offset!(+0), hooks);

            let error = service.delete(&id).unwrap_err();

            assert!(error.to_string().contains("changed"), "{error:#}");
            assert!(audio.exists());
            let records = service.page(HistoryQuery::default()).unwrap();
            assert_ids(&records, &[&id]);
            assert!(tmp_files(&history_dir).is_empty());
            let _ = fs::remove_dir_all(state_dir_for_history(&history_dir));
        }

        #[test]
        fn audio_failure_after_record_delete_reports_partial_success() {
            let history_dir = temp_history_dir("partial");
            let id = ulid::Ulid::new().to_string();
            write_line(
                &history_dir,
                record(&id, datetime!(2026-06-01 00:00:00 UTC), "one"),
            );
            let audio = write_audio(&history_dir, &id, "flac");
            let changed = Arc::new(AtomicBool::new(false));
            let hooks = TestHooks::default().with_before_history_delete_rename({
                let audio = audio.clone();
                let changed = Arc::clone(&changed);
                move || {
                    if !changed.swap(true, Ordering::SeqCst) {
                        fs::remove_file(&audio).unwrap();
                        fs::create_dir(&audio).unwrap();
                    }
                }
            });
            let service = HistoryService::with_test_hooks(history_dir.clone(), offset!(+0), hooks);

            let parent = audio.parent().unwrap();
            let result = service.delete(&id).unwrap();

            assert!(result.record_deleted);
            assert!(!result.audio_deleted);
            assert!(result
                .audio_error
                .as_deref()
                .is_some_and(|error| error.contains("regular file")));
            assert!(audio.is_dir());
            assert!(service.page(HistoryQuery::default()).unwrap().is_empty());
            assert_eq!(service.stats().total.records, 0);
            fs::remove_dir(&audio).unwrap();
            assert!(parent.exists());
            let _ = fs::remove_dir_all(state_dir_for_history(&history_dir));
        }

        #[test]
        fn history_delete_rejects_audio_conflict_without_deleting_record() {
            let history_dir = temp_history_dir("audio-conflict-reject");
            let id = ulid::Ulid::new().to_string();
            write_line(
                &history_dir,
                record(&id, datetime!(2026-06-01 00:00:00 UTC), "one"),
            );
            let flac = write_audio(&history_dir, &id, "flac");
            let m4a = write_audio(&history_dir, &id, "m4a");
            let service = HistoryService::with_test_hooks(
                history_dir.clone(),
                offset!(+0),
                TestHooks::default(),
            );

            let error = service.delete(&id).unwrap_err();

            assert!(error.to_string().contains("conflict"), "{error:#}");
            assert!(flac.exists());
            assert!(m4a.exists());
            assert_history_ids(&service, &[&id]);
            let _ = fs::remove_dir_all(state_dir_for_history(&history_dir));
        }

        #[cfg(unix)]
        #[test]
        fn history_delete_rejects_audio_symlink_without_deleting_record() {
            use std::os::unix::fs::symlink;

            let history_dir = temp_history_dir("audio-symlink-reject");
            let id = ulid::Ulid::new().to_string();
            write_line(
                &history_dir,
                record(&id, datetime!(2026-06-01 00:00:00 UTC), "one"),
            );
            let audio_dir = audio_dir_for_history(&history_dir);
            fs::create_dir_all(&audio_dir).unwrap();
            let target = audio_dir.join("target.flac");
            let link = audio_dir.join(format!("{id}.flac"));
            fs::write(&target, [1]).unwrap();
            symlink(&target, &link).unwrap();
            let service = HistoryService::with_test_hooks(
                history_dir.clone(),
                offset!(+0),
                TestHooks::default(),
            );

            let error = service.delete(&id).unwrap_err();

            assert!(error.to_string().contains("symlink"), "{error:#}");
            assert!(link.exists());
            assert_history_ids(&service, &[&id]);
            let _ = fs::remove_dir_all(state_dir_for_history(&history_dir));
        }

        #[test]
        fn history_delete_rejects_non_regular_audio_without_deleting_record() {
            let history_dir = temp_history_dir("audio-directory-reject");
            let id = ulid::Ulid::new().to_string();
            write_line(
                &history_dir,
                record(&id, datetime!(2026-06-01 00:00:00 UTC), "one"),
            );
            let audio_dir = audio_dir_for_history(&history_dir);
            fs::create_dir_all(&audio_dir).unwrap();
            let audio = audio_dir.join(format!("{id}.flac"));
            fs::create_dir(&audio).unwrap();
            let service = HistoryService::with_test_hooks(
                history_dir.clone(),
                offset!(+0),
                TestHooks::default(),
            );

            let error = service.delete(&id).unwrap_err();

            assert!(error.to_string().contains("regular file"), "{error:#}");
            assert!(audio.is_dir());
            assert_history_ids(&service, &[&id]);
            let _ = fs::remove_dir_all(state_dir_for_history(&history_dir));
        }

        #[test]
        fn repeated_delete_is_idempotent() {
            let history_dir = temp_history_dir("idempotent");
            let id = ulid::Ulid::new().to_string();
            write_line(
                &history_dir,
                record(&id, datetime!(2026-06-01 00:00:00 UTC), "one"),
            );
            write_audio(&history_dir, &id, "flac");
            let service = HistoryService::with_test_hooks(
                history_dir.clone(),
                offset!(+0),
                TestHooks::default(),
            );

            let first = service.delete(&id).unwrap();
            let second = service.delete(&id).unwrap();
            let audio_first = service.delete_audio(&id).unwrap();

            assert!(first.record_deleted);
            assert!(first.audio_deleted);
            assert!(!second.record_deleted);
            assert!(!second.audio_deleted);
            assert!(second.audio_error.is_none());
            assert!(!audio_first.deleted);
            assert_eq!(service.stats().status, HistoryStatsStatus::Ready);
            assert_eq!(service.stats().total.records, 0);
            assert!(tmp_files(&history_dir).is_empty());
            let _ = fs::remove_dir_all(state_dir_for_history(&history_dir));
        }

        fn temp_history_dir(name: &str) -> PathBuf {
            std::env::temp_dir()
                .join(format!(
                    "shuohua-history-delete-{name}-{}",
                    ulid::Ulid::new()
                ))
                .join("history")
        }

        fn state_dir_for_history(history_dir: &Path) -> PathBuf {
            history_dir.parent().unwrap().to_path_buf()
        }

        fn audio_dir_for_history(history_dir: &Path) -> PathBuf {
            state_dir_for_history(history_dir).join("audio")
        }

        fn write_line(dir: &Path, record: crate::history::HistoryRecord) {
            let path = path_for_month_in_dir(dir, record.started_at);
            crate::history::store::append_record(&path, &record).unwrap();
        }

        fn write_audio(history_dir: &Path, id: &str, ext: &str) -> PathBuf {
            let audio_dir = audio_dir_for_history(history_dir);
            fs::create_dir_all(&audio_dir).unwrap();
            let path = audio_dir.join(format!("{id}.{ext}"));
            fs::write(&path, [1, 2, 3]).unwrap();
            path
        }

        fn assert_ids(records: &[crate::history::HistoryRecord], expected: &[&str]) {
            let ids: Vec<_> = records.iter().map(|record| record.id.as_str()).collect();
            assert_eq!(ids, expected);
        }

        fn assert_history_ids(service: &HistoryService, expected: &[&str]) {
            let records = service.page(HistoryQuery::default()).unwrap();
            assert_ids(&records, expected);
        }

        fn tmp_files(history_dir: &Path) -> Vec<PathBuf> {
            match fs::read_dir(history_dir) {
                Ok(entries) => entries
                    .map(|entry| entry.unwrap().path())
                    .filter(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.contains(".tmp-delete-"))
                    })
                    .collect(),
                Err(_) => Vec::new(),
            }
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
