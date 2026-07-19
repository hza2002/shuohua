use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::text_stats::TextStats;

pub const DEFAULT_HISTORY_PAGE_LIMIT: usize = 50;
pub const MAX_HISTORY_PAGE_LIMIT: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryQuery {
    pub limit: usize,
    #[serde(with = "time::serde::rfc3339::option")]
    pub before: Option<OffsetDateTime>,
    pub before_id: Option<String>,
    pub query: Option<String>,
}

impl Default for HistoryQuery {
    fn default() -> Self {
        Self {
            limit: DEFAULT_HISTORY_PAGE_LIMIT,
            before: None,
            before_id: None,
            query: None,
        }
    }
}

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
    #[serde(default)]
    pub text_stats: TextStats,
    pub asr: AsrHistory,
    pub pipeline: Vec<PipelineStepHistory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<HistoryError>,
}

impl HistoryRecord {
    pub fn text_stats(&self) -> TextStats {
        if self.text_stats.words == 0 && !self.text.is_empty() {
            crate::text_stats::compute(&self.text)
        } else {
            self.text_stats
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteResult {
    pub id: String,
    pub record_deleted: bool,
    pub audio_deleted: bool,
    pub audio_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDeleteResult {
    pub id: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStatus {
    Submitted,
    Canceled,
    Empty,
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

/// 批量清理的操作范围，对应单条 `d`（仅音频）/ `x`（记录+音频）的批量版本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupScope {
    AudioOnly,
    RecordAndAudio,
}

/// 清理命中的时间窗口。记录 `started_at` 落在 `[lower, upper)` 内即命中；
/// `None` 表示该侧无界。预设相对 `now` 解析；`Range` 是绝对日期（UTC 天边界，
/// `to` 按天含尾）。`Range` 的 `Date` 由 UI 构造恒合法，serde 以 ISO 上线。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupWindow {
    All,
    LastHours(u32),
    LastDays(u32),
    OlderThanDays(u32),
    Range { from: time::Date, to: time::Date },
}

impl CleanupWindow {
    /// 解析为 `[lower, upper)` 时刻边界。命中判据：
    /// `lower.map_or(true, |l| started_at >= l) && upper.map_or(true, |u| started_at < u)`。
    pub fn bounds(self, now: OffsetDateTime) -> (Option<OffsetDateTime>, Option<OffsetDateTime>) {
        match self {
            CleanupWindow::All => (None, None),
            CleanupWindow::LastHours(h) => (Some(now - time::Duration::hours(h as i64)), None),
            CleanupWindow::LastDays(d) => (Some(now - time::Duration::days(d as i64)), None),
            CleanupWindow::OlderThanDays(n) => (None, Some(now - time::Duration::days(n as i64))),
            CleanupWindow::Range { from, to } => {
                let lower = from.midnight().assume_utc();
                // `to` 含当天：上界取次日 0 点。
                let upper = to.next_day().unwrap_or(to).midnight().assume_utc();
                (Some(lower), Some(upper))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupFilter {
    pub scope: CleanupScope,
    pub window: CleanupWindow,
}

/// audio 无法被安全清理的原因。`id` 是 ULID，不含正文，可安全上线协议/日志。
/// preview 用它标注被排除的危险音频；execute 另外用 `Io` 表示单条删除时的 IO 失败
/// （不中断整批）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupIssue {
    Conflict,
    Symlink,
    NonRegular,
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupWarning {
    pub id: String,
    pub issue: CleanupIssue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupPreview {
    pub filter: CleanupFilter,
    /// 命中且可安全删除的 audio 对应 record ID 快照；execute 只处理这批。
    pub ids: Vec<String>,
    pub audio_bytes: u64,
    /// 命中记录的语音总时长（Σ asr.audio_ms），供 preview 展示。
    pub audio_ms: u64,
    #[serde(with = "time::serde::rfc3339::option")]
    pub oldest: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub newest: Option<OffsetDateTime>,
    pub warnings: Vec<CleanupWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupError {
    pub id: String,
    pub issue: CleanupIssue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupResult {
    pub requested: u64,
    pub deleted: u64,
    /// preview 后音频已不在（被单条删除或外部改动），非错误。
    pub missing: u64,
    pub errors: Vec<CleanupError>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn all_window_is_unbounded() {
        let now = datetime!(2026-07-07 12:00:00 UTC);
        assert_eq!(CleanupWindow::All.bounds(now), (None, None));
    }

    #[test]
    fn older_than_bounds_upper_only() {
        let now = datetime!(2026-07-07 12:00:00 UTC);
        assert_eq!(
            CleanupWindow::OlderThanDays(30).bounds(now),
            (None, Some(datetime!(2026-06-07 12:00:00 UTC)))
        );
    }

    #[test]
    fn recent_windows_bound_lower_only() {
        let now = datetime!(2026-07-07 12:00:00 UTC);
        assert_eq!(
            CleanupWindow::LastHours(1).bounds(now),
            (Some(datetime!(2026-07-07 11:00:00 UTC)), None)
        );
        assert_eq!(
            CleanupWindow::LastDays(7).bounds(now),
            (Some(datetime!(2026-06-30 12:00:00 UTC)), None)
        );
    }

    #[test]
    fn range_window_is_inclusive_by_day() {
        use time::macros::date;
        let now = datetime!(2026-07-07 12:00:00 UTC);
        assert_eq!(
            CleanupWindow::Range {
                from: date!(2026 - 01 - 01),
                to: date!(2026 - 01 - 31),
            }
            .bounds(now),
            (
                Some(datetime!(2026-01-01 00:00:00 UTC)),
                // `to` day included → upper is next day midnight.
                Some(datetime!(2026-02-01 00:00:00 UTC))
            )
        );
    }

    #[test]
    fn range_round_trips_as_iso_strings() {
        use time::macros::date;
        let window = CleanupWindow::Range {
            from: date!(2026 - 01 - 01),
            to: date!(2026 - 01 - 15),
        };
        let json = serde_json::to_string(&window).unwrap();
        assert!(
            json.contains("2026-01-01") && json.contains("2026-01-15"),
            "{json}"
        );
        assert_eq!(
            serde_json::from_str::<CleanupWindow>(&json).unwrap(),
            window
        );
    }

    #[test]
    fn preview_round_trips_as_json() {
        let preview = CleanupPreview {
            filter: CleanupFilter {
                scope: CleanupScope::AudioOnly,
                window: CleanupWindow::OlderThanDays(30),
            },
            ids: vec!["01HAUDIO".to_string()],
            audio_bytes: 333_447_168,
            audio_ms: 5_300_000,
            oldest: Some(datetime!(2026-04-12 00:00:00 UTC)),
            newest: Some(datetime!(2026-06-01 00:00:00 UTC)),
            warnings: vec![CleanupWarning {
                id: "01HBAD".to_string(),
                issue: CleanupIssue::Conflict,
            }],
        };

        let json = serde_json::to_string(&preview).unwrap();
        assert_eq!(
            serde_json::from_str::<CleanupPreview>(&json).unwrap(),
            preview
        );
    }

    #[test]
    fn result_round_trips_as_json() {
        let result = CleanupResult {
            requested: 42,
            deleted: 41,
            missing: 1,
            errors: vec![CleanupError {
                id: "01HBAD".to_string(),
                issue: CleanupIssue::Symlink,
            }],
        };

        let json = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serde_json::from_str::<CleanupResult>(&json).unwrap(),
            result
        );
    }
}
