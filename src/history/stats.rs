use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use time::{Date, Month, OffsetDateTime, UtcOffset};
use tokio::sync::broadcast;

use crate::history::{store, HistoryRecord};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateStats {
    pub records: u64,
    pub words: u64,
    pub duration_ms: u64,
    pub asr_audio_ms: u64,
}

impl AggregateStats {
    fn add_record(&mut self, record: &HistoryRecord) {
        self.records = self.records.saturating_add(1);
        self.words = self
            .words
            .saturating_add(u64::try_from(record.text_stats.words).unwrap_or(u64::MAX));
        self.duration_ms = self.duration_ms.saturating_add(record.duration_ms);
        self.asr_audio_ms = self.asr_audio_ms.saturating_add(record.asr.audio_ms);
    }

    fn add_stats(&mut self, other: AggregateStats) {
        self.records = self.records.saturating_add(other.records);
        self.words = self.words.saturating_add(other.words);
        self.duration_ms = self.duration_ms.saturating_add(other.duration_ms);
        self.asr_audio_ms = self.asr_audio_ms.saturating_add(other.asr_audio_ms);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStatsStatus {
    Ready,
    Stale,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryStatsSnapshot {
    pub status: HistoryStatsStatus,
    pub total: AggregateStats,
    pub current_month: AggregateStats,
    pub today: AggregateStats,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalyticsPeriod {
    Year,
    Month,
    Day,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyticsQuery {
    pub period: AnalyticsPeriod,
    pub anchor: String,
}

impl AnalyticsQuery {
    pub fn new(period: AnalyticsPeriod, anchor: impl Into<String>) -> Self {
        Self {
            period,
            anchor: anchor.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyticsPoint {
    pub key: String,
    pub stats: AggregateStats,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyticsSnapshot {
    pub status: HistoryStatsStatus,
    pub period: AnalyticsPeriod,
    pub anchor: String,
    pub points: Vec<AnalyticsPoint>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryEvent {
    Appended,
    Changed,
}

#[derive(Clone)]
pub struct HistoryService {
    inner: Arc<Mutex<ServiceInner>>,
    events: broadcast::Sender<HistoryEvent>,
}

struct ServiceInner {
    dir: PathBuf,
    local_offset: UtcOffset,
    state: IndexState,
    failed: Option<FailedScan>,
    hooks: Hooks,
}

enum IndexState {
    Uninitialized,
    Ready(Index),
    Stale { last_valid: Index, error: String },
    Unavailable { error: String },
}

struct FailedScan {
    fingerprint: FileSetFingerprint,
}

#[derive(Clone)]
struct Index {
    offset: UtcOffset,
    total: AggregateStats,
    hourly: BTreeMap<LocalHour, AggregateStats>,
    fingerprints: FileSetFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSetFingerprint(Vec<FileFingerprintEntry>);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FileFingerprintEntry {
    name: String,
    fingerprint: FileFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FileFingerprint {
    dev: u64,
    ino: u64,
    mtime_sec: i64,
    mtime_nsec: i64,
    ctime_sec: i64,
    ctime_nsec: i64,
    len: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct LocalHour {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
}

#[derive(Clone, Default)]
struct Hooks {
    before_list: Option<Arc<dyn Fn() + Send + Sync>>,
    before_scan_attempt: Option<Arc<dyn Fn() + Send + Sync>>,
    after_read_file: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Hooks {
    fn before_list(&self) {
        if let Some(hook) = &self.before_list {
            hook();
        }
    }

    fn before_scan_attempt(&self) {
        if let Some(hook) = &self.before_scan_attempt {
            hook();
        }
    }

    fn after_read_file(&self) {
        if let Some(hook) = &self.after_read_file {
            hook();
        }
    }
}

impl HistoryService {
    pub fn new() -> Self {
        Self::with_dir(store::history_dir())
    }

    pub fn with_dir(dir: PathBuf) -> Self {
        Self::from_parts(dir, current_local_offset(), Hooks::default())
    }

    fn from_parts(dir: PathBuf, local_offset: UtcOffset, hooks: Hooks) -> Self {
        let (events, _rx) = broadcast::channel(128);
        Self {
            inner: Arc::new(Mutex::new(ServiceInner {
                dir,
                local_offset,
                state: IndexState::Uninitialized,
                failed: None,
                hooks,
            })),
            events,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_hooks(
        dir: PathBuf,
        local_offset: UtcOffset,
        hooks: tests_support::TestHooks,
    ) -> Self {
        Self::from_parts(dir, local_offset, hooks.into_hooks())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<HistoryEvent> {
        self.events.subscribe()
    }

    pub fn stats(&self) -> HistoryStatsSnapshot {
        let mut inner = self.inner.lock().expect("history service lock poisoned");
        let offset = inner.local_offset;
        match ensure_index(&mut inner) {
            Ok(IndexView::Ready(index)) => {
                stats_snapshot(index, offset, HistoryStatsStatus::Ready, None)
            }
            Ok(IndexView::Stale { index, error }) => {
                stats_snapshot(index, offset, HistoryStatsStatus::Stale, Some(error))
            }
            Ok(IndexView::Unavailable { error }) => HistoryStatsSnapshot {
                status: HistoryStatsStatus::Unavailable,
                total: AggregateStats::default(),
                current_month: AggregateStats::default(),
                today: AggregateStats::default(),
                error: Some(error),
            },
            Err(error) => HistoryStatsSnapshot {
                status: HistoryStatsStatus::Unavailable,
                total: AggregateStats::default(),
                current_month: AggregateStats::default(),
                today: AggregateStats::default(),
                error: Some(error.to_string()),
            },
        }
    }

    pub fn analytics(&self, query: AnalyticsQuery) -> Result<AnalyticsSnapshot> {
        let mut inner = self.inner.lock().expect("history service lock poisoned");
        let offset = inner.local_offset;
        let view = ensure_index(&mut inner)?;
        match view {
            IndexView::Ready(index) => {
                analytics_snapshot(index, query, offset, HistoryStatsStatus::Ready, None)
            }
            IndexView::Stale { index, error } => {
                analytics_snapshot(index, query, offset, HistoryStatsStatus::Stale, Some(error))
            }
            IndexView::Unavailable { error } => {
                let period = query.period;
                let anchor = query.anchor.clone();
                Ok(AnalyticsSnapshot {
                    status: HistoryStatsStatus::Unavailable,
                    period,
                    anchor,
                    points: empty_points(query)?,
                    error: Some(error),
                })
            }
        }
    }

    pub fn append(&self, record: HistoryRecord) -> Result<()> {
        let mut events = Vec::new();
        {
            let mut inner = self.inner.lock().expect("history service lock poisoned");
            reconcile_if_ready(&mut inner, &mut events)?;
            store::append_record(
                &store::path_for_month_in_dir(&inner.dir, record.started_at),
                &record,
            )?;
            let new_fingerprints = if matches!(inner.state, IndexState::Ready(_)) {
                Some(fingerprint_file_set(&inner.dir, &inner.hooks)?)
            } else {
                None
            };
            if let (IndexState::Ready(index), Some(new_fingerprints)) =
                (&mut inner.state, new_fingerprints)
            {
                add_record_to_index(index, &record);
                index.fingerprints = new_fingerprints;
            }
            events.push(HistoryEvent::Appended);
        }
        for event in events {
            let _ = self.events.send(event);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_test_local_offset(&self, offset: UtcOffset) {
        let mut inner = self.inner.lock().expect("history service lock poisoned");
        if inner.local_offset != offset {
            inner.local_offset = offset;
            inner.state = IndexState::Uninitialized;
            inner.failed = None;
        }
    }
}

impl Default for HistoryService {
    fn default() -> Self {
        Self::new()
    }
}

enum IndexView<'a> {
    Ready(&'a Index),
    Stale { index: &'a Index, error: String },
    Unavailable { error: String },
}

fn ensure_index(inner: &mut ServiceInner) -> Result<IndexView<'_>> {
    if let IndexState::Ready(index)
    | IndexState::Stale {
        last_valid: index, ..
    } = &inner.state
    {
        if index.offset != inner.local_offset {
            inner.state = IndexState::Uninitialized;
            inner.failed = None;
        }
    }

    if matches!(inner.state, IndexState::Uninitialized) {
        apply_scan_outcome(
            inner,
            scan_stable(&inner.dir, inner.local_offset, &inner.hooks)?,
        );
    } else if let Some(failed) = &inner.failed {
        let current = fingerprint_file_set(&inner.dir, &inner.hooks)?;
        if current != failed.fingerprint {
            apply_scan_outcome(
                inner,
                scan_stable(&inner.dir, inner.local_offset, &inner.hooks)?,
            );
        }
    } else if let IndexState::Ready(index)
    | IndexState::Stale {
        last_valid: index, ..
    } = &inner.state
    {
        let current = fingerprint_file_set(&inner.dir, &inner.hooks)?;
        if current != index.fingerprints {
            apply_scan_outcome(
                inner,
                scan_stable(&inner.dir, inner.local_offset, &inner.hooks)?,
            );
        }
    }

    Ok(match &inner.state {
        IndexState::Ready(index) => IndexView::Ready(index),
        IndexState::Stale { last_valid, error } => IndexView::Stale {
            index: last_valid,
            error: error.clone(),
        },
        IndexState::Unavailable { error } => IndexView::Unavailable {
            error: error.clone(),
        },
        IndexState::Uninitialized => IndexView::Unavailable {
            error: "history index is uninitialized".to_string(),
        },
    })
}

fn apply_scan_outcome(inner: &mut ServiceInner, outcome: ScanOutcome) {
    match outcome {
        ScanOutcome::Ready(index) => {
            inner.failed = None;
            inner.state = IndexState::Ready(index);
        }
        ScanOutcome::Failed { fingerprint, error } => {
            inner.failed = Some(FailedScan { fingerprint });
            inner.state = match std::mem::replace(
                &mut inner.state,
                IndexState::Unavailable {
                    error: error.clone(),
                },
            ) {
                IndexState::Ready(last_valid) | IndexState::Stale { last_valid, .. } => {
                    IndexState::Stale { last_valid, error }
                }
                IndexState::Uninitialized | IndexState::Unavailable { .. } => {
                    IndexState::Unavailable { error }
                }
            };
        }
    }
}

fn reconcile_if_ready(inner: &mut ServiceInner, events: &mut Vec<HistoryEvent>) -> Result<()> {
    let IndexState::Ready(index) = &inner.state else {
        return Ok(());
    };
    let current = fingerprint_file_set(&inner.dir, &inner.hooks)?;
    if current == index.fingerprints {
        return Ok(());
    }
    let outcome = scan_stable(&inner.dir, inner.local_offset, &inner.hooks)?;
    if matches!(outcome, ScanOutcome::Ready(_)) {
        events.push(HistoryEvent::Changed);
    }
    apply_scan_outcome(inner, outcome);
    Ok(())
}

enum ScanOutcome {
    Ready(Index),
    Failed {
        fingerprint: FileSetFingerprint,
        error: String,
    },
}

fn scan_stable(dir: &Path, offset: UtcOffset, hooks: &Hooks) -> Result<ScanOutcome> {
    let mut last_fingerprint = FileSetFingerprint(Vec::new());
    let mut last_error = None;
    for attempt in 0..2 {
        hooks.before_scan_attempt();
        let before = fingerprint_file_set(dir, hooks)?;
        last_fingerprint = before.clone();
        match scan_once(dir, offset, &before, hooks) {
            Ok(index) => {
                let after = fingerprint_file_set(dir, hooks)?;
                if before == after {
                    return Ok(ScanOutcome::Ready(index));
                }
                last_fingerprint = after;
                last_error = Some("history changed while scanning".to_string());
            }
            Err(error) => {
                last_error = Some(error.to_string());
            }
        }
        if attempt == 1 {
            break;
        }
    }
    Ok(ScanOutcome::Failed {
        fingerprint: last_fingerprint,
        error: last_error.unwrap_or_else(|| "history could not be scanned stably".to_string()),
    })
}

fn scan_once(
    dir: &Path,
    offset: UtcOffset,
    before: &FileSetFingerprint,
    hooks: &Hooks,
) -> Result<Index> {
    let mut index = Index {
        offset,
        total: AggregateStats::default(),
        hourly: BTreeMap::new(),
        fingerprints: before.clone(),
    };
    for entry in &before.0 {
        let path = dir.join(&entry.name);
        let body = fs::read_to_string(&path)
            .with_context(|| format!("read history {}", path.display()))?;
        hooks.after_read_file();
        let has_complete_final_line = body.ends_with('\n');
        let mut lines = body.split('\n').peekable();
        let mut previous_key: Option<(OffsetDateTime, String)> = None;
        let mut line_no = 0usize;
        while let Some(line) = lines.next() {
            line_no += 1;
            if line.trim().is_empty() {
                continue;
            }
            let record: HistoryRecord = match serde_json::from_str(line) {
                Ok(record) => record,
                Err(error) if lines.peek().is_none() && !has_complete_final_line => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %error,
                        "ignoring truncated final history line"
                    );
                    break;
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("parse history line in {}", path.display()));
                }
            };
            validate_record(&record, &path, line_no)?;
            let key = (record.started_at, record.id.clone());
            if previous_key
                .as_ref()
                .is_some_and(|previous| previous > &key)
            {
                bail!(
                    "history records are not monotonic by (started_at,id) at {}:{}",
                    path.display(),
                    line_no
                );
            }
            previous_key = Some(key);
            add_record_to_index(&mut index, &record);
        }
    }
    Ok(index)
}

fn validate_record(record: &HistoryRecord, path: &Path, line_no: usize) -> Result<()> {
    if record.version != 1 {
        bail!(
            "unsupported history schema version {} at {}:{}",
            record.version,
            path.display(),
            line_no
        );
    }
    Ok(())
}

fn add_record_to_index(index: &mut Index, record: &HistoryRecord) {
    index.total.add_record(record);
    let hour = local_hour(record.started_at, index.offset);
    index.hourly.entry(hour).or_default().add_record(record);
}

fn stats_snapshot(
    index: &Index,
    offset: UtcOffset,
    status: HistoryStatsStatus,
    error: Option<String>,
) -> HistoryStatsSnapshot {
    let now = OffsetDateTime::now_utc().to_offset(offset);
    let mut current_month = AggregateStats::default();
    let mut today = AggregateStats::default();
    for (hour, stats) in &index.hourly {
        if hour.year == now.year() && hour.month == u8::from(now.month()) {
            current_month.add_stats(*stats);
            if hour.day == now.day() {
                today.add_stats(*stats);
            }
        }
    }
    HistoryStatsSnapshot {
        status,
        total: index.total,
        current_month,
        today,
        error,
    }
}

fn analytics_snapshot(
    index: &Index,
    query: AnalyticsQuery,
    offset: UtcOffset,
    status: HistoryStatsStatus,
    error: Option<String>,
) -> Result<AnalyticsSnapshot> {
    let period = query.period;
    let anchor = query.anchor.clone();
    Ok(AnalyticsSnapshot {
        status,
        period,
        anchor,
        points: analytics_points(index, query, offset)?,
        error,
    })
}

fn local_hour(value: OffsetDateTime, offset: UtcOffset) -> LocalHour {
    let local = value.to_offset(offset);
    LocalHour {
        year: local.year(),
        month: u8::from(local.month()),
        day: local.day(),
        hour: local.hour(),
    }
}

fn analytics_points(
    index: &Index,
    query: AnalyticsQuery,
    _offset: UtcOffset,
) -> Result<Vec<AnalyticsPoint>> {
    match query.period {
        AnalyticsPeriod::Day => {
            let date = parse_anchor_date(&query.anchor)?;
            let mut points = Vec::with_capacity(24);
            for hour in 0..24 {
                let key = LocalHour {
                    year: date.year(),
                    month: u8::from(date.month()),
                    day: date.day(),
                    hour,
                };
                points.push(AnalyticsPoint {
                    key: format!("{hour:02}"),
                    stats: index.hourly.get(&key).copied().unwrap_or_default(),
                });
            }
            Ok(points)
        }
        AnalyticsPeriod::Month => {
            let (year, month) = parse_anchor_month(&query.anchor)?;
            let days = days_in_month(year, month);
            let mut points = Vec::with_capacity(usize::from(days));
            for day in 1..=days {
                let mut stats = AggregateStats::default();
                for hour in 0..24 {
                    let key = LocalHour {
                        year,
                        month: u8::from(month),
                        day,
                        hour,
                    };
                    stats.add_stats(index.hourly.get(&key).copied().unwrap_or_default());
                }
                points.push(AnalyticsPoint {
                    key: format!("{day:02}"),
                    stats,
                });
            }
            Ok(points)
        }
        AnalyticsPeriod::Year => {
            let year = parse_anchor_year(&query.anchor)?;
            let mut points = Vec::with_capacity(12);
            for month_number in 1..=12 {
                let month = Month::try_from(month_number)
                    .map_err(|_| anyhow!("invalid month {month_number}"))?;
                let mut stats = AggregateStats::default();
                for day in 1..=days_in_month(year, month) {
                    for hour in 0..24 {
                        let key = LocalHour {
                            year,
                            month: month_number,
                            day,
                            hour,
                        };
                        stats.add_stats(index.hourly.get(&key).copied().unwrap_or_default());
                    }
                }
                points.push(AnalyticsPoint {
                    key: format!("{month_number:02}"),
                    stats,
                });
            }
            Ok(points)
        }
    }
}

fn empty_points(query: AnalyticsQuery) -> Result<Vec<AnalyticsPoint>> {
    let empty = Index {
        offset: UtcOffset::UTC,
        total: AggregateStats::default(),
        hourly: BTreeMap::new(),
        fingerprints: FileSetFingerprint(Vec::new()),
    };
    analytics_points(&empty, query, UtcOffset::UTC)
}

fn parse_anchor_year(anchor: &str) -> Result<i32> {
    if anchor.len() != 4 || !anchor.bytes().all(|b| b.is_ascii_digit()) {
        bail!("invalid year analytics anchor: {anchor}");
    }
    anchor.parse().context("parse year analytics anchor")
}

fn parse_anchor_month(anchor: &str) -> Result<(i32, Month)> {
    let Some((year, month)) = anchor.split_once('-') else {
        bail!("invalid month analytics anchor: {anchor}");
    };
    let year = parse_anchor_year(year)?;
    let month: u8 = month.parse().context("parse month analytics anchor")?;
    let month = Month::try_from(month).context("validate month analytics anchor")?;
    Ok((year, month))
}

fn parse_anchor_date(anchor: &str) -> Result<Date> {
    let mut parts = anchor.split('-');
    let year = parts
        .next()
        .ok_or_else(|| anyhow!("invalid day analytics anchor: {anchor}"))
        .and_then(parse_anchor_year)?;
    let month: u8 = parts
        .next()
        .ok_or_else(|| anyhow!("invalid day analytics anchor: {anchor}"))?
        .parse()
        .context("parse day analytics month")?;
    let day: u8 = parts
        .next()
        .ok_or_else(|| anyhow!("invalid day analytics anchor: {anchor}"))?
        .parse()
        .context("parse day analytics day")?;
    if parts.next().is_some() {
        bail!("invalid day analytics anchor: {anchor}");
    }
    Date::from_calendar_date(year, Month::try_from(month)?, day)
        .context("validate day analytics anchor")
}

fn days_in_month(year: i32, month: Month) -> u8 {
    let first = Date::from_calendar_date(year, month, 1).expect("valid month");
    let next = if month == Month::December {
        Date::from_calendar_date(year + 1, Month::January, 1).expect("valid next month")
    } else {
        Date::from_calendar_date(year, Month::try_from(u8::from(month) + 1).unwrap(), 1)
            .expect("valid next month")
    };
    u8::try_from((next - first).whole_days()).unwrap_or(31)
}

fn fingerprint_file_set(dir: &Path, hooks: &Hooks) -> Result<FileSetFingerprint> {
    hooks.before_list();
    let mut files = store::monthly_history_files_in_dir(dir)?;
    files.sort();
    let mut entries = Vec::with_capacity(files.len());
    for path in files {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("invalid history filename {}", path.display()))?
            .to_string();
        entries.push(FileFingerprintEntry {
            name,
            fingerprint: fingerprint_path(&path)?,
        });
    }
    Ok(FileSetFingerprint(entries))
}

#[cfg(unix)]
fn fingerprint_path(path: &Path) -> Result<FileFingerprint> {
    use std::os::unix::fs::MetadataExt;

    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("stat history {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("history file must not be a symlink: {}", path.display());
    }
    if !metadata.file_type().is_file() {
        bail!("history path must be a regular file: {}", path.display());
    }
    Ok(FileFingerprint {
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
fn fingerprint_path(path: &Path) -> Result<FileFingerprint> {
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
    Ok(FileFingerprint {
        dev: 0,
        ino: 0,
        mtime_sec: system_time_secs(modified),
        mtime_nsec: system_time_nanos(modified),
        ctime_sec: system_time_secs(changed),
        ctime_nsec: system_time_nanos(changed),
        len: metadata.len(),
    })
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

fn current_local_offset() -> UtcOffset {
    UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
}

#[cfg(test)]
pub(crate) fn fingerprint_path_for_test(path: &Path) -> Result<String> {
    Ok(format!("{:?}", fingerprint_path(path)?))
}

#[cfg(test)]
pub(crate) mod tests_support {
    use std::sync::Arc;

    use time::OffsetDateTime;

    use crate::history::{
        AsrHistory, AsrSessionHistory, HistoryRecord, HistoryStatus, PipelineStepHistory,
        PipelineStepStatus,
    };

    use super::Hooks;

    #[derive(Clone, Default)]
    pub struct TestHooks {
        hooks: Hooks,
    }

    impl TestHooks {
        pub fn with_before_list(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
            self.hooks.before_list = Some(Arc::new(hook));
            self
        }

        pub fn with_before_scan_attempt(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
            self.hooks.before_scan_attempt = Some(Arc::new(hook));
            self
        }

        pub fn with_after_read_file(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
            self.hooks.after_read_file = Some(Arc::new(hook));
            self
        }

        pub(super) fn into_hooks(self) -> Hooks {
            self.hooks
        }
    }

    pub fn record(id: &str, started_at: OffsetDateTime, text: &str) -> HistoryRecord {
        HistoryRecord {
            version: 1,
            id: id.to_string(),
            started_at,
            ended_at: started_at + time::Duration::seconds(1),
            duration_ms: 1000,
            status: HistoryStatus::Submitted,
            app: Some("com.example.App".to_string()),
            text: text.to_string(),
            text_stats: crate::text_stats::compute(text),
            asr: AsrHistory {
                provider: "test".to_string(),
                text: text.to_string(),
                duration_ms: 1000,
                audio_ms: 1000,
                sessions: vec![AsrSessionHistory {
                    text: text.to_string(),
                    started_at,
                    ended_at: started_at + time::Duration::seconds(1),
                    audio_ms: 1000,
                }],
            },
            pipeline: vec![PipelineStepHistory {
                name: "test".to_string(),
                status: PipelineStepStatus::Ok,
                duration_ms: 1.0,
                text: Some(text.to_string()),
                error: None,
            }],
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };

    use time::macros::{datetime, offset};

    use crate::history::{
        store::path_for_month_in_dir, AnalyticsPeriod, AnalyticsQuery, HistoryService,
        HistoryStatsStatus,
    };

    use super::tests_support::{record, TestHooks};

    #[test]
    fn aggregates_all_statuses_into_local_hours() {
        let dir = temp_dir("aggregate-statuses");
        let mut records = vec![
            record("a", datetime!(2026-06-01 16:00:00 UTC), "hello world"),
            record("b", datetime!(2026-06-01 16:30:00 UTC), "cancel"),
            record("c", datetime!(2026-06-01 16:45:00 UTC), ""),
            record("d", datetime!(2026-06-01 16:50:00 UTC), "err"),
            record("e", datetime!(2026-06-01 16:55:00 UTC), "timeout"),
        ];
        records[1].status = crate::history::HistoryStatus::Canceled;
        records[2].status = crate::history::HistoryStatus::Empty;
        records[3].status = crate::history::HistoryStatus::Error;
        records[4].status = crate::history::HistoryStatus::Timeout;
        for record in records {
            write_line(&dir, record);
        }

        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+8), TestHooks::default());
        let snapshot = service.stats();
        let day = service
            .analytics(AnalyticsQuery::new(AnalyticsPeriod::Day, "2026-06-02"))
            .unwrap();

        assert_eq!(snapshot.status, HistoryStatsStatus::Ready);
        assert_eq!(snapshot.total.records, 5);
        assert_eq!(snapshot.total.duration_ms, 5000);
        assert_eq!(snapshot.total.asr_audio_ms, 5000);
        assert_eq!(day.points[0].key, "00");
        assert_eq!(day.points[0].stats.records, 5);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn year_month_day_fixed_buckets() {
        let dir = temp_dir("fixed-buckets");
        write_line(
            &dir,
            record("a", datetime!(2026-06-15 12:00:00 UTC), "hello"),
        );
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let day = service
            .analytics(AnalyticsQuery::new(AnalyticsPeriod::Day, "2026-06-15"))
            .unwrap();
        let month = service
            .analytics(AnalyticsQuery::new(AnalyticsPeriod::Month, "2026-06"))
            .unwrap();
        let year = service
            .analytics(AnalyticsQuery::new(AnalyticsPeriod::Year, "2026"))
            .unwrap();

        assert_eq!(day.points.len(), 24);
        assert_eq!(month.points.len(), 30);
        assert_eq!(year.points.len(), 12);
        assert_eq!(day.points[12].stats.records, 1);
        assert_eq!(month.points[14].stats.records, 1);
        assert_eq!(year.points[5].stats.records, 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn scan_ignores_truncated_final_line() {
        let dir = temp_dir("truncated-final");
        write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
        fs::OpenOptions::new()
            .append(true)
            .open(dir.join("2026-06.jsonl"))
            .unwrap()
            .write_all(b"{\"version\":1")
            .unwrap();
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let snapshot = service.stats();

        assert_eq!(snapshot.status, HistoryStatsStatus::Ready);
        assert_eq!(snapshot.total.records, 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn scan_retries_when_fingerprint_changes_during_read() {
        let dir = temp_dir("retry-fingerprint");
        write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
        let mutated = Arc::new(AtomicBool::new(false));
        let attempts = Arc::new(AtomicUsize::new(0));
        let hooks = TestHooks::default()
            .with_before_scan_attempt({
                let attempts = Arc::clone(&attempts);
                move || {
                    attempts.fetch_add(1, Ordering::SeqCst);
                }
            })
            .with_after_read_file({
                let dir = dir.clone();
                let mutated = Arc::clone(&mutated);
                move || {
                    if !mutated.swap(true, Ordering::SeqCst) {
                        write_line(&dir, record("b", datetime!(2026-06-01 01:00:00 UTC), "two"));
                    }
                }
            });
        let service = HistoryService::with_test_hooks(dir.clone(), offset!(+0), hooks);

        let snapshot = service.stats();

        assert_eq!(snapshot.status, HistoryStatsStatus::Ready);
        assert_eq!(snapshot.total.records, 2);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn same_size_rewrite_changes_unix_fingerprint() {
        let dir = temp_dir("same-size-fingerprint");
        let path = dir.join("2026-06.jsonl");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, "aaaaaaaaaa\n").unwrap();
        set_mtime(&path, 1, 0);
        let before = crate::history::stats::fingerprint_path_for_test(&path).unwrap();
        fs::write(&path, "bbbbbbbbbb\n").unwrap();
        set_mtime(&path, 2, 0);
        let after = crate::history::stats::fingerprint_path_for_test(&path).unwrap();

        assert_ne!(before, after);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn second_scan_change_marks_unavailable_without_partial_totals() {
        let dir = temp_dir("second-change-unavailable");
        write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
        let hooks = TestHooks::default().with_after_read_file({
            let dir = dir.clone();
            let next = Arc::new(AtomicUsize::new(0));
            move || {
                let id = next.fetch_add(1, Ordering::SeqCst);
                write_line(
                    &dir,
                    record(
                        &format!("changed-{id}"),
                        datetime!(2026-06-01 01:00:00 UTC),
                        "two",
                    ),
                );
            }
        });
        let service = HistoryService::with_test_hooks(dir.clone(), offset!(+0), hooks);

        let snapshot = service.stats();

        assert_eq!(snapshot.status, HistoryStatsStatus::Unavailable);
        assert_eq!(snapshot.total.records, 0);
        assert!(snapshot.error.is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_edit_keeps_last_valid_stats_and_marks_stale() {
        let dir = temp_dir("stale-last-valid");
        let path = dir.join("2026-06.jsonl");
        write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());
        assert_eq!(service.stats().total.records, 1);
        fs::write(&path, "not json\n").unwrap();

        let snapshot = service.stats();

        assert_eq!(snapshot.status, HistoryStatsStatus::Stale);
        assert_eq!(snapshot.total.records, 1);
        assert!(snapshot.error.is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unchanged_failed_fingerprint_is_not_rescanned() {
        let dir = temp_dir("stale-no-rescan");
        let path = dir.join("2026-06.jsonl");
        write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
        let scan_calls = Arc::new(AtomicUsize::new(0));
        let hooks = TestHooks::default().with_before_scan_attempt({
            let scan_calls = Arc::clone(&scan_calls);
            move || {
                scan_calls.fetch_add(1, Ordering::SeqCst);
            }
        });
        let service = HistoryService::with_test_hooks(dir.clone(), offset!(+0), hooks);
        assert_eq!(service.stats().status, HistoryStatsStatus::Ready);
        fs::write(&path, "not json\n").unwrap();

        assert_eq!(service.stats().status, HistoryStatsStatus::Stale);
        assert_eq!(service.stats().status, HistoryStatsStatus::Stale);

        assert_eq!(scan_calls.load(Ordering::SeqCst), 3);
        let _ = fs::remove_dir_all(dir);
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("shuohua-history-{name}-{}", ulid::Ulid::new()))
    }

    fn write_line(dir: &std::path::Path, record: crate::history::HistoryRecord) {
        let path = path_for_month_in_dir(dir, record.started_at);
        crate::history::store::append_record(&path, &record).unwrap();
    }

    #[cfg(unix)]
    fn set_mtime(path: &std::path::Path, seconds: i64, nanoseconds: i64) {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let times = [
            libc::timespec {
                tv_sec: seconds,
                tv_nsec: nanoseconds,
            },
            libc::timespec {
                tv_sec: seconds,
                tv_nsec: nanoseconds,
            },
        ];
        let rc = unsafe { libc::utimensat(libc::AT_FDCWD, path.as_ptr(), times.as_ptr(), 0) };
        assert_eq!(rc, 0, "utimensat failed");
    }

    #[cfg(not(unix))]
    fn set_mtime(_path: &std::path::Path, _seconds: i64, _nanoseconds: i64) {}
}
