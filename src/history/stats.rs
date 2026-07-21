use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use time::{Date, Month, OffsetDateTime, UtcOffset};
use tokio::sync::broadcast;

use crate::history::{
    assets, store, AudioAssetInfo, AudioDeleteResult, CleanupError, CleanupFilter, CleanupIssue,
    CleanupPreview, CleanupResult, CleanupScope, CleanupWarning, DeleteResult, HistoryQuery,
    HistoryRecord, DEFAULT_HISTORY_PAGE_LIMIT, MAX_HISTORY_PAGE_LIMIT,
};
use crate::trash::{system_trash, FileDeleter};

const REVERSE_READ_CHUNK_SIZE: usize = 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateStats {
    pub records: u64,
    pub words: u64,
    pub duration_ms: u64,
    pub asr_duration_ms: u64,
    pub asr_audio_ms: u64,
}

impl AggregateStats {
    fn add_record(&mut self, record: &HistoryRecord) {
        self.records = self.records.saturating_add(1);
        self.words = self
            .words
            .saturating_add(u64::try_from(record.text_stats().words).unwrap_or(u64::MAX));
        self.duration_ms = self.duration_ms.saturating_add(record.duration_ms);
        self.asr_duration_ms = self.asr_duration_ms.saturating_add(record.asr.duration_ms);
        self.asr_audio_ms = self.asr_audio_ms.saturating_add(record.asr.audio_ms);
    }

