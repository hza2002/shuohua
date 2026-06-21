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