    fn add_stats(&mut self, other: AggregateStats) {
        self.records = self.records.saturating_add(other.records);
        self.words = self.words.saturating_add(other.words);
        self.duration_ms = self.duration_ms.saturating_add(other.duration_ms);
        self.asr_duration_ms = self.asr_duration_ms.saturating_add(other.asr_duration_ms);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalyticsPeriod {
    #[serde(rename = "last_7_days")]
    Last7Days,
    #[serde(rename = "last_30_days")]
    Last30Days,
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

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryPageResult {
    pub records: Vec<HistoryRecord>,
    pub matched: Option<u64>,
    pub stats: Option<AggregateStats>,
}

impl std::ops::Deref for HistoryPageResult {
    type Target = [HistoryRecord];

    fn deref(&self) -> &Self::Target {
        &self.records
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HistoryEvent {
    Appended(Box<HistoryRecord>),
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
    dirty: DirtySet,
    failed: Option<FailedScan>,
    observed: FileSetFingerprint,
    hooks: Hooks,
    /// 删除音频文件的策略：生产=移到系统废纸篓；测试=移到临时目录/直接删。
    deleter: FileDeleter,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum DirtySet {
    #[default]
    None,
    All,
    Months(BTreeSet<String>),
}

impl DirtySet {
    fn is_empty(&self) -> bool {
        matches!(self, Self::None)
    }

    fn mark_all(&mut self) {
        *self = Self::All;
    }

    fn mark_month(&mut self, month: String) {
        match self {
            Self::None => {
                let mut months = BTreeSet::new();
                months.insert(month);
                *self = Self::Months(months);
            }
            Self::All => {}
            Self::Months(months) => {
                months.insert(month);
            }
        }
    }

    fn clear(&mut self) {
        *self = Self::None;
    }

    #[cfg(test)]
    fn count(&self) -> usize {
        match self {
            Self::None => 0,
            Self::All => usize::MAX,
            Self::Months(months) => months.len(),
        }
    }
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
    before_history_delete_rename: Option<Arc<dyn Fn() + Send + Sync>>,
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

    fn before_history_delete_rename(&self) {
        if let Some(hook) = &self.before_history_delete_rename {
            hook();
        }
    }
}

impl HistoryService {
    pub fn new() -> Self {
        Self::with_dir(store::history_dir())
    }

    pub fn with_dir(dir: PathBuf) -> Self {
        Self::from_parts(
            dir,
            current_local_offset(),
            Hooks::default(),
            system_trash(),
        )
    }

    fn from_parts(
        dir: PathBuf,
        local_offset: UtcOffset,
        hooks: Hooks,
        deleter: FileDeleter,
    ) -> Self {
        let (events, _rx) = broadcast::channel(128);
        Self {
            inner: Arc::new(Mutex::new(ServiceInner {
                dir,
                local_offset,
                state: IndexState::Uninitialized,
                dirty: DirtySet::None,
                failed: None,
                observed: FileSetFingerprint(Vec::new()),
                hooks,
                deleter,
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
        // 测试构造器默认不碰真实废纸篓。
        Self::from_parts(
            dir,
            local_offset,
            hooks.into_hooks(),
            crate::trash::remove_deleter(),
        )
    }

    /// 注入删除策略（测试用：记录/临时目录，避免触碰真实 `~/.Trash`）。
    #[cfg(test)]
    pub(crate) fn with_deleter(self, deleter: FileDeleter) -> Self {
        self.inner
            .lock()
            .expect("history service lock poisoned")
            .deleter = deleter;
        self
    }

    pub fn subscribe(&self) -> broadcast::Receiver<HistoryEvent> {
        self.events.subscribe()
    }

    pub fn stats(&self) -> HistoryStatsSnapshot {
        let mut events = Vec::new();
        let snapshot = {
            let mut inner = self.inner.lock().expect("history service lock poisoned");
            let offset = inner.local_offset;
            match ensure_index(&mut inner, &mut events) {
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
        };
        self.publish(events);
        snapshot
    }

    pub fn analytics(&self, query: AnalyticsQuery) -> Result<AnalyticsSnapshot> {
        let mut events = Vec::new();
        let snapshot = {
            let mut inner = self.inner.lock().expect("history service lock poisoned");
            let offset = inner.local_offset;
            let view = ensure_index(&mut inner, &mut events)?;
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
        };
        self.publish(events);
        snapshot
    }

    pub fn page(&self, query: HistoryQuery) -> Result<HistoryPageResult> {
        let mut events = Vec::new();
        let page = {
            let mut inner = self.inner.lock().expect("history service lock poisoned");
            reconcile_if_initialized(&mut inner, &mut events)?;
            page_stable(&inner.dir, query, &inner.hooks)?
        };
        self.publish(events);
        Ok(page)
    }

    pub fn audio(&self, id: &str) -> Result<AudioAssetInfo> {
        let audio_dir = {
            let inner = self.inner.lock().expect("history service lock poisoned");
            assets::audio_dir_for_history_dir(&inner.dir)
        };
        assets::audio_info_in_dir(&audio_dir, id)
    }

    pub fn delete_audio(&self, id: &str) -> Result<AudioDeleteResult> {
        let (audio_dir, deleter) = {
            let inner = self.inner.lock().expect("history service lock poisoned");
            (
                assets::audio_dir_for_history_dir(&inner.dir),
                inner.deleter.clone(),
            )
        };
        assets::delete_audio_in_dir(&audio_dir, id, &deleter)
    }

    pub fn delete(&self, id: &str) -> Result<DeleteResult> {
        let mut events = Vec::new();
        let (record_deleted, audio_dir, deleter) = {
            let mut inner = self.inner.lock().expect("history service lock poisoned");
            reconcile_if_initialized(&mut inner, &mut events)?;
            let audio_dir = assets::audio_dir_for_history_dir(&inner.dir);
            assets::preflight_history_delete_audio_in_dir(&audio_dir, id)?;
            let hooks = inner.hooks.clone();
            let outcome =
                store::delete_record_in_dir(&inner.dir, id, inner.local_offset, move || {
                    hooks.before_history_delete_rename();
                })?;
            if outcome.deleted {
                let scan = scan_stable(&inner.dir, inner.local_offset, &inner.hooks)?;
                apply_scan_outcome(&mut inner, scan);
                events.push(HistoryEvent::Changed);
            }
            (outcome.deleted, audio_dir, inner.deleter.clone())
        };

        let audio_result = assets::delete_audio_in_dir(&audio_dir, id, &deleter);
        let (audio_deleted, audio_error) = match audio_result {
            Ok(result) => (result.deleted, None),
            Err(error) => (false, Some(error.to_string())),
        };
        self.publish(events);
        Ok(DeleteResult {
            id: id.to_string(),
            record_deleted,
            audio_deleted,
            audio_error,
        })
    }

    /// 计算 retained audio 批量清理预览：命中过滤条件、且音频可安全删除的 record ID
    /// 快照 + 总字节 + 时间范围 + 危险音频告警。不改 JSONL、不删任何文件。
    pub fn preview_cleanup(&self, filter: CleanupFilter) -> Result<CleanupPreview> {
        self.preview_cleanup_at(OffsetDateTime::now_utc(), filter)
    }

    fn preview_cleanup_at(
        &self,
        now: OffsetDateTime,
        filter: CleanupFilter,
    ) -> Result<CleanupPreview> {
        let (lower, upper) = filter.window.bounds(now);
        let mut events = Vec::new();
        let candidates = {
            let mut inner = self.inner.lock().expect("history service lock poisoned");
            reconcile_if_initialized(&mut inner, &mut events)?;
            let audio_dir = assets::audio_dir_for_history_dir(&inner.dir);
            cleanup_preview_stable(
                &inner.dir,
                &audio_dir,
                lower,
                upper,
                filter.scope,
                &inner.hooks,
            )?
        };
        self.publish(events);
        Ok(CleanupPreview {
            filter,
            ids: candidates.ids,
            audio_bytes: candidates.audio_bytes,
            audio_ms: candidates.audio_ms,
            oldest: candidates.oldest,
            newest: candidates.newest,
            warnings: candidates.warnings,
        })
    }

    /// 删除 preview 快照中的这批 ID。`AudioOnly` 只删音频，不改 JSONL；
    /// `RecordAndAudio` 批量重写 shards 删除记录，并删除成功删除记录的 linked audio。
    pub fn execute_cleanup(
        &self,
        filter: CleanupFilter,
        ids: Vec<String>,
    ) -> Result<CleanupResult> {
        match filter.scope {
            CleanupScope::AudioOnly => self.execute_audio_cleanup(ids),
            CleanupScope::RecordAndAudio => self.execute_record_cleanup(ids),
        }
    }

    fn execute_audio_cleanup(&self, ids: Vec<String>) -> Result<CleanupResult> {
        let (audio_dir, deleter) = {
            let inner = self.inner.lock().expect("history service lock poisoned");
            (
                assets::audio_dir_for_history_dir(&inner.dir),
                inner.deleter.clone(),
            )
        };
        let mut result = CleanupResult {
            requested: ids.len() as u64,
            deleted: 0,
            missing: 0,
            errors: Vec::new(),
        };
        for id in &ids {
            match assets::classify_cleanup_audio(&audio_dir, id) {
                Ok(assets::CleanupAudioClass::Missing) => result.missing += 1,
                Ok(assets::CleanupAudioClass::Unsafe(issue)) => result.errors.push(CleanupError {
                    id: id.clone(),
                    issue,
                }),
                Ok(assets::CleanupAudioClass::Present { path, .. }) => {
                    match deleter.delete(&path) {
                        Ok(()) => result.deleted += 1,
                        Err(error) => {
                            tracing::warn!(error = %error, "cleanup audio trash failed");
                            result.errors.push(CleanupError {
                                id: id.clone(),
                                issue: CleanupIssue::Io,
                            });
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(error = %error, "cleanup audio classify failed");
                    result.errors.push(CleanupError {
                        id: id.clone(),
                        issue: CleanupIssue::Io,
                    });
                }
            }
        }
        Ok(result)
    }

    fn execute_record_cleanup(&self, ids: Vec<String>) -> Result<CleanupResult> {
        let mut result = CleanupResult {
            requested: ids.len() as u64,
            deleted: 0,
            missing: 0,
            errors: Vec::new(),
        };
        let mut safe_ids = Vec::new();
        let mut audio_paths = BTreeMap::new();
        let mut events = Vec::new();
        let deleter;
        let audio_dir;
        {
            let mut inner = self.inner.lock().expect("history service lock poisoned");
            reconcile_if_initialized(&mut inner, &mut events)?;
            deleter = inner.deleter.clone();
            audio_dir = assets::audio_dir_for_history_dir(&inner.dir);
            for id in &ids {
                match assets::classify_cleanup_audio(&audio_dir, id) {
                    Ok(assets::CleanupAudioClass::Unsafe(issue)) => {
                        result.errors.push(CleanupError {
                            id: id.clone(),
                            issue,
                        });
                    }
                    Ok(assets::CleanupAudioClass::Missing) => {
                        safe_ids.push(id.clone());
                    }
                    Ok(assets::CleanupAudioClass::Present { path, identity, .. }) => {
                        safe_ids.push(id.clone());
                        audio_paths.insert(id.clone(), (path, identity));
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "cleanup audio classify failed");
                        result.errors.push(CleanupError {
                            id: id.clone(),
                            issue: CleanupIssue::Io,
                        });
                    }
                }
            }
            let hooks = inner.hooks.clone();
            let outcome = store::delete_records_in_dir(
                &inner.dir,
                &safe_ids,
                inner.local_offset,
                move || hooks.before_history_delete_rename(),
            );
            let outcome = outcome?;
            let deleted_ids = outcome.deleted_ids;
            result.deleted = deleted_ids.len() as u64;
            result.missing = (safe_ids.len().saturating_sub(deleted_ids.len())) as u64;
            if !deleted_ids.is_empty() {
                let scan = scan_stable(&inner.dir, inner.local_offset, &inner.hooks)?;
                apply_scan_outcome(&mut inner, scan);
                events.push(HistoryEvent::Changed);
            }
        }

        for (id, (path, expected_identity)) in audio_paths {
            match assets::classify_cleanup_audio(&audio_dir, &id) {
                Ok(assets::CleanupAudioClass::Missing) => {}
                Ok(assets::CleanupAudioClass::Present {
                    path: current_path,
                    identity,
                    ..
                }) if current_path == path && identity == expected_identity => {
                    if let Err(error) = deleter.delete(&path) {
                        tracing::warn!(error = %error, "cleanup audio trash failed");
                        result.errors.push(CleanupError {
                            id,
                            issue: CleanupIssue::Io,
                        });
                    }
                }
                Ok(assets::CleanupAudioClass::Unsafe(issue)) => {
                    result.errors.push(CleanupError { id, issue });
                }
                Ok(assets::CleanupAudioClass::Present { .. }) => {
                    tracing::warn!(%id, "cleanup audio changed after preflight; keeping file");
                    result.errors.push(CleanupError {
                        id,
                        issue: CleanupIssue::Io,
                    });
                }
                Err(error) => {
                    tracing::warn!(error = %error, "cleanup audio revalidation failed");
                    result.errors.push(CleanupError {
                        id,
                        issue: CleanupIssue::Io,
                    });
                }
            }
        }
        self.publish(events);
        Ok(result)
    }

    pub fn append(&self, record: HistoryRecord) -> Result<()> {
        let mut events = Vec::new();
        {
            let mut inner = self.inner.lock().expect("history service lock poisoned");
            reconcile_if_initialized(&mut inner, &mut events)?;
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
                index.fingerprints = new_fingerprints.clone();
                inner.observed = new_fingerprints;
                inner.dirty.clear();
                inner.failed = None;
            } else {
                let name = store::path_for_month_in_dir(&inner.dir, record.started_at)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string);
                if let Some(name) = name {
                    if let Ok(fingerprint) = fingerprint_optional_path(&inner.dir.join(&name)) {
                        update_observed_entry(&mut inner.observed, name, fingerprint);
                    }
                }
            }
            events.push(HistoryEvent::Appended(Box::new(record)));
        }
        self.publish(events);
        Ok(())
    }

    pub fn watch(&self) -> Result<crate::history::HistoryWatcher> {
        crate::history::watcher::start(self.clone())
    }

    pub(crate) fn history_dir_for_watcher(&self) -> PathBuf {
        self.inner
            .lock()
            .expect("history service lock poisoned")
            .dir
            .clone()
    }

    pub(crate) fn mark_history_paths_changed(&self, paths: &[PathBuf]) {
        let mut events = Vec::new();
        {
            let mut inner = self.inner.lock().expect("history service lock poisoned");
            mark_paths_dirty(&mut inner, paths, &mut events);
        }
        self.publish(events);
    }

    pub(crate) fn mark_history_watcher_error(&self) {
        let mut events = Vec::new();
        {
            let mut inner = self.inner.lock().expect("history service lock poisoned");
            let was_clean = inner.dirty.is_empty();
            inner.dirty.mark_all();
            if was_clean && !matches!(inner.state, IndexState::Uninitialized) {
                events.push(HistoryEvent::Changed);
            }
        }
        self.publish(events);
    }

    fn publish(&self, events: Vec<HistoryEvent>) {
        for event in events {
            let _ = self.events.send(event);
        }
    }

    #[cfg(test)]
    pub(crate) fn set_test_local_offset(&self, offset: UtcOffset) {
        let mut inner = self.inner.lock().expect("history service lock poisoned");
        if inner.local_offset != offset {
            inner.local_offset = offset;
            inner.state = IndexState::Uninitialized;
            inner.dirty.clear();
            inner.failed = None;
            inner.observed = FileSetFingerprint(Vec::new());
        }
    }

    #[cfg(test)]
    pub(crate) fn debug_dirty_month_count(&self) -> usize {
        self.inner
            .lock()
            .expect("history service lock poisoned")
            .dirty
            .count()
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

fn ensure_index<'a>(
    inner: &'a mut ServiceInner,
    events: &mut Vec<HistoryEvent>,
) -> Result<IndexView<'a>> {
    if let IndexState::Ready(index)
    | IndexState::Stale {
        last_valid: index, ..
    } = &inner.state
    {
        if index.offset != inner.local_offset {
            inner.state = IndexState::Uninitialized;
            inner.dirty.clear();
            inner.failed = None;
            inner.observed = FileSetFingerprint(Vec::new());
        }
    }

    if matches!(inner.state, IndexState::Uninitialized) {
        apply_scan_outcome(
            inner,
            scan_stable(&inner.dir, inner.local_offset, &inner.hooks)?,
        );
    } else {
        reconcile_if_initialized(inner, events)?;
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
            inner.dirty.clear();
            inner.observed = index.fingerprints.clone();
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

fn reconcile_if_initialized(
    inner: &mut ServiceInner,
    events: &mut Vec<HistoryEvent>,
) -> Result<()> {
    if matches!(inner.state, IndexState::Uninitialized) {
        return Ok(());
    }
    let current = fingerprint_file_set(&inner.dir, &inner.hooks)?;
    if inner
        .failed
        .as_ref()
        .is_some_and(|failed| current == failed.fingerprint)
    {
        return Ok(());
    }
    let indexed = match &inner.state {
        IndexState::Ready(index)
        | IndexState::Stale {
            last_valid: index, ..
        } => Some(&index.fingerprints),
        IndexState::Unavailable { .. } | IndexState::Uninitialized => None,
    };
    if inner.dirty.is_empty() && indexed.is_some_and(|fingerprint| *fingerprint == current) {
        return Ok(());
    }
    let retrying_failed_fingerprint = inner.failed.is_some();
    if inner.dirty.is_empty() {
        inner.dirty.mark_all();
        events.push(HistoryEvent::Changed);
    }
    let outcome = scan_stable(&inner.dir, inner.local_offset, &inner.hooks)?;
    apply_scan_outcome(inner, outcome);
    if retrying_failed_fingerprint {
        events.push(HistoryEvent::Changed);
    }
    Ok(())
}

fn mark_paths_dirty(inner: &mut ServiceInner, paths: &[PathBuf], events: &mut Vec<HistoryEvent>) {
    let was_clean = inner.dirty.is_empty();
    let mut marked = false;
    for path in paths {
        let Some(name) = month_file_name(path) else {
            continue;
        };
        let current = match fingerprint_optional_path(&inner.dir.join(&name)) {
            Ok(current) => current,
            Err(error) => {
                tracing::warn!(
                    path = %inner.dir.join(&name).display(),
                    error = ?error,
                    "history watcher could not fingerprint changed path"
                );
                inner.dirty.mark_month(name);
                marked = true;
                continue;
            }
        };
        if observed_entry(&inner.observed, &name) == current.as_ref() {
            continue;
        }
        update_observed_entry(&mut inner.observed, name.clone(), current);
        inner.dirty.mark_month(name);
        marked = true;
    }
    if marked && was_clean && !matches!(inner.state, IndexState::Uninitialized) {
        events.push(HistoryEvent::Changed);
    }
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

#[derive(Default)]
struct CleanupCandidates {
    ids: Vec<String>,
    audio_bytes: u64,
    audio_ms: u64,
    oldest: Option<OffsetDateTime>,
    newest: Option<OffsetDateTime>,
    warnings: Vec<CleanupWarning>,
}

/// 与 page/stats 相同的双 fingerprint 稳定读取：读前后 file-set fingerprint 一致才接受，
/// 否则 retry once。扫描期间不持有额外锁，只读文件。
fn cleanup_preview_stable(
    dir: &Path,
    audio_dir: &Path,
    lower: Option<OffsetDateTime>,
    upper: Option<OffsetDateTime>,
    scope: CleanupScope,
    hooks: &Hooks,
) -> Result<CleanupCandidates> {
    let mut last_error = None;
    for attempt in 0..2 {
        hooks.before_scan_attempt();
        let before = fingerprint_file_set(dir, hooks)?;
        match cleanup_preview_once(dir, audio_dir, lower, upper, scope, &before, hooks) {
            Ok(candidates) => {
                if fingerprint_file_set(dir, hooks)? == before {
                    return Ok(candidates);
                }
                last_error = Some("unstable history source during cleanup preview".to_string());
            }
            Err(error) => {
                last_error = Some(error.to_string());
                if attempt == 1 {
                    break;
                }
            }
        }
    }
    bail!(
        "{}",
        last_error.unwrap_or_else(|| "unstable history source during cleanup preview".to_string())
    )
}

fn cleanup_preview_once(
    dir: &Path,
    audio_dir: &Path,
    lower: Option<OffsetDateTime>,
    upper: Option<OffsetDateTime>,
    scope: CleanupScope,
    fingerprint: &FileSetFingerprint,
    hooks: &Hooks,
) -> Result<CleanupCandidates> {
    let mut candidates = CleanupCandidates::default();
    for entry in &fingerprint.0 {
        let path = dir.join(&entry.name);
        let body = fs::read_to_string(&path)
            .with_context(|| format!("read history {}", path.display()))?;
        hooks.after_read_file();
        let has_complete_final_line = body.ends_with('\n');
        let mut lines = body.split('\n').peekable();
        while let Some(line) = lines.next() {
            if line.trim().is_empty() {
                continue;
            }
            let record: HistoryRecord = match serde_json::from_str(line) {
                Ok(record) => record,
                Err(_) if lines.peek().is_none() && !has_complete_final_line => break,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("parse history line in {}", path.display()));
                }
            };
            let in_window = lower.is_none_or(|l| record.started_at >= l)
                && upper.is_none_or(|u| record.started_at < u);
            if !in_window {
                continue;
            }
            match assets::classify_cleanup_audio(audio_dir, &record.id)? {
                assets::CleanupAudioClass::Missing => {
                    if scope == CleanupScope::RecordAndAudio {
                        candidates.audio_ms =
                            candidates.audio_ms.saturating_add(record.asr.audio_ms);
                        candidates.oldest = Some(match candidates.oldest {
                            Some(current) => current.min(record.started_at),
                            None => record.started_at,
                        });
                        candidates.newest = Some(match candidates.newest {
                            Some(current) => current.max(record.started_at),
                            None => record.started_at,
                        });
                        candidates.ids.push(record.id);
                    }
                }
                assets::CleanupAudioClass::Present { size_bytes, .. } => {
                    candidates.audio_bytes = candidates.audio_bytes.saturating_add(size_bytes);
                    candidates.audio_ms = candidates.audio_ms.saturating_add(record.asr.audio_ms);
                    candidates.oldest = Some(match candidates.oldest {
                        Some(current) => current.min(record.started_at),
                        None => record.started_at,
                    });
                    candidates.newest = Some(match candidates.newest {
                        Some(current) => current.max(record.started_at),
                        None => record.started_at,
                    });
                    candidates.ids.push(record.id);
                }
                assets::CleanupAudioClass::Unsafe(issue) => {
                    candidates.warnings.push(CleanupWarning {
                        id: record.id,
                        issue,
                    })
                }
            }
        }
    }
    Ok(candidates)
}

fn validate_record(record: &HistoryRecord, path: &Path, line_no: usize) -> Result<()> {
    if !matches!(record.version, 1 | 2) {
        bail!(
            "unsupported history schema version {} at {}:{}",
            record.version,
            path.display(),
            line_no
        );
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct PreparedHistoryQuery {
    limit: usize,
    before: Option<OffsetDateTime>,
    before_id: Option<String>,
    query: Option<String>,
}

impl PreparedHistoryQuery {
    fn new(query: HistoryQuery) -> Result<Self> {
        if query.before_id.is_some() && query.before.is_none() {
            bail!("bad_command: before_id requires before");
        }
        Ok(Self {
            limit: normalize_history_limit(query.limit),
            before: query.before,
            before_id: query.before_id,
            query: query.query.map(|value| value.to_lowercase()),
        })
    }

    fn includes(&self, record: &HistoryRecord) -> bool {
        if let Some(before) = self.before {
            if let Some(before_id) = self.before_id.as_deref() {
                if record.started_at > before
                    || (record.started_at == before && record.id.as_str() >= before_id)
                {
                    return false;
                }
            } else if record.started_at >= before {
                return false;
            }
        }
        self.query
            .as_deref()
            .is_none_or(|query| history_record_matches(record, query))
    }
}

fn normalize_history_limit(limit: usize) -> usize {
    if limit == 0 {
        DEFAULT_HISTORY_PAGE_LIMIT
    } else {
        limit.min(MAX_HISTORY_PAGE_LIMIT)
    }
}

fn page_stable(dir: &Path, query: HistoryQuery, hooks: &Hooks) -> Result<HistoryPageResult> {
    let query = PreparedHistoryQuery::new(query)?;
    let mut last_error = None;
    for attempt in 0..2 {
        hooks.before_scan_attempt();
        let before = fingerprint_file_set(dir, hooks)?;
        match page_once(dir, &before, &query, hooks) {
            Ok(read) => {
                if fingerprint_file_set(dir, hooks)? == before {
                    return Ok(HistoryPageResult {
                        records: read.records,
                        matched: read.matched,
                        stats: read.stats,
                    });
                }
                last_error = Some("unstable history source during page read".to_string());
            }
            Err(error) => {
                last_error = Some(error.to_string());
                if attempt == 1 {
                    break;
                }
            }
        }
    }
    bail!(
        "{}",
        last_error.unwrap_or_else(|| "unstable history source during page read".to_string())
    )
}

struct PageRead {
    records: Vec<HistoryRecord>,
    matched: Option<u64>,
    stats: Option<AggregateStats>,
}

fn page_once(
    dir: &Path,
    fingerprint: &FileSetFingerprint,
    query: &PreparedHistoryQuery,
    hooks: &Hooks,
) -> Result<PageRead> {
    let mut cursors = Vec::new();
    let mut heap = std::collections::BinaryHeap::new();

    for entry in &fingerprint.0 {
        let mut cursor = ShardCursor::new(dir.join(&entry.name), hooks.clone())?;
        if let Some(record) = cursor.next_matching(query)? {
            heap.push(HeapEntry {
                key: RecordKey::from(&record),
                cursor_index: cursors.len(),
                record,
            });
        }
        cursors.push(cursor);
    }

    let mut records = Vec::with_capacity(query.limit);
    let mut matched = query.query.as_ref().map(|_| 0u64);
    let mut stats = query.query.as_ref().map(|_| AggregateStats::default());
    while query.query.is_some() || records.len() < query.limit {
        let Some(entry) = heap.pop() else {
            break;
        };
        let cursor_index = entry.cursor_index;
        if let Some(matched) = &mut matched {
            *matched = matched.saturating_add(1);
        }
        if let Some(stats) = &mut stats {
            stats.add_record(&entry.record);
        }
        if records.len() < query.limit {
            records.push(entry.record);
        }
        if let Some(record) = cursors[cursor_index].next_matching(query)? {
            heap.push(HeapEntry {
                key: RecordKey::from(&record),
                cursor_index,
                record,
            });
        }
    }

    Ok(PageRead {
        records,
        matched,
        stats,
    })
}

struct ShardCursor {
    path: PathBuf,
    reader: ReverseLineReader,
    hooks: Hooks,
    last_key: Option<RecordKey>,
}

impl ShardCursor {
    fn new(path: PathBuf, hooks: Hooks) -> Result<Self> {
        Ok(Self {
            reader: ReverseLineReader::open(&path)?,
            path,
            hooks,
            last_key: None,
        })
    }

    fn next_matching(&mut self, query: &PreparedHistoryQuery) -> Result<Option<HistoryRecord>> {
        while let Some(line) = self.reader.next_line()? {
            self.hooks.after_read_file();
            if line.bytes.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let line_str = match std::str::from_utf8(&line.bytes) {
                Ok(value) => value,
                Err(error) if line.is_final_unterminated => {
                    tracing::warn!(
                        path = %self.path.display(),
                        error = %error,
                        "ignoring truncated final history line"
                    );
                    continue;
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("parse history line in {}", self.path.display()));
                }
            };
            let record: HistoryRecord = match serde_json::from_str(line_str) {
                Ok(record) => record,
                Err(error) if line.is_final_unterminated => {
                    tracing::warn!(
                        path = %self.path.display(),
                        error = %error,
                        "ignoring truncated final history line"
                    );
                    continue;
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("parse history line in {}", self.path.display()));
                }
            };
            validate_record(&record, &self.path, 0)?;
            let key = RecordKey::from(&record);
            if self.last_key.as_ref().is_some_and(|last| &key > last) {
                bail!(
                    "history records are not monotonic by (started_at,id) in {}",
                    self.path.display()
                );
            }
            self.last_key = Some(key);
            if query.includes(&record) {
                return Ok(Some(record));
            }
        }
        Ok(None)
    }
}

struct ReverseLine {
    bytes: Vec<u8>,
    is_final_unterminated: bool,
}

struct ReverseLineReader {
    file: fs::File,
    position: u64,
    buffer: Vec<u8>,
    yielded_tail: bool,
    file_ends_with_newline: bool,
}

impl ReverseLineReader {
    fn open(path: &Path) -> Result<Self> {
        let mut file =
            fs::File::open(path).with_context(|| format!("read history {}", path.display()))?;
        let len = file
            .metadata()
            .with_context(|| format!("stat history {}", path.display()))?
            .len();
        let file_ends_with_newline = if len == 0 {
            true
        } else {
            file.seek(SeekFrom::Start(len - 1))
                .with_context(|| format!("seek history {}", path.display()))?;
            let mut byte = [0u8; 1];
            file.read_exact(&mut byte)
                .with_context(|| format!("read history {}", path.display()))?;
            byte[0] == b'\n'
        };
        Ok(Self {
            file,
            position: len,
            buffer: Vec::new(),
            yielded_tail: false,
            file_ends_with_newline,
        })
    }

    fn next_line(&mut self) -> Result<Option<ReverseLine>> {
        loop {
            if let Some(index) = self.buffer.iter().rposition(|byte| *byte == b'\n') {
                let bytes = self.buffer[index + 1..].to_vec();
                self.buffer.truncate(index);
                let is_final_unterminated = !self.yielded_tail && !self.file_ends_with_newline;
                self.yielded_tail = true;
                return Ok(Some(ReverseLine {
                    bytes,
                    is_final_unterminated,
                }));
            }
            if self.position == 0 {
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                let bytes = std::mem::take(&mut self.buffer);
                let is_final_unterminated = !self.yielded_tail && !self.file_ends_with_newline;
                self.yielded_tail = true;
                return Ok(Some(ReverseLine {
                    bytes,
                    is_final_unterminated,
                }));
            }

            let read_len = self.position.min(REVERSE_READ_CHUNK_SIZE as u64) as usize;
            self.position -= read_len as u64;
            self.file
                .seek(SeekFrom::Start(self.position))
                .context("seek history chunk")?;
            let mut chunk = vec![0; read_len];
            self.file
                .read_exact(&mut chunk)
                .context("read history chunk")?;
            chunk.extend_from_slice(&self.buffer);
            self.buffer = chunk;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RecordKey {
    started_at: OffsetDateTime,
    id: String,
}

impl From<&HistoryRecord> for RecordKey {
    fn from(record: &HistoryRecord) -> Self {
        Self {
            started_at: record.started_at,
            id: record.id.clone(),
        }
    }
}

struct HeapEntry {
    key: RecordKey,
    cursor_index: usize,
    record: HistoryRecord,
}

impl PartialEq for HeapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.cursor_index == other.cursor_index
    }
}

impl Eq for HeapEntry {}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key
            .cmp(&other.key)
            .then_with(|| self.cursor_index.cmp(&other.cursor_index))
    }
}

fn history_record_matches(record: &HistoryRecord, query: &str) -> bool {
    let haystack = [
        record.id.as_str(),
        record.app.as_deref().unwrap_or_default(),
        record.asr.text.as_str(),
        &record.text,
    ]
    .join("\n")
    .to_lowercase();
    haystack.contains(query)
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
        AnalyticsPeriod::Last7Days => rolling_day_points(index, &query.anchor, 7),
        AnalyticsPeriod::Last30Days => rolling_day_points(index, &query.anchor, 30),
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

fn rolling_day_points(index: &Index, anchor: &str, days: u16) -> Result<Vec<AnalyticsPoint>> {
    let end = parse_anchor_date(anchor)?;
    let start = end
        .checked_sub(time::Duration::days(i64::from(days.saturating_sub(1))))
        .ok_or_else(|| anyhow!("invalid rolling analytics anchor: {anchor}"))?;
    let mut points = Vec::with_capacity(usize::from(days));
    for offset in 0..days {
        let date = start
            .checked_add(time::Duration::days(i64::from(offset)))
            .ok_or_else(|| anyhow!("invalid rolling analytics anchor: {anchor}"))?;
        let mut stats = AggregateStats::default();
        for hour in 0..24 {
            let key = LocalHour {
                year: date.year(),
                month: u8::from(date.month()),
                day: date.day(),
                hour,
            };
            stats.add_stats(index.hourly.get(&key).copied().unwrap_or_default());
        }
        points.push(AnalyticsPoint {
            key: format!("{:02}-{:02}", u8::from(date.month()), date.day()),
            stats,
        });
    }
    Ok(points)
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

fn month_file_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    store::is_monthly_history_file(name).then(|| name.to_string())
}

fn observed_entry<'a>(observed: &'a FileSetFingerprint, name: &str) -> Option<&'a FileFingerprint> {
    observed
        .0
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| &entry.fingerprint)
}

fn update_observed_entry(
    observed: &mut FileSetFingerprint,
    name: String,
    fingerprint: Option<FileFingerprint>,
) {
    match fingerprint {
        Some(fingerprint) => match observed.0.binary_search_by(|entry| entry.name.cmp(&name)) {
            Ok(index) => observed.0[index].fingerprint = fingerprint,
            Err(index) => observed
                .0
                .insert(index, FileFingerprintEntry { name, fingerprint }),
        },
        None => {
            if let Ok(index) = observed.0.binary_search_by(|entry| entry.name.cmp(&name)) {
                observed.0.remove(index);
            }
        }
    }
}

fn fingerprint_optional_path(path: &Path) -> Result<Option<FileFingerprint>> {
    match fingerprint_path(path) {
        Ok(fingerprint) => Ok(Some(fingerprint)),
        Err(error) if is_not_found(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
        .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
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

        pub fn with_before_history_delete_rename(
            mut self,
            hook: impl Fn() + Send + Sync + 'static,
        ) -> Self {
            self.hooks.before_history_delete_rename = Some(Arc::new(hook));
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
    use std::path::Path;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };

    use time::macros::{datetime, offset};

    use crate::history::{
        store::path_for_month_in_dir, AnalyticsPeriod, AnalyticsQuery, HistoryQuery,
        HistoryService, HistoryStatsStatus,
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
    fn legacy_records_without_text_stats_are_counted_from_text() {
        let dir = temp_dir("legacy-text-stats");
        let path = path_for_month_in_dir(&dir, datetime!(2026-06-01 00:00:00 UTC));
        fs::create_dir_all(&dir).unwrap();
        let mut value = serde_json::to_value(record(
            "legacy",
            datetime!(2026-06-01 00:00:00 UTC),
            "hello world",
        ))
        .unwrap();
        value.as_object_mut().unwrap().remove("text_stats");
        fs::write(&path, format!("{value}\n")).unwrap();

        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());
        let snapshot = service.stats();
        let page = service.page(HistoryQuery::default()).unwrap();

        assert_eq!(snapshot.status, HistoryStatsStatus::Ready);
        assert_eq!(snapshot.total.records, 1);
        assert_eq!(snapshot.total.words, 2);
        assert_eq!(page[0].text_stats().words, 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn version_two_records_with_v1_shape_are_indexed() {
        let dir = temp_dir("version-two");
        let path = path_for_month_in_dir(&dir, datetime!(2026-06-01 00:00:00 UTC));
        fs::create_dir_all(&dir).unwrap();
        let mut value = serde_json::to_value(record(
            "v2",
            datetime!(2026-06-01 00:00:00 UTC),
            "hello world",
        ))
        .unwrap();
        value["version"] = serde_json::json!(2);
        fs::write(&path, format!("{value}\n")).unwrap();

        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());
        let snapshot = service.stats();

        assert_eq!(snapshot.status, HistoryStatsStatus::Ready);
        assert_eq!(snapshot.total.records, 1);
        assert_eq!(snapshot.total.words, 2);
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
    fn rolling_day_analytics_returns_fixed_windows_across_months() {
        let dir = temp_dir("rolling-buckets");
        write_line(
            &dir,
            record("a", datetime!(2026-06-30 12:00:00 UTC), "hello"),
        );
        write_line(
            &dir,
            record("b", datetime!(2026-07-02 12:00:00 UTC), "world"),
        );
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let seven = service
            .analytics(AnalyticsQuery::new(
                AnalyticsPeriod::Last7Days,
                "2026-07-03",
            ))
            .unwrap();
        let thirty = service
            .analytics(AnalyticsQuery::new(
                AnalyticsPeriod::Last30Days,
                "2026-07-03",
            ))
            .unwrap();

        assert_eq!(seven.points.len(), 7);
        assert_eq!(seven.points[0].key, "06-27");
        assert_eq!(seven.points[6].key, "07-03");
        assert_eq!(seven.points.iter().map(|p| p.stats.records).sum::<u64>(), 2);
        assert_eq!(thirty.points.len(), 30);
        assert_eq!(
            thirty.points.iter().map(|p| p.stats.records).sum::<u64>(),
            2
        );
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

    #[test]
    fn delete_after_stale_rescans_and_returns_ready_stats() {
        let dir = temp_dir("delete-stale-rescan");
        let delete_id = "01KB1KQ4GF8AFS76W7W7HZ5VSZ";
        let keep_id = "01KB1KQ4GF8AFS76W7W7HZ5VT0";
        let external_id = "01KB1KQ4GF8AFS76W7W7HZ5VT3";
        let appended_id_1 = "01KB1KQ4GF8AFS76W7W7HZ5VT1";
        let appended_id_2 = "01KB1KQ4GF8AFS76W7W7HZ5VT2";
        write_line(
            &dir,
            record(delete_id, datetime!(2026-06-01 00:00:00 UTC), "one"),
        );
        write_line(
            &dir,
            record(keep_id, datetime!(2026-06-01 01:00:00 UTC), "two"),
        );
        let mutate = Arc::new(AtomicBool::new(false));
        let mutation = Arc::new(AtomicUsize::new(0));
        let hooks = TestHooks::default().with_after_read_file({
            let dir = dir.clone();
            let mutate = Arc::clone(&mutate);
            let mutation = Arc::clone(&mutation);
            move || {
                if !mutate.load(Ordering::SeqCst) {
                    return;
                }
                let id = match mutation.fetch_add(1, Ordering::SeqCst) {
                    0 => appended_id_1,
                    1 => appended_id_2,
                    _ => return,
                };
                let timestamp = if id == appended_id_1 {
                    datetime!(2026-06-01 03:00:00 UTC)
                } else {
                    datetime!(2026-06-01 04:00:00 UTC)
                };
                write_line(&dir, record(id, timestamp, "extra"));
            }
        });
        let service = HistoryService::with_test_hooks(dir.clone(), offset!(+0), hooks);
        assert_eq!(service.stats().status, HistoryStatsStatus::Ready);

        write_line(
            &dir,
            record(external_id, datetime!(2026-06-01 02:00:00 UTC), "extra"),
        );
        mutate.store(true, Ordering::SeqCst);
        let stale = service.stats();
        mutate.store(false, Ordering::SeqCst);
        assert_eq!(stale.status, HistoryStatsStatus::Stale);
        assert_eq!(stale.total.records, 2);

        let result = service.delete(delete_id).unwrap();
        let snapshot = service.stats();

        assert!(result.record_deleted);
        assert_eq!(snapshot.status, HistoryStatsStatus::Ready);
        assert_eq!(snapshot.total.records, 4);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn latest_page_reads_newest_records_across_months() {
        let dir = temp_dir("page-across-months");
        write_line(
            &dir,
            record("may", datetime!(2026-05-31 23:59:00 UTC), "may"),
        );
        write_line(
            &dir,
            record("jun", datetime!(2026-06-30 23:59:00 UTC), "jun"),
        );
        write_line(
            &dir,
            record("jul", datetime!(2026-07-01 00:00:00 UTC), "jul"),
        );
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let records = service
            .page(HistoryQuery {
                limit: 2,
                ..HistoryQuery::default()
            })
            .unwrap();

        assert_ids(&records, &["jul", "jun"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn overlapping_shards_are_k_way_merged_by_record_order() {
        let dir = temp_dir("page-k-way");
        write_line(
            &dir,
            record("a-old", datetime!(2026-06-30 23:00:00 UTC), "old"),
        );
        write_raw_record(
            &dir.join("2026-05.jsonl"),
            record("z-new", datetime!(2026-07-01 00:00:00 UTC), "new"),
        );
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let records = service
            .page(HistoryQuery {
                limit: 2,
                ..HistoryQuery::default()
            })
            .unwrap();

        assert_ids(&records, &["z-new", "a-old"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn before_id_keeps_equal_timestamp_records_pageable() {
        let dir = temp_dir("page-before-id");
        let ts = datetime!(2026-06-01 00:00:00 UTC);
        write_line(&dir, record("a", ts, "same"));
        write_line(&dir, record("b", ts, "same"));
        write_line(&dir, record("c", ts, "same"));
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let timestamp_only = service
            .page(HistoryQuery {
                limit: 10,
                before: Some(ts),
                ..HistoryQuery::default()
            })
            .unwrap();
        let tuple = service
            .page(HistoryQuery {
                limit: 10,
                before: Some(ts),
                before_id: Some("c".to_string()),
                ..HistoryQuery::default()
            })
            .unwrap();

        assert_ids(&timestamp_only, &[]);
        assert_ids(&tuple, &["b", "a"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn before_id_without_before_is_rejected() {
        let service = HistoryService::with_test_hooks(
            temp_dir("page-bad-before-id"),
            offset!(+0),
            TestHooks::default(),
        );

        let error = service
            .page(HistoryQuery {
                before_id: Some("a".to_string()),
                ..HistoryQuery::default()
            })
            .unwrap_err();

        assert!(error.to_string().contains("bad_command"), "{error:#}");
    }

    #[test]
    fn query_scans_until_it_collects_the_limit() {
        let dir = temp_dir("page-query-limit");
        write_line(
            &dir,
            record("old-hit", datetime!(2026-05-01 00:00:00 UTC), "needle"),
        );
        write_line(
            &dir,
            record("miss", datetime!(2026-06-01 00:00:00 UTC), "plain"),
        );
        write_line(
            &dir,
            record("new-hit", datetime!(2026-07-01 00:00:00 UTC), "needle"),
        );
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let records = service
            .page(HistoryQuery {
                limit: 2,
                query: Some("needle".to_string()),
                ..HistoryQuery::default()
            })
            .unwrap();

        assert_ids(&records.records, &["new-hit", "old-hit"]);
        assert_eq!(records.matched, Some(2));
        assert_eq!(records.stats.unwrap().records, 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn query_metadata_counts_matches_beyond_page_limit() {
        let dir = temp_dir("page-query-metadata");
        write_line(
            &dir,
            record("old-hit", datetime!(2026-05-01 00:00:00 UTC), "needle one"),
        );
        write_line(
            &dir,
            record("mid-hit", datetime!(2026-06-01 00:00:00 UTC), "needle two"),
        );
        write_line(
            &dir,
            record(
                "new-hit",
                datetime!(2026-07-01 00:00:00 UTC),
                "needle three",
            ),
        );
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let page = service
            .page(HistoryQuery {
                limit: 1,
                query: Some("needle".to_string()),
                ..HistoryQuery::default()
            })
            .unwrap();

        assert_ids(&page.records, &["new-hit"]);
        assert_eq!(page.matched, Some(3));
        let stats = page.stats.unwrap();
        assert_eq!(stats.records, 3);
        assert_eq!(stats.words, 6);
        assert_eq!(stats.duration_ms, 3000);
        assert_eq!(stats.asr_duration_ms, 3000);
        assert_eq!(stats.asr_audio_ms, 3000);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn search_query_is_filtered_by_daemon_not_tui_loaded_records() {
        let dir = temp_dir("page-query-fields");
        let mut app_match = record("app-match", datetime!(2026-06-01 00:00:00 UTC), "plain");
        app_match.app = Some("Com.Example.Special".to_string());
        let mut asr_match = record("asr-match", datetime!(2026-06-01 01:00:00 UTC), "plain");
        asr_match.asr.text = "provider Needle text".to_string();
        write_line(&dir, app_match);
        write_line(&dir, asr_match);
        write_line(
            &dir,
            record(
                "id-Needle-match",
                datetime!(2026-06-01 02:00:00 UTC),
                "plain",
            ),
        );
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let records = service
            .page(HistoryQuery {
                limit: 10,
                query: Some("needle".to_string()),
                ..HistoryQuery::default()
            })
            .unwrap();

        assert_ids(&records, &["id-Needle-match", "asr-match"]);
        let app_records = service
            .page(HistoryQuery {
                limit: 10,
                query: Some("special".to_string()),
                ..HistoryQuery::default()
            })
            .unwrap();
        assert_ids(&app_records, &["app-match"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn reverse_reader_handles_utf8_and_lines_across_chunks() {
        let dir = temp_dir("page-utf8-chunks");
        let path = dir.join("2026-06.jsonl");
        for n in 0..40 {
            write_raw_record(
                &path,
                record(
                    &format!("r{n:02}"),
                    datetime!(2026-06-01 00:00:00 UTC) + time::Duration::seconds(n),
                    &format!("跨块 UTF-8 文本 {n} 😀"),
                ),
            );
        }
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let records = service
            .page(HistoryQuery {
                limit: 3,
                ..HistoryQuery::default()
            })
            .unwrap();

        assert_ids(&records, &["r39", "r38", "r37"]);
        assert_eq!(records[0].text, "跨块 UTF-8 文本 39 😀");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn newline_exactly_on_chunk_boundary_is_handled() {
        let dir = temp_dir("page-newline-boundary");
        let path = dir.join("2026-06.jsonl");
        write_chunk_sized_line(
            &path,
            "boundary-old",
            datetime!(2026-06-01 00:00:00 UTC),
            2048,
        );
        write_raw_record(
            &path,
            record("boundary-new", datetime!(2026-06-01 00:00:01 UTC), "new"),
        );
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let records = service
            .page(HistoryQuery {
                limit: 2,
                ..HistoryQuery::default()
            })
            .unwrap();

        assert_ids(&records, &["boundary-new", "boundary-old"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn truncated_tail_spanning_chunks_is_ignored() {
        let dir = temp_dir("page-truncated-tail");
        let path = dir.join("2026-06.jsonl");
        write_raw_record(
            &path,
            record("valid", datetime!(2026-06-01 00:00:00 UTC), "valid"),
        );
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(br#"{"version":1,"id":"truncated","text":""#)
            .unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&vec![b'x'; 2048])
            .unwrap();
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let records = service.page(HistoryQuery::default()).unwrap();

        assert_ids(&records, &["valid"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_complete_line_is_rejected() {
        let dir = temp_dir("page-corrupt-line");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("2026-06.jsonl"), "not-json\n").unwrap();
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let error = service.page(HistoryQuery::default()).unwrap_err();

        assert!(
            error.to_string().contains("parse history line"),
            "{error:#}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn page_rejects_month_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("page-symlink");
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("target.jsonl");
        fs::write(&target, "").unwrap();
        symlink(&target, dir.join("2026-06.jsonl")).unwrap();
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let error = service.page(HistoryQuery::default()).unwrap_err();

        assert!(error.to_string().contains("symlink"), "{error:#}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn page_retries_when_participating_file_changes_during_read() {
        let dir = temp_dir("page-retry");
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

        let records = service.page(HistoryQuery::default()).unwrap();

        assert_ids(&records, &["b", "a"]);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn page_retries_when_file_set_changes_during_read() {
        let dir = temp_dir("page-file-set-retry");
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
                        write_raw_record(
                            &dir.join("2026-07.jsonl"),
                            record("b", datetime!(2026-07-01 00:00:00 UTC), "two"),
                        );
                    }
                }
            });
        let service = HistoryService::with_test_hooks(dir.clone(), offset!(+0), hooks);

        let records = service.page(HistoryQuery::default()).unwrap();

        assert_ids(&records, &["b", "a"]);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn page_rejects_non_monotonic_complete_shard() {
        let dir = temp_dir("page-non-monotonic");
        let path = dir.join("2026-06.jsonl");
        write_raw_record(
            &path,
            record("new-first", datetime!(2026-06-01 00:00:10 UTC), "new"),
        );
        write_raw_record(
            &path,
            record("old-second", datetime!(2026-06-01 00:00:00 UTC), "old"),
        );
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

        let error = service.page(HistoryQuery::default()).unwrap_err();

        assert!(error.to_string().contains("monotonic"), "{error:#}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn page_reports_unstable_source_after_second_read_change() {
        let dir = temp_dir("page-unstable");
        write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
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
                let next = Arc::new(AtomicUsize::new(0));
                move || {
                    let id = next.fetch_add(1, Ordering::SeqCst);
                    write_line(
                        &dir,
                        record(
                            &format!("changed-{id}"),
                            datetime!(2026-06-01 01:00:00 UTC) + time::Duration::seconds(id as i64),
                            "two",
                        ),
                    );
                }
            });
        let service = HistoryService::with_test_hooks(dir.clone(), offset!(+0), hooks);

        let error = service.page(HistoryQuery::default()).unwrap_err();

        assert!(error.to_string().contains("unstable"), "{error:#}");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        let _ = fs::remove_dir_all(dir);
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("shuohua-history-{name}-{}", ulid::Ulid::generate()))
    }

    fn write_line(dir: &std::path::Path, record: crate::history::HistoryRecord) {
        let path = path_for_month_in_dir(dir, record.started_at);
        crate::history::store::append_record(&path, &record).unwrap();
    }

    fn write_raw_record(path: &Path, record: crate::history::HistoryRecord) {
        crate::history::store::append_record(path, &record).unwrap();
    }

    fn assert_ids(records: &[crate::history::HistoryRecord], expected: &[&str]) {
        let ids: Vec<_> = records.iter().map(|record| record.id.as_str()).collect();
        assert_eq!(ids, expected);
    }

    fn write_chunk_sized_line(
        path: &Path,
        id: &str,
        started_at: time::OffsetDateTime,
        target_len: usize,
    ) {
        let record = record(id, started_at, "");
        let mut line = serde_json::to_vec(&record).unwrap();
        assert!(
            line.len() < target_len,
            "record line exceeded target length {target_len}"
        );
        line.resize(target_len - 1, b' ');
        line.push(b'\n');
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(&line)
            .unwrap();
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

#[cfg(test)]
mod cleanup_tests {
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };

    use time::macros::{datetime, offset};
    use time::{Duration, OffsetDateTime};

    use crate::history::{
        stats::tests_support::TestHooks, AsrHistory, AsrSessionHistory, CleanupFilter,
        CleanupIssue, CleanupScope, CleanupWindow, HistoryRecord, HistoryService, HistoryStatus,
        PipelineStepHistory, PipelineStepStatus,
    };

    const NOW: OffsetDateTime = datetime!(2026-07-07 12:00:00 UTC);

    fn audio_only(window: CleanupWindow) -> CleanupFilter {
        CleanupFilter {
            scope: CleanupScope::AudioOnly,
            window,
        }
    }

    fn record_and_audio(window: CleanupWindow) -> CleanupFilter {
        CleanupFilter {
            scope: CleanupScope::RecordAndAudio,
            window,
        }
    }

    struct Fixture {
        service: HistoryService,
        root: PathBuf,
        history_dir: PathBuf,
        audio_dir: PathBuf,
        deleted: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            Self::new_with_hooks(name, TestHooks::default())
        }

        fn new_with_hooks(name: &str, hooks: TestHooks) -> Self {
            let root = std::env::temp_dir()
                .join(format!("shuohua-cleanup-{name}-{}", ulid::Ulid::generate()));
            let history_dir = root.join("history");
            let audio_dir = root.join("audio");
            fs::create_dir_all(&history_dir).unwrap();
            fs::create_dir_all(&audio_dir).unwrap();
            // 记录型 deleter：真实删文件（不碰废纸篓）并记录被删路径，供断言删除确实
            // 走了注入的 seam。
            let (deleter, deleted) = crate::trash::recording_deleter();
            Self {
                service: HistoryService::with_test_hooks(history_dir.clone(), offset!(+0), hooks)
                    .with_deleter(deleter),
                root,
                history_dir,
                audio_dir,
                deleted,
            }
        }

        fn deleted_paths(&self) -> Vec<PathBuf> {
            self.deleted.lock().unwrap().clone()
        }

        fn write_record(&self, id: &str, started_at: OffsetDateTime) {
            let record = record(id, started_at);
            let path = crate::history::store::path_for_month_in_dir(&self.history_dir, started_at);
            crate::history::store::append_record(&path, &record).unwrap();
        }

        fn write_audio(&self, id: &str, ext: &str, bytes: &[u8]) {
            fs::write(self.audio_dir.join(format!("{id}.{ext}")), bytes).unwrap();
        }

        fn audio_exists(&self, id: &str) -> bool {
            self.audio_dir.join(format!("{id}.flac")).exists()
                || self.audio_dir.join(format!("{id}.m4a")).exists()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn ulid_id() -> String {
        ulid::Ulid::generate().to_string()
    }

    fn record(id: &str, started_at: OffsetDateTime) -> HistoryRecord {
        HistoryRecord {
            version: 1,
            id: id.to_string(),
            started_at,
            ended_at: started_at + Duration::seconds(1),
            duration_ms: 1000,
            status: HistoryStatus::Submitted,
            app: None,
            text: "hello".to_string(),
            text_stats: crate::text_stats::compute("hello"),
            asr: AsrHistory {
                provider: "test".to_string(),
                text: "hello".to_string(),
                duration_ms: 1000,
                audio_ms: 1000,
                sessions: vec![AsrSessionHistory {
                    text: "hello".to_string(),
                    started_at,
                    ended_at: started_at + Duration::seconds(1),
                    audio_ms: 1000,
                }],
            },
            pipeline: vec![PipelineStepHistory {
                name: "test".to_string(),
                status: PipelineStepStatus::Ok,
                duration_ms: 1.0,
                text: Some("hello".to_string()),
                error: None,
            }],
            error: None,
        }
    }

    #[test]
    fn delete_audio_routes_through_injected_deleter() {
        let fx = Fixture::new("delete-audio-seam");
        let id = ulid_id();
        fx.write_record(&id, NOW - Duration::days(1));
        fx.write_audio(&id, "flac", &[0u8; 8]);
        let audio_path = fx.audio_dir.join(format!("{id}.flac"));

        let result = fx.service.delete_audio(&id).unwrap();

        assert!(result.deleted);
        assert!(
            !audio_path.exists(),
            "audio must leave its original location"
        );
        assert_eq!(fx.deleted_paths(), vec![audio_path]);
    }

    #[test]
    fn preview_includes_only_records_with_present_audio() {
        let fx = Fixture::new("present-audio");
        let a = ulid_id();
        let b = ulid_id();
        let c = ulid_id();
        // Written monotonically by started_at within the month shard.
        fx.write_record(&a, NOW - Duration::days(40));
        fx.write_record(&b, NOW - Duration::days(39));
        fx.write_record(&c, NOW - Duration::days(38));
        fx.write_audio(&a, "flac", &[0u8; 10]);
        // b has no audio.
        fx.write_audio(&c, "m4a", &[0u8; 25]);

        let preview = fx
            .service
            .preview_cleanup_at(NOW, audio_only(CleanupWindow::All))
            .unwrap();

        assert_eq!(preview.ids, vec![a, c]);
        assert_eq!(preview.audio_bytes, 35);
        // Each test record carries asr.audio_ms = 1000; two matched records.
        assert_eq!(preview.audio_ms, 2000);
        assert!(preview.warnings.is_empty());
        assert_eq!(preview.oldest, Some(NOW - Duration::days(40)));
        assert_eq!(preview.newest, Some(NOW - Duration::days(38)));
    }

    #[test]
    fn preview_age_cutoff_excludes_recent_records() {
        let fx = Fixture::new("age-cutoff");
        let old = ulid_id();
        let recent = ulid_id();
        fx.write_record(&old, NOW - Duration::days(40));
        fx.write_record(&recent, NOW - Duration::days(10));
        fx.write_audio(&old, "flac", &[0u8; 8]);
        fx.write_audio(&recent, "flac", &[0u8; 8]);

        let preview = fx
            .service
            .preview_cleanup_at(NOW, audio_only(CleanupWindow::OlderThanDays(30)))
            .unwrap();

        assert_eq!(preview.ids, vec![old]);
    }

    #[test]
    fn preview_reports_conflict_audio_as_warning_not_candidate() {
        let fx = Fixture::new("conflict");
        let id = ulid_id();
        fx.write_record(&id, NOW - Duration::days(40));
        fx.write_audio(&id, "flac", &[1]);
        fx.write_audio(&id, "m4a", &[2]);

        let preview = fx
            .service
            .preview_cleanup_at(NOW, audio_only(CleanupWindow::All))
            .unwrap();

        assert!(preview.ids.is_empty());
        assert_eq!(preview.warnings.len(), 1);
        assert_eq!(preview.warnings[0].id, id);
        assert_eq!(preview.warnings[0].issue, CleanupIssue::Conflict);
    }

    #[cfg(unix)]
    #[test]
    fn preview_reports_symlink_audio_as_warning() {
        use std::os::unix::fs::symlink;

        let fx = Fixture::new("symlink");
        let id = ulid_id();
        fx.write_record(&id, NOW - Duration::days(40));
        let target = fx.audio_dir.join("target.flac");
        fs::write(&target, [1]).unwrap();
        symlink(&target, fx.audio_dir.join(format!("{id}.flac"))).unwrap();

        let preview = fx
            .service
            .preview_cleanup_at(NOW, audio_only(CleanupWindow::All))
            .unwrap();

        assert!(preview.ids.is_empty());
        assert_eq!(preview.warnings.len(), 1);
        assert_eq!(preview.warnings[0].issue, CleanupIssue::Symlink);
    }

    #[test]
    fn execute_deletes_present_audio_and_keeps_records() {
        let fx = Fixture::new("execute");
        let a = ulid_id();
        let c = ulid_id();
        fx.write_record(&a, NOW - Duration::days(40));
        fx.write_record(&c, NOW - Duration::days(38));
        fx.write_audio(&a, "flac", &[0u8; 10]);
        fx.write_audio(&c, "m4a", &[0u8; 10]);

        let result = fx
            .service
            .execute_cleanup(audio_only(CleanupWindow::All), vec![a.clone(), c.clone()])
            .unwrap();

        assert_eq!(result.requested, 2);
        assert_eq!(result.deleted, 2);
        assert_eq!(result.missing, 0);
        assert!(result.errors.is_empty());
        assert!(!fx.audio_exists(&a));
        assert!(!fx.audio_exists(&c));
        // 删除走了注入的 deleter（生产=废纸篓），不是硬编码 fs::remove_file。
        let deleted = fx.deleted_paths();
        assert!(deleted.contains(&fx.audio_dir.join(format!("{a}.flac"))));
        assert!(deleted.contains(&fx.audio_dir.join(format!("{c}.m4a"))));
        // JSONL records untouched.
        let records = fx
            .service
            .page(crate::history::HistoryQuery::default())
            .unwrap();
        assert_eq!(records.records.len(), 2);
    }

    #[test]
    fn execute_reports_missing_when_audio_absent() {
        let fx = Fixture::new("missing");
        let id = ulid_id();
        fx.write_record(&id, NOW - Duration::days(40));
        // no audio file written

        let result = fx
            .service
            .execute_cleanup(audio_only(CleanupWindow::All), vec![id])
            .unwrap();

        assert_eq!(result.requested, 1);
        assert_eq!(result.deleted, 0);
        assert_eq!(result.missing, 1);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn execute_reports_conflict_as_error_without_deleting() {
        let fx = Fixture::new("execute-conflict");
        let id = ulid_id();
        fx.write_record(&id, NOW - Duration::days(40));
        fx.write_audio(&id, "flac", &[1]);
        fx.write_audio(&id, "m4a", &[2]);

        let result = fx
            .service
            .execute_cleanup(audio_only(CleanupWindow::All), vec![id.clone()])
            .unwrap();

        assert_eq!(result.deleted, 0);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].id, id);
        assert_eq!(result.errors[0].issue, CleanupIssue::Conflict);
        assert!(fx.audio_dir.join(format!("{id}.flac")).exists());
        assert!(fx.audio_dir.join(format!("{id}.m4a")).exists());
    }

    #[test]
    fn record_and_audio_preview_includes_records_without_audio() {
        let fx = Fixture::new("record-preview");
        let with_audio = ulid_id();
        let without_audio = ulid_id();
        fx.write_record(&with_audio, NOW - Duration::days(40));
        fx.write_record(&without_audio, NOW - Duration::days(39));
        fx.write_audio(&with_audio, "flac", &[0u8; 10]);

        let preview = fx
            .service
            .preview_cleanup_at(NOW, record_and_audio(CleanupWindow::All))
            .unwrap();

        assert_eq!(preview.ids, vec![with_audio, without_audio]);
        assert_eq!(preview.audio_bytes, 10);
        assert_eq!(preview.audio_ms, 2000);
        assert!(preview.warnings.is_empty());
    }

    #[test]
    fn record_and_audio_execute_rewrites_shards_and_deletes_linked_audio() {
        let fx = Fixture::new("record-execute");
        let keep = ulid_id();
        let delete_with_audio = ulid_id();
        let delete_without_audio = ulid_id();
        fx.write_record(&keep, NOW - Duration::days(40));
        fx.write_record(&delete_with_audio, NOW - Duration::days(39));
        fx.write_record(&delete_without_audio, NOW - Duration::days(38));
        fx.write_audio(&delete_with_audio, "m4a", &[0u8; 10]);

        let result = fx
            .service
            .execute_cleanup(
                record_and_audio(CleanupWindow::All),
                vec![delete_with_audio.clone(), delete_without_audio.clone()],
            )
            .unwrap();

        assert_eq!(result.requested, 2);
        assert_eq!(result.deleted, 2);
        assert_eq!(result.missing, 0);
        assert!(result.errors.is_empty());
        assert!(!fx.audio_exists(&delete_with_audio));
        // 原音频路径直接交给注入的 deleter（生产=trash crate）。
        assert_eq!(
            fx.deleted_paths(),
            vec![fx.audio_dir.join(format!("{delete_with_audio}.m4a"))]
        );

        let page = fx
            .service
            .page(crate::history::HistoryQuery::default())
            .unwrap();
        let ids: Vec<_> = page.records.into_iter().map(|record| record.id).collect();
        assert_eq!(ids, vec![keep]);
    }

    #[test]
    fn record_and_audio_execute_skips_unsafe_audio_before_record_delete() {
        let fx = Fixture::new("record-execute-conflict");
        let id = ulid_id();
        fx.write_record(&id, NOW - Duration::days(40));
        fx.write_audio(&id, "flac", &[1]);
        fx.write_audio(&id, "m4a", &[2]);

        let result = fx
            .service
            .execute_cleanup(record_and_audio(CleanupWindow::All), vec![id.clone()])
            .unwrap();

        assert_eq!(result.deleted, 0);
        assert_eq!(result.missing, 0);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].id, id);
        assert_eq!(result.errors[0].issue, CleanupIssue::Conflict);
        let page = fx
            .service
            .page(crate::history::HistoryQuery::default())
            .unwrap();
        assert_eq!(page.records.len(), 1);
    }

    #[test]
    fn record_and_audio_keeps_audio_when_shard_rewrite_fails() {
        let changed = Arc::new(AtomicBool::new(false));
        let path_slot = Arc::new(std::sync::Mutex::new(None::<PathBuf>));
        let hooks = TestHooks::default().with_before_history_delete_rename({
            let changed = Arc::clone(&changed);
            let path_slot = Arc::clone(&path_slot);
            move || {
                if !changed.swap(true, Ordering::SeqCst) {
                    let path = path_slot.lock().unwrap().clone().unwrap();
                    fs::OpenOptions::new()
                        .append(true)
                        .open(&path)
                        .unwrap()
                        .write_all(b"\n")
                        .unwrap();
                }
            }
        });
        let fx = Fixture::new_with_hooks("record-rollback", hooks);
        let id = ulid_id();
        fx.write_record(&id, NOW - Duration::days(40));
        fx.write_audio(&id, "flac", &[1, 2, 3]);
        *path_slot.lock().unwrap() = Some(crate::history::store::path_for_month_in_dir(
            &fx.history_dir,
            NOW - Duration::days(40),
        ));

        let error = fx
            .service
            .execute_cleanup(record_and_audio(CleanupWindow::All), vec![id.clone()])
            .unwrap_err();

        assert!(error.to_string().contains("changed"), "{error:#}");
        assert!(fx.audio_dir.join(format!("{id}.flac")).exists());
        let page = fx
            .service
            .page(crate::history::HistoryQuery::default())
            .unwrap();
        assert_eq!(page.records.len(), 1);
    }

    #[test]
    fn record_and_audio_keeps_replaced_audio_after_preflight() {
        let audio_slot = Arc::new(std::sync::Mutex::new(None::<PathBuf>));
        let hooks = TestHooks::default().with_before_history_delete_rename({
            let audio_slot = Arc::clone(&audio_slot);
            move || {
                let path = audio_slot.lock().unwrap().clone().unwrap();
                fs::remove_file(&path).unwrap();
                fs::write(&path, b"new recording").unwrap();
            }
        });
        let fx = Fixture::new_with_hooks("record-audio-replaced", hooks);
        let id = ulid_id();
        fx.write_record(&id, NOW - Duration::days(40));
        fx.write_audio(&id, "flac", b"old recording");
        let audio_path = fx.audio_dir.join(format!("{id}.flac"));
        *audio_slot.lock().unwrap() = Some(audio_path.clone());

        let result = fx
            .service
            .execute_cleanup(record_and_audio(CleanupWindow::All), vec![id.clone()])
            .unwrap();

        assert_eq!(result.deleted, 1);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].id, id);
        assert_eq!(result.errors[0].issue, CleanupIssue::Io);
        assert_eq!(fs::read(&audio_path).unwrap(), b"new recording");
        assert!(fx.deleted_paths().is_empty());
    }

    #[test]
    fn record_and_audio_deletes_audio_when_previewed_record_is_already_missing() {
        let fx = Fixture::new("record-missing-audio-present");
        let id = ulid_id();
        fx.write_audio(&id, "flac", &[1, 2, 3]);

        let result = fx
            .service
            .execute_cleanup(record_and_audio(CleanupWindow::All), vec![id.clone()])
            .unwrap();

        assert_eq!(result.requested, 1);
        assert_eq!(result.deleted, 0);
        assert_eq!(result.missing, 1);
        assert!(result.errors.is_empty());
        assert!(!fx.audio_exists(&id));
    }
}
