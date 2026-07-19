use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;
use std::collections::{HashMap, VecDeque};
use std::path::Path;

use crate::config::theme::TuiTheme;
use crate::history::{
    AggregateStats, AnalyticsPeriod, AnalyticsSnapshot, CleanupFilter, CleanupPreview,
    CleanupResult, CleanupScope, CleanupWindow, HistoryRecord, HistoryStatsSnapshot,
    HistoryStatsStatus,
};
use crate::ipc::protocol::{Command, Event};
use crate::tui::audio::{
    audio_info_for_record, missing_audio_info_for_record, open_audio_path, reveal_audio_path,
    AudioInfo,
};
use crate::tui::history::render::render_history;
use crate::tui::page::{KeyHint, KeyOutcome, MouseKind, Page};
use std::cell::RefCell;

mod render;

#[cfg(test)]
mod tests;

pub const HISTORY_PAGE_SIZE: usize = 50;
const AUTO_LOAD_MORE_REMAINING: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryDetail {
    Details,
    Asr,
    Pipeline,
    Sessions,
    Error,
    Json,
}

impl HistoryDetail {
    /// All detail views in tab order (h/l cycles through these, and the sub-tab
    /// bar renders them left to right).
    pub const ALL: [HistoryDetail; 6] = [
        Self::Details,
        Self::Asr,
        Self::Pipeline,
        Self::Sessions,
        Self::Error,
        Self::Json,
    ];

    pub fn next(self) -> Self {
        match self {
            Self::Details => Self::Asr,
            Self::Asr => Self::Pipeline,
            Self::Pipeline => Self::Sessions,
            Self::Sessions => Self::Error,
            Self::Error => Self::Json,
            Self::Json => Self::Details,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Details => Self::Json,
            Self::Asr => Self::Details,
            Self::Pipeline => Self::Asr,
            Self::Sessions => Self::Pipeline,
            Self::Error => Self::Sessions,
            Self::Json => Self::Error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirm {
    DeleteAudio { record_id: String },
    DeleteHistory { record_id: String },
}

/// Retained-audio cleanup modal. Independent from `confirm`/search: while `Some`,
/// keys are intercepted by `feed_cleanup_key` before global keybindings, and the
/// UI draws a centered popup over the History page.
///
/// The window is chosen in `Selecting` (inline, incl. editable "older than N days"
/// and a custom date range) **before** any scan runs — no switching mid-scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupMode {
    Selecting(CleanupSelect),
    Scanning {
        filter: CleanupFilter,
    },
    Preview {
        preview: CleanupPreview,
        confirm: CleanupConfirm,
    },
    Executing {
        preview: CleanupPreview,
    },
    Done {
        result: CleanupResult,
    },
    Failed {
        message: String,
    },
}

/// Time-window options on the selection screen. `OlderThan` carries an editable
/// day count; `Custom` carries an editable `[from, to]` date range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowChoice {
    LastHours,
    LastDays,
    OlderThan,
    Custom,
}

pub const WINDOW_CHOICES: [WindowChoice; 4] = [
    WindowChoice::LastHours,
    WindowChoice::LastDays,
    WindowChoice::OlderThan,
    WindowChoice::Custom,
];
const CLEANUP_HOUR_CHOICES: [u32; 6] = [1, 3, 6, 12, 18, 24];
const CLEANUP_DAY_CHOICES: [u32; 4] = [1, 3, 5, 7];
const CLEANUP_OLDER_DAY_CHOICES: [u32; 5] = [14, 30, 60, 90, 180];

/// Live selection state: the highlighted choice plus the editable parameters for
/// the `OlderThan` (days) and `Custom` (from/to + active field) rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupSelect {
    pub scope: CleanupScope,
    pub scope_active: bool,
    pub choice: WindowChoice,
    pub hour_index: usize,
    pub day_index: usize,
    pub older_index: usize,
    pub from: time::Date,
    pub to: time::Date,
    pub field: RangeField,
    pub custom_editing: bool,
}

impl CleanupSelect {
    fn new(records: &[HistoryRecord]) -> Self {
        let today = today_local();
        let (from, to) = cleanup_default_range(records, today);
        Self {
            scope: CleanupScope::AudioOnly,
            scope_active: false,
            choice: WindowChoice::OlderThan,
            hour_index: 0,
            day_index: 3,
            older_index: 1,
            from,
            to,
            field: RangeField::FromDay,
            custom_editing: false,
        }
    }

    fn toggle_scope(&mut self) {
        self.scope = match self.scope {
            CleanupScope::AudioOnly => CleanupScope::RecordAndAudio,
            CleanupScope::RecordAndAudio => CleanupScope::AudioOnly,
        };
    }

    fn window(&self) -> CleanupWindow {
        match self.choice {
            WindowChoice::LastHours => CleanupWindow::LastHours(self.hours()),
            WindowChoice::LastDays => CleanupWindow::LastDays(self.days()),
            WindowChoice::OlderThan => CleanupWindow::OlderThanDays(self.older_days()),
            WindowChoice::Custom => CleanupWindow::Range {
                from: self.from,
                to: self.to,
            },
        }
    }

    pub fn hours(&self) -> u32 {
        CLEANUP_HOUR_CHOICES[self.hour_index.min(CLEANUP_HOUR_CHOICES.len() - 1)]
    }

    pub fn days(&self) -> u32 {
        CLEANUP_DAY_CHOICES[self.day_index.min(CLEANUP_DAY_CHOICES.len() - 1)]
    }

    pub fn older_days(&self) -> u32 {
        CLEANUP_OLDER_DAY_CHOICES[self.older_index.min(CLEANUP_OLDER_DAY_CHOICES.len() - 1)]
    }

    fn move_choice(&mut self, forward: bool) {
        if self.scope_active {
            self.scope_active = false;
            self.choice = if forward {
                WINDOW_CHOICES[0]
            } else {
                WINDOW_CHOICES[WINDOW_CHOICES.len() - 1]
            };
            return;
        }
        let index = WINDOW_CHOICES
            .iter()
            .position(|c| *c == self.choice)
            .unwrap_or(0);
        if forward && index + 1 == WINDOW_CHOICES.len() {
            self.scope_active = true;
            return;
        }
        if !forward && index == 0 {
            self.scope_active = true;
            return;
        }
        let len = WINDOW_CHOICES.len();
        let next = if forward {
            (index + 1) % len
        } else {
            (index + len - 1) % len
        };
        self.choice = WINDOW_CHOICES[next];
    }

    /// `←/→` or `h/l` on an editable row: preset rows cycle allowed values;
    /// `Custom` moves the active date field. Returns whether the key was consumed.
    fn edit_horizontal(&mut self, forward: bool) -> bool {
        if self.scope_active {
            self.toggle_scope();
            return true;
        }
        match self.choice {
            WindowChoice::LastHours => {
                self.hour_index = step_index(self.hour_index, CLEANUP_HOUR_CHOICES.len(), forward);
                true
            }
            WindowChoice::LastDays => {
                self.day_index = step_index(self.day_index, CLEANUP_DAY_CHOICES.len(), forward);
                true
            }
            WindowChoice::OlderThan => {
                self.older_index =
                    step_index(self.older_index, CLEANUP_OLDER_DAY_CHOICES.len(), forward);
                true
            }
            WindowChoice::Custom => {
                if !self.custom_editing {
                    self.custom_editing = true;
                }
                self.field = if forward {
                    self.field.next()
                } else {
                    self.field.prev()
                };
                true
            }
        }
    }

    /// `↑/↓` on the `Custom` row adjusts the active date field. Returns whether
    /// the key was consumed (only on `Custom`; elsewhere it stays row navigation).
    fn edit_vertical(&mut self, forward: bool) -> bool {
        if self.choice != WindowChoice::Custom || !self.custom_editing {
            return false;
        }
        let (date, part) = (self.field.is_from(), self.field.part());
        if date {
            self.from = adjust_date(self.from, part, forward);
        } else {
            self.to = adjust_date(self.to, part, forward);
        }
        true
    }
}

fn step_index(current: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if forward {
        (current + 1) % len
    } else {
        (current + len - 1) % len
    }
}

fn cleanup_default_range(
    records: &[HistoryRecord],
    fallback: time::Date,
) -> (time::Date, time::Date) {
    let mut dates = records.iter().map(|record| record.started_at.date());
    let Some(first) = dates.next() else {
        return (
            fallback
                .checked_sub(time::Duration::days(30))
                .unwrap_or(fallback),
            fallback,
        );
    };
    dates.fold((first, first), |(min, max), date| {
        (min.min(date), max.max(date))
    })
}

/// One of the six editable fields of the custom date range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeField {
    FromYear,
    FromMonth,
    FromDay,
    ToYear,
    ToMonth,
    ToDay,
}

impl RangeField {
    const ORDER: [RangeField; 6] = [
        RangeField::FromYear,
        RangeField::FromMonth,
        RangeField::FromDay,
        RangeField::ToYear,
        RangeField::ToMonth,
        RangeField::ToDay,
    ];

    fn is_from(self) -> bool {
        matches!(self, Self::FromYear | Self::FromMonth | Self::FromDay)
    }

    fn part(self) -> DateField {
        match self {
            Self::FromYear | Self::ToYear => DateField::Year,
            Self::FromMonth | Self::ToMonth => DateField::Month,
            Self::FromDay | Self::ToDay => DateField::Day,
        }
    }

    fn prev(self) -> Self {
        let i = Self::ORDER.iter().position(|f| *f == self).unwrap_or(0);
        Self::ORDER[(i + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }

    fn next(self) -> Self {
        let i = Self::ORDER.iter().position(|f| *f == self).unwrap_or(0);
        Self::ORDER[(i + 1) % Self::ORDER.len()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateField {
    Year,
    Month,
    Day,
}

fn today_local() -> time::Date {
    time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
        .date()
}

/// Adjust one field of `date` by ±1, always yielding a valid date (day is
/// clamped to the target month's length; `time` guards year/month arithmetic).
fn adjust_date(date: time::Date, field: DateField, forward: bool) -> time::Date {
    match field {
        DateField::Day => {
            let delta = if forward { 1 } else { -1 };
            date.checked_add(time::Duration::days(delta))
                .unwrap_or(date)
        }
        DateField::Month => {
            let (year, month) = if forward {
                let m = date.month().next();
                let y = if date.month() == time::Month::December {
                    date.year() + 1
                } else {
                    date.year()
                };
                (y, m)
            } else {
                let m = date.month().previous();
                let y = if date.month() == time::Month::January {
                    date.year() - 1
                } else {
                    date.year()
                };
                (y, m)
            };
            with_ymd_clamped(year, month, date.day())
        }
        DateField::Year => {
            let year = if forward {
                date.year() + 1
            } else {
                date.year() - 1
            };
            with_ymd_clamped(year, date.month(), date.day())
        }
    }
}

fn with_ymd_clamped(year: i32, month: time::Month, day: u8) -> time::Date {
    let max = month.length(year);
    time::Date::from_calendar_date(year, month, day.min(max)).unwrap_or_else(|_| today_local())
}

/// Which button is focused on the preview screen. Defaults to `Cancel` so an
/// accidental Enter never deletes — the user must move to `Delete` first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupConfirm {
    Cancel,
    Delete,
}

impl CleanupConfirm {
    fn toggled(self) -> Self {
        match self {
            CleanupConfirm::Cancel => CleanupConfirm::Delete,
            CleanupConfirm::Delete => CleanupConfirm::Cancel,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HistoryRequestKind {
    RefreshPage { query: Option<String> },
    Search,
    LoadMore,
    Discard,
    DiscardRefresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchStats {
    pub query: String,
    pub matched: u64,
    pub stats: AggregateStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnalyticsRequestKind {
    Refresh,
    Standalone,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AnalyticsCacheKey {
    period: AnalyticsPeriod,
    anchor: String,
}

impl AnalyticsCacheKey {
    fn new(period: AnalyticsPeriod, anchor: impl Into<String>) -> Self {
        Self {
            period,
            anchor: anchor.into(),
        }
    }

    fn from_selection(selection: &AnalyticsSelection) -> Self {
        Self::new(selection.period, selection.anchor.clone())
    }

    fn from_snapshot(snapshot: &AnalyticsSnapshot) -> Self {
        Self::new(snapshot.period, snapshot.anchor.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryView {
    Records,
    Analytics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyticsMetric {
    Records,
    Words,
    Duration,
    AsrAudio,
}

impl AnalyticsMetric {
    fn next(self) -> Self {
        match self {
            Self::Records => Self::Words,
            Self::Words => Self::Duration,
            Self::Duration => Self::AsrAudio,
            Self::AsrAudio => Self::Records,
        }
    }
}

impl AnalyticsMetric {
    pub const ALL: [Self; 4] = [Self::Records, Self::Words, Self::Duration, Self::AsrAudio];

    fn index(self) -> usize {
        match self {
            Self::Records => 0,
            Self::Words => 1,
            Self::Duration => 2,
            Self::AsrAudio => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalyticsSelection {
    pub period: AnalyticsPeriod,
    pub anchor: String,
    pub metric: AnalyticsMetric,
    pub visible_metrics: [bool; 4],
}

impl Default for AnalyticsSelection {
    fn default() -> Self {
        Self {
            period: AnalyticsPeriod::Last30Days,
            anchor: current_anchor(AnalyticsPeriod::Last30Days),
            metric: AnalyticsMetric::Records,
            visible_metrics: [true; 4],
        }
    }
}

#[derive(Debug, Default)]
pub struct HistoryAnalyticsState {
    pub selection: AnalyticsSelection,
    pub snapshot: Option<AnalyticsSnapshot>,
    pub warning: Option<String>,
    cache: HashMap<AnalyticsCacheKey, AnalyticsSnapshot>,
}

/// Geometry of the currently rendered record list, captured each frame so a
/// mouse click can be mapped to the record under the cursor.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ListHit {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub first: usize,
}

#[derive(Debug)]
pub struct HistoryPage {
    pub records: Vec<HistoryRecord>,
    pub selected: usize,
    pub detail: HistoryDetail,
    pub search: String,
    pub searching: bool,
    pub audio_cache: HashMap<String, AudioInfo>,
    pub confirm: Option<Confirm>,
    pub cleanup: Option<CleanupMode>,
    pub loading_more: bool,
    pub has_more: bool,
    pub stats: Option<HistoryStatsSnapshot>,
    pub search_stats: Option<SearchStats>,
    pub view: HistoryView,
    pub analytics: HistoryAnalyticsState,
    pub initial_loaded: bool,
    pub refresh_needed: bool,
    pub refresh_in_flight: bool,
    refresh_responses_pending: u8,
    pending_history_requests: VecDeque<HistoryRequestKind>,
    pending_analytics_requests: VecDeque<AnalyticsRequestKind>,
    /// Record-list geometry from the last render; consumed by `on_mouse`.
    pub(crate) list_hit: RefCell<Option<ListHit>>,
    /// Detail sub-tab hit regions from the last render; consumed by `on_mouse`.
    pub(crate) detail_tabs: RefCell<Vec<(ratatui::layout::Rect, HistoryDetail)>>,
    /// Vertical scroll offset of the right detail pane; reset when the selected
    /// record or the detail view changes.
    pub detail_scroll: u16,
    /// Max detail scroll (content rows beyond the viewport), recomputed each
    /// render so scroll handlers can clamp.
    pub detail_max_scroll: std::cell::Cell<u16>,
    /// Detail pane rect from the last render; a wheel here scrolls the detail
    /// instead of moving the record selection.
    pub(crate) detail_hit: RefCell<Option<ratatui::layout::Rect>>,
}

impl HistoryPage {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            selected: 0,
            detail: HistoryDetail::Details,
            search: String::new(),
            searching: false,
            audio_cache: HashMap::new(),
            confirm: None,
            cleanup: None,
            loading_more: false,
            has_more: true,
            stats: None,
            search_stats: None,
            view: HistoryView::Records,
            analytics: HistoryAnalyticsState::default(),
            initial_loaded: false,
            refresh_needed: false,
            refresh_in_flight: false,
            refresh_responses_pending: 0,
            pending_history_requests: VecDeque::new(),
            pending_analytics_requests: VecDeque::new(),
            list_hit: RefCell::new(None),
            detail_tabs: RefCell::new(Vec::new()),
            detail_scroll: 0,
            detail_max_scroll: std::cell::Cell::new(0),
            detail_hit: RefCell::new(None),
        }
    }

    /// Click selects the record under the cursor; wheel scrolls the selection.
    pub fn on_mouse(&mut self, column: u16, row: u16, kind: MouseKind) -> KeyOutcome {
        // Wheel over the right detail pane scrolls its text; elsewhere it moves
        // the record selection.
        let over_detail = self.detail_hit.borrow().is_some_and(|r| {
            column >= r.x && column < r.x + r.width && row >= r.y && row < r.y + r.height
        });
        match kind {
            MouseKind::ScrollDown if over_detail => {
                self.scroll_detail(1);
                KeyOutcome::none()
            }
            MouseKind::ScrollUp if over_detail => {
                self.scroll_detail(-1);
                KeyOutcome::none()
            }
            MouseKind::ScrollDown => {
                self.move_down();
                self.auto_load_more_outcome()
            }
            MouseKind::ScrollUp => {
                self.move_up();
                KeyOutcome::none()
            }
            MouseKind::Down => {
                // A click on a detail sub-tab switches the view; tabs win over
                // the record list if regions ever overlap.
                let tab = self
                    .detail_tabs
                    .borrow()
                    .iter()
                    .find(|(rect, _)| {
                        column >= rect.x
                            && column < rect.x + rect.width
                            && row >= rect.y
                            && row < rect.y + rect.height
                    })
                    .map(|(_, detail)| *detail);
                if let Some(detail) = tab {
                    self.select_detail(detail);
                    return KeyOutcome::none();
                }
                let target = self.list_hit.borrow().and_then(|hit| {
                    let in_bounds = column >= hit.x
                        && column < hit.x + hit.width
                        && row >= hit.y
                        && row < hit.y + hit.height;
                    in_bounds.then(|| hit.first + (row - hit.y) as usize)
                });
                if let Some(idx) = target {
                    if idx < self.filtered().len() {
                        self.selected = idx;
                        self.detail_scroll = 0;
                    }
                }
                KeyOutcome::none()
            }
        }
    }

    /// The visible record set. The daemon already applies `query` filtering
    /// server-side (identical field/substring logic), so the loaded page IS the
    /// match set — we do not re-filter locally.
    pub fn filtered(&self) -> Vec<&HistoryRecord> {
        self.records.iter().collect()
    }

    pub fn selected_record(&self) -> Option<&HistoryRecord> {
        self.filtered().get(self.selected).copied()
    }

    pub fn audio_info_for_record(&self, record: &HistoryRecord) -> AudioInfo {
        self.audio_cache
            .get(&record.id)
            .cloned()
            .unwrap_or_else(|| missing_audio_info_for_record(record))
    }

    pub fn move_down(&mut self) {
        let len = self.filtered().len();
        if len > 0 {
            self.selected = (self.selected + 1).min(len - 1);
        }
        self.detail_scroll = 0;
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.detail_scroll = 0;
    }

    pub fn move_top(&mut self) {
        self.selected = 0;
        self.detail_scroll = 0;
    }

    pub fn move_bottom(&mut self) {
        let len = self.filtered().len();
        self.selected = len.saturating_sub(1);
        self.detail_scroll = 0;
    }

    pub fn next_detail(&mut self) {
        self.detail = self.detail.next();
        self.detail_scroll = 0;
    }

    pub fn prev_detail(&mut self) {
        self.detail = self.detail.prev();
        self.detail_scroll = 0;
    }

    pub fn select_detail(&mut self, detail: HistoryDetail) {
        self.detail = detail;
        self.detail_scroll = 0;
    }

    /// Scroll the right detail pane, clamped to the content height recorded at
    /// the last render.
    pub fn scroll_detail(&mut self, delta: i32) {
        let max = self.detail_max_scroll.get();
        let next = (self.detail_scroll as i32 + delta).clamp(0, max as i32);
        self.detail_scroll = next as u16;
    }

    pub fn start_search(&mut self) {
        if self.view == HistoryView::Analytics {
            return;
        }
        self.searching = true;
    }

    pub fn cancel_search(&mut self) {
        self.searching = false;
    }

    pub fn clear_search(&mut self) {
        self.search.clear();
        self.searching = false;
        self.selected = 0;
        self.confirm = None;
        self.reset_search_paging();
    }

    pub fn search_char(&mut self, ch: char) {
        self.search.push(ch);
        self.selected = 0;
        self.reset_search_paging();
    }

    pub fn search_backspace(&mut self) {
        self.search.pop();
        self.selected = 0;
        self.reset_search_paging();
    }

    fn reset_search_paging(&mut self) {
        self.loading_more = false;
        self.has_more = true;
        self.search_stats = None;
        let query = self.query();
        for request in &mut self.pending_history_requests {
            if matches!(
                request,
                HistoryRequestKind::Search | HistoryRequestKind::LoadMore
            ) {
                *request = HistoryRequestKind::Discard;
            }
            if matches!(
                request,
                HistoryRequestKind::RefreshPage { query: request_query } if request_query != &query
            ) {
                *request = HistoryRequestKind::DiscardRefresh;
            }
        }
    }

    pub fn enter_commands(&mut self) -> Vec<Command> {
        if self.initial_loaded || self.refresh_in_flight {
            return Vec::new();
        }
        self.refresh_in_flight = true;
        self.refresh_responses_pending = self.refresh_response_count();
        self.refresh_needed = false;
        self.initial_loaded = true;
        self.pending_history_requests
            .push_back(HistoryRequestKind::RefreshPage {
                query: self.query(),
            });
        self.refresh_batch_commands()
    }

    pub fn refresh_commands(&mut self) -> Vec<Command> {
        if !self.refresh_needed || self.refresh_in_flight {
            return Vec::new();
        }
        self.refresh_in_flight = true;
        self.refresh_responses_pending = self.refresh_response_count();
        self.refresh_needed = false;
        self.pending_history_requests
            .push_back(HistoryRequestKind::RefreshPage {
                query: self.query(),
            });
        self.refresh_batch_commands()
    }

    fn refresh_batch_commands(&self) -> Vec<Command> {
        let mut commands = vec![self.first_page_command(), Command::GetHistoryStats];
        if self.view == HistoryView::Analytics {
            commands.push(self.analytics_command());
        }
        commands
    }

    fn refresh_response_count(&mut self) -> u8 {
        if self.view == HistoryView::Analytics {
            self.pending_analytics_requests
                .push_back(AnalyticsRequestKind::Refresh);
            3
        } else {
            2
        }
    }

    fn first_page_command(&self) -> Command {
        Command::GetHistory {
            limit: HISTORY_PAGE_SIZE,
            before: None,
            before_id: None,
            query: self.query(),
        }
    }

    fn query(&self) -> Option<String> {
        let query = self.search.trim();
        (!query.is_empty()).then(|| query.to_string())
    }

    fn analytics_command(&self) -> Command {
        Command::GetHistoryAnalytics {
            period: self.analytics.selection.period,
            anchor: self.analytics.selection.anchor.clone(),
        }
    }

    fn restore_cached_analytics(&mut self) -> bool {
        let key = AnalyticsCacheKey::from_selection(&self.analytics.selection);
        let Some(snapshot) = self.analytics.cache.get(&key).cloned() else {
            return false;
        };
        self.analytics.snapshot = Some(snapshot);
        self.analytics.warning = None;
        true
    }

    fn analytics_selection_outcome(&mut self) -> KeyOutcome {
        if self.restore_cached_analytics() {
            return KeyOutcome::none();
        }
        self.pending_analytics_requests
            .push_back(AnalyticsRequestKind::Standalone);
        KeyOutcome::command_and_status(
            self.analytics_command(),
            crate::t!("tui.history.analytics.loading"),
        )
    }

    fn search_command(&self) -> Command {
        self.first_page_command()
    }

    fn search_outcome(&mut self) -> KeyOutcome {
        self.pending_history_requests
            .push_back(HistoryRequestKind::Search);
        KeyOutcome::command_and_status(self.search_command(), crate::t!("tui.history.searching"))
    }

    fn copy_text_outcome(&self) -> KeyOutcome {
        let Some(text) = self.selected_record().map(|record| record.text.clone()) else {
            return KeyOutcome::none();
        };
        match crate::platform::clipboard::write_string(&text) {
            Ok(()) => KeyOutcome::status(crate::t!("tui.history.copy.final_ok")),
            Err(e) => KeyOutcome::status(crate::i18n::tr(
                "tui.error.clipboard",
                &[("error", e.to_string())],
            )),
        }
    }

    fn copy_asr_outcome(&self) -> KeyOutcome {
        let Some(text) = self.selected_record().map(|record| record.asr.text.clone()) else {
            return KeyOutcome::none();
        };
        match crate::platform::clipboard::write_string(&text) {
            Ok(()) => KeyOutcome::status(crate::t!("tui.history.copy.asr_ok")),
            Err(e) => KeyOutcome::status(crate::i18n::tr(
                "tui.error.clipboard",
                &[("error", e.to_string())],
            )),
        }
    }

    pub fn open_selected_audio(&self) -> String {
        self.run_audio_action(open_audio_path, "tui.history.audio.opening")
    }

    pub fn reveal_selected_audio(&self) -> String {
        self.run_audio_action(reveal_audio_path, "tui.history.audio.revealing")
    }

    pub fn request_delete_audio(&mut self) -> String {
        let Some(record_id) = self.selected_record().map(|record| record.id.clone()) else {
            return crate::t!("tui.no_history_selected");
        };
        self.confirm = Some(Confirm::DeleteAudio { record_id });
        crate::t!("tui.confirm.delete_audio")
    }

    pub fn request_delete_history(&mut self) -> String {
        let Some(record_id) = self.selected_record().map(|record| record.id.clone()) else {
            return crate::t!("tui.no_history_selected");
        };
        self.confirm = Some(Confirm::DeleteHistory { record_id });
        crate::t!("tui.confirm.delete_history")
    }

    pub fn feed_confirm_key(&mut self, key: KeyEvent) -> Option<KeyOutcome> {
        if key.kind != KeyEventKind::Press || self.confirm.is_none() {
            return None;
        }
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => Some(self.confirm_yes()),
            KeyCode::Char('n') | KeyCode::Esc => {
                self.confirm = None;
                Some(KeyOutcome::status(crate::t!("tui.confirm.cancelled")))
            }
            _ => Some(KeyOutcome::none()),
        }
    }

    fn confirm_yes(&mut self) -> KeyOutcome {
        let Some(confirm) = self.confirm.take() else {
            return KeyOutcome::none();
        };
        match confirm {
            Confirm::DeleteAudio { record_id } => KeyOutcome::command_and_status(
                Command::DeleteAudio { id: record_id },
                crate::t!("tui.history.audio.delete_requested"),
            ),
            Confirm::DeleteHistory { record_id } => KeyOutcome::command_and_status(
                Command::DeleteHistory { id: record_id },
                crate::t!("tui.history.delete_requested"),
            ),
        }
    }

    /// Open the cleanup modal at the age-selection step. No scan is issued yet.
    fn open_cleanup(&mut self) -> KeyOutcome {
        self.cleanup = Some(CleanupMode::Selecting(CleanupSelect::new(&self.records)));
        KeyOutcome::status(crate::t!("tui.history.cleanup.select"))
    }

    /// Modal key handling while the cleanup popup is open. Returns `None` when the
    /// popup is closed so the caller falls through to normal key routing.
    pub fn feed_cleanup_key(&mut self, key: KeyEvent) -> Option<KeyOutcome> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        let outcome = match self.cleanup.as_ref()? {
            CleanupMode::Done { .. } | CleanupMode::Failed { .. } => {
                // Terminal states: any key dismisses the popup.
                self.cleanup = None;
                KeyOutcome::none()
            }
            CleanupMode::Selecting { .. } => self.selecting_key(key),
            CleanupMode::Preview { .. } => self.preview_key(key),
            CleanupMode::Scanning { .. } | CleanupMode::Executing { .. } => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('n')) {
                    self.cleanup = None;
                    KeyOutcome::status(crate::t!("tui.history.cleanup.cancelled"))
                } else {
                    KeyOutcome::none()
                }
            }
        };
        Some(outcome)
    }

    fn selecting_key(&mut self, key: KeyEvent) -> KeyOutcome {
        let Some(CleanupMode::Selecting(select)) = self.cleanup.as_ref() else {
            return KeyOutcome::none();
        };
        // `CleanupSelect` is `Copy`; edit the copy, then write it back. This frees
        // the borrow so Esc/Enter can touch `self`.
        let mut select = *select;
        // Editing keys are contextual to the highlighted row. Preset rows use h/l
        // for discrete values. Custom range has a lightweight edit mode: Enter or
        // h/l enters it, h/l moves fields, and j/k adjusts the active field.
        match key.code {
            KeyCode::Esc if select.custom_editing => {
                select.custom_editing = false;
            }
            KeyCode::Esc | KeyCode::Char('n') => {
                self.cleanup = None;
                return KeyOutcome::status(crate::t!("tui.history.cleanup.cancelled"));
            }
            KeyCode::Enter => {
                if !select.scope_active
                    && select.choice == WindowChoice::Custom
                    && !select.custom_editing
                {
                    select.custom_editing = true;
                } else {
                    return self.start_scan(select.window());
                }
            }
            KeyCode::Char('j') if select.custom_editing => {
                select.edit_vertical(false);
            }
            KeyCode::Char('k') if select.custom_editing => {
                select.edit_vertical(true);
            }
            KeyCode::Char('j') => select.move_choice(true),
            KeyCode::Char('k') => select.move_choice(false),
            KeyCode::Left | KeyCode::Char('h') => {
                select.edit_horizontal(false);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                select.edit_horizontal(true);
            }
            KeyCode::Up => {
                if !select.edit_vertical(true) {
                    select.move_choice(false);
                }
            }
            KeyCode::Down => {
                if !select.edit_vertical(false) {
                    select.move_choice(true);
                }
            }
            _ => return KeyOutcome::none(),
        }
        self.cleanup = Some(CleanupMode::Selecting(select));
        KeyOutcome::none()
    }

    fn preview_key(&mut self, key: KeyEvent) -> KeyOutcome {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') => {
                self.cleanup = None;
                KeyOutcome::status(crate::t!("tui.history.cleanup.cancelled"))
            }
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l')
            | KeyCode::Tab => {
                if let Some(CleanupMode::Preview { confirm, .. }) = &mut self.cleanup {
                    *confirm = confirm.toggled();
                }
                KeyOutcome::none()
            }
            KeyCode::Enter => self.confirm_cleanup_preview(),
            _ => KeyOutcome::none(),
        }
    }

    fn start_scan(&mut self, window: CleanupWindow) -> KeyOutcome {
        let scope = self
            .cleanup
            .as_ref()
            .and_then(|mode| match mode {
                CleanupMode::Selecting(select) => Some(select.scope),
                _ => None,
            })
            .unwrap_or(CleanupScope::AudioOnly);
        let filter = CleanupFilter { scope, window };
        self.cleanup = Some(CleanupMode::Scanning { filter });
        KeyOutcome::command_and_status(
            Command::PreviewHistoryCleanup { filter },
            crate::t!("tui.history.cleanup.loading"),
        )
    }

    /// Act on the focused preview button: Delete executes, Cancel closes.
    fn confirm_cleanup_preview(&mut self) -> KeyOutcome {
        let Some(CleanupMode::Preview { preview, confirm }) = &self.cleanup else {
            return KeyOutcome::none();
        };
        if *confirm == CleanupConfirm::Cancel {
            self.cleanup = None;
            return KeyOutcome::status(crate::t!("tui.history.cleanup.cancelled"));
        }
        if preview.ids.is_empty() {
            self.cleanup = None;
            return KeyOutcome::status(crate::t!("tui.history.cleanup.empty"));
        }
        let preview = preview.clone();
        let command = Command::ExecuteHistoryCleanup {
            filter: preview.filter,
            ids: preview.ids.clone(),
        };
        self.cleanup = Some(CleanupMode::Executing { preview });
        KeyOutcome::command_and_status(command, crate::t!("tui.history.cleanup.executing"))
    }

    fn mark_cleanup_audio_missing(&mut self, ids: &[String]) {
        for id in ids {
            if let Some(record) = self.records.iter().find(|record| &record.id == id).cloned() {
                self.audio_cache
                    .insert(record.id.clone(), missing_audio_info_for_record(&record));
            }
        }
    }

    fn remove_cleanup_records(&mut self, ids: &[String]) {
        let ids: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
        self.records
            .retain(|record| !ids.contains(record.id.as_str()));
        self.audio_cache.retain(|id, _| !ids.contains(id.as_str()));
        if self.selected >= self.records.len() {
            self.selected = self.records.len().saturating_sub(1);
        }
    }

    fn run_audio_action(&self, action: fn(&Path) -> Result<()>, status_key: &str) -> String {
        let Some(record) = self.selected_record() else {
            return crate::t!("tui.no_history_selected");
        };
        let info = self.audio_info_for_record(record);
        if !info.exists() {
            return crate::t!("tui.history.audio.missing_status");
        }
        match action(&info.path) {
            Ok(()) => crate::i18n::tr(status_key, &[("path", info.path.display().to_string())]),
            Err(e) => crate::t!("tui.error.audio_action", error = e),
        }
    }

    fn refresh_audio_cache(&mut self) {
        self.audio_cache.clear();
        let records = self.records.clone();
        for record in &records {
            self.refresh_audio_cache_for_record(record);
        }
    }

    fn refresh_audio_cache_for_record(&mut self, record: &HistoryRecord) {
        self.audio_cache
            .insert(record.id.clone(), audio_info_for_record(record));
    }

    pub fn load_more_outcome(&mut self) -> KeyOutcome {
        if self.refresh_in_flight
            || self
                .pending_history_requests
                .iter()
                .any(|request| matches!(request, HistoryRequestKind::RefreshPage { .. }))
        {
            return KeyOutcome::status(crate::t!("tui.history.loading_more"));
        }
        if self
            .pending_history_requests
            .iter()
            .any(|request| matches!(request, HistoryRequestKind::Search))
        {
            return KeyOutcome::status(crate::t!("tui.history.searching"));
        }
        if self.loading_more {
            return KeyOutcome::status(crate::t!("tui.history.loading_more"));
        }
        if !self.has_more {
            return KeyOutcome::status(crate::t!("tui.history.no_more"));
        }
        let Some(oldest) = self.records.last() else {
            self.loading_more = true;
            self.pending_history_requests
                .push_back(HistoryRequestKind::LoadMore);
            return KeyOutcome::command_and_status(
                Command::GetHistory {
                    limit: HISTORY_PAGE_SIZE,
                    before: None,
                    before_id: None,
                    query: self.query(),
                },
                crate::t!("tui.history.loading_more"),
            );
        };
        self.loading_more = true;
        self.pending_history_requests
            .push_back(HistoryRequestKind::LoadMore);
        KeyOutcome::command_and_status(
            Command::GetHistory {
                limit: HISTORY_PAGE_SIZE,
                before: Some(format_rfc3339(oldest.started_at)),
                before_id: Some(oldest.id.clone()),
                query: self.query(),
            },
            crate::t!("tui.history.loading_more"),
        )
    }

    fn auto_load_more_outcome(&mut self) -> KeyOutcome {
        if self.view != HistoryView::Records {
            return KeyOutcome::none();
        }
        let len = self.filtered().len();
        if len == 0 || self.selected + AUTO_LOAD_MORE_REMAINING < len {
            return KeyOutcome::none();
        }
        let mut outcome = self.load_more_outcome();
        if outcome.command.is_some() {
            outcome.status = None;
        }
        outcome
    }

    fn merge_newest(&mut self, records: Vec<HistoryRecord>) {
        self.records.clear();
        for record in records {
            self.insert_unique(record, true);
        }
        self.selected = 0;
        self.has_more = self.records.len() >= HISTORY_PAGE_SIZE;
    }

    fn merge_older(&mut self, records: Vec<HistoryRecord>) {
        self.loading_more = false;
        self.has_more = records.len() >= HISTORY_PAGE_SIZE;
        for record in records {
            self.insert_unique(record, true);
        }
        self.selected = self.selected.min(self.records.len().saturating_sub(1));
    }

    fn insert_unique(&mut self, record: HistoryRecord, append: bool) {
        if self.records.iter().any(|existing| existing.id == record.id) {
            return;
        }
        self.refresh_audio_cache_for_record(&record);
        if append {
            self.records.push(record);
        } else {
            self.records.insert(0, record);
        }
    }

    fn toggle_view(&mut self) -> KeyOutcome {
        self.view = match self.view {
            HistoryView::Records => HistoryView::Analytics,
            HistoryView::Analytics => HistoryView::Records,
        };
        if self.view == HistoryView::Analytics {
            return self.analytics_selection_outcome();
        }
        KeyOutcome::none()
    }

    fn next_period_outcome(&mut self) -> KeyOutcome {
        self.analytics.selection.period = match self.analytics.selection.period {
            AnalyticsPeriod::Last30Days => AnalyticsPeriod::Last7Days,
            AnalyticsPeriod::Last7Days => AnalyticsPeriod::Month,
            AnalyticsPeriod::Month => AnalyticsPeriod::Year,
            AnalyticsPeriod::Year => AnalyticsPeriod::Day,
            AnalyticsPeriod::Day => AnalyticsPeriod::Last30Days,
        };
        self.analytics.selection.anchor = current_anchor(self.analytics.selection.period);
        self.analytics_selection_outcome()
    }

    fn shift_anchor_outcome(&mut self, delta: i32) -> KeyOutcome {
        self.analytics.selection.anchor = shift_anchor(
            self.analytics.selection.period,
            &self.analytics.selection.anchor,
            delta,
        );
        self.analytics_selection_outcome()
    }

    fn next_metric(&mut self) {
        self.analytics.selection.metric = self.analytics.selection.metric.next();
    }

    fn toggle_analytics_metric(&mut self, metric: AnalyticsMetric) {
        let index = metric.index();
        let visible = &mut self.analytics.selection.visible_metrics;
        if visible[index] && visible.iter().filter(|shown| **shown).count() == 1 {
            return;
        }
        visible[index] = !visible[index];
        if !visible[self.analytics.selection.metric.index()] {
            self.analytics.selection.metric = AnalyticsMetric::ALL
                .into_iter()
                .find(|metric| visible[metric.index()])
                .unwrap_or(AnalyticsMetric::Records);
        }
    }

    fn mark_refresh_response(&mut self) {
        self.refresh_responses_pending = self.refresh_responses_pending.saturating_sub(1);
        self.refresh_in_flight = self.refresh_responses_pending > 0;
    }

    fn finish_history_request_with_records(
        &mut self,
        records: Vec<HistoryRecord>,
        matched: Option<u64>,
        stats: Option<AggregateStats>,
    ) {
        match self.pending_history_requests.pop_front() {
            Some(HistoryRequestKind::LoadMore) => {
                self.update_search_stats(matched, stats);
                self.merge_older(records);
            }
            Some(HistoryRequestKind::Search) => {
                self.update_search_stats(matched, stats);
                self.merge_newest(records);
            }
            Some(HistoryRequestKind::RefreshPage { query }) => {
                if query != self.query() {
                    self.mark_refresh_response();
                    return;
                }
                self.update_search_stats(matched, stats);
                self.merge_newest(records);
                self.refresh_audio_cache();
                self.mark_refresh_response();
            }
            Some(HistoryRequestKind::DiscardRefresh) => self.mark_refresh_response(),
            Some(HistoryRequestKind::Discard) => {}
            None => {}
        }
    }

    fn update_search_stats(&mut self, matched: Option<u64>, stats: Option<AggregateStats>) {
        let Some(query) = self.query() else {
            self.search_stats = None;
            return;
        };
        if let (Some(matched), Some(stats)) = (matched, stats) {
            self.search_stats = Some(SearchStats {
                query,
                matched,
                stats,
            });
        }
    }

    fn finish_history_error(&mut self, kind: &str) {
        match kind {
            "history_read" => match self.pending_history_requests.pop_front() {
                Some(
                    HistoryRequestKind::RefreshPage { .. } | HistoryRequestKind::DiscardRefresh,
                ) => self.mark_refresh_response(),
                Some(HistoryRequestKind::LoadMore) => self.loading_more = false,
                Some(HistoryRequestKind::Search | HistoryRequestKind::Discard) | None => {}
            },
            "history_stats" => self.mark_refresh_response(),
            "history_analytics" => {
                if matches!(
                    self.pending_analytics_requests.pop_front(),
                    Some(AnalyticsRequestKind::Refresh)
                ) {
                    self.mark_refresh_response();
                }
            }
            _ => {}
        }
    }

    fn finish_analytics_response(&mut self, snapshot: &AnalyticsSnapshot) {
        if snapshot.status == HistoryStatsStatus::Ready {
            self.analytics
                .cache
                .insert(AnalyticsCacheKey::from_snapshot(snapshot), snapshot.clone());
        }
        let refresh_response = matches!(
            self.pending_analytics_requests.pop_front(),
            Some(AnalyticsRequestKind::Refresh)
        );
        if snapshot.period != self.analytics.selection.period
            || snapshot.anchor != self.analytics.selection.anchor
        {
            if refresh_response {
                self.mark_refresh_response();
            }
            return;
        }
        match snapshot.status {
            HistoryStatsStatus::Ready => {
                self.analytics.snapshot = Some(snapshot.clone());
                self.analytics.warning = None;
            }
            HistoryStatsStatus::Stale => {
                if self.analytics.snapshot.is_none() {
                    self.analytics.snapshot = Some(snapshot.clone());
                }
                self.analytics.warning = Some(
                    snapshot
                        .error
                        .clone()
                        .unwrap_or_else(|| crate::t!("tui.history.analytics.stale")),
                );
            }
            HistoryStatsStatus::Unavailable => {
                self.analytics.warning = Some(
                    snapshot
                        .error
                        .clone()
                        .unwrap_or_else(|| crate::t!("tui.history.analytics.unavailable")),
                );
            }
        }
        if refresh_response {
            self.mark_refresh_response();
        }
    }
}

impl Page for HistoryPage {
    fn apply_event(&mut self, event: &Event, _active: bool) {
        match event {
            Event::HistoryAppended { record } => {
                self.analytics.cache.clear();
                let selected_id = self.selected_record().map(|record| record.id.clone());
                let was_empty = self.records.is_empty();
                let inserted = !self.records.iter().any(|existing| existing.id == record.id);
                self.insert_unique((**record).clone(), false);
                if let Some(selected_id) = selected_id {
                    if let Some(position) = self
                        .filtered()
                        .iter()
                        .position(|record| record.id == selected_id)
                    {
                        self.selected = position;
                    }
                } else if inserted && !was_empty {
                    self.selected += 1;
                }
                self.selected = self.selected.min(self.records.len().saturating_sub(1));
            }
            Event::History {
                records,
                matched,
                stats,
            } => {
                self.finish_history_request_with_records(records.clone(), *matched, *stats);
            }
            Event::HistoryStats { snapshot } => {
                self.stats = Some(snapshot.clone());
                self.mark_refresh_response();
            }
            Event::HistoryAnalytics { snapshot } => {
                self.finish_analytics_response(snapshot);
            }
            Event::HistoryCleanupPreview { preview } => {
                // Only apply if it echoes the filter we are currently scanning,
                // so a stale preview from a superseded request is dropped.
                if let Some(CleanupMode::Scanning { filter }) = &self.cleanup {
                    if *filter == preview.filter {
                        self.cleanup = Some(CleanupMode::Preview {
                            preview: preview.clone(),
                            confirm: CleanupConfirm::Cancel,
                        });
                    }
                }
            }
            Event::HistoryCleanupDone { result } => {
                let executed = match &self.cleanup {
                    Some(CleanupMode::Executing { preview }) => {
                        Some((preview.filter.scope, preview.ids.clone()))
                    }
                    _ => None,
                };
                if let Some((scope, ids)) = executed {
                    match scope {
                        CleanupScope::AudioOnly => self.mark_cleanup_audio_missing(&ids),
                        CleanupScope::RecordAndAudio => self.remove_cleanup_records(&ids),
                    }
                    self.cleanup = Some(CleanupMode::Done {
                        result: result.clone(),
                    });
                }
            }
            Event::Error { kind, msg, .. } => {
                if kind == "history_cleanup" {
                    if self.cleanup.is_some() {
                        self.cleanup = Some(CleanupMode::Failed {
                            message: msg.clone(),
                        });
                    }
                } else {
                    self.finish_history_error(kind);
                }
            }
            Event::AudioDeleted { id, .. } => {
                if let Some(record) = self.records.iter().find(|record| &record.id == id) {
                    self.audio_cache
                        .insert(record.id.clone(), missing_audio_info_for_record(record));
                }
            }
            Event::HistoryDeleted { .. } => {
                self.analytics.cache.clear();
            }
            Event::HistoryChanged if !self.refresh_needed => {
                self.analytics.cache.clear();
                self.refresh_needed = true;
            }
            _ => {}
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> KeyOutcome {
        if key.kind != KeyEventKind::Press {
            return KeyOutcome::none();
        }
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.clear_search();
                    return self.search_outcome();
                }
                KeyCode::Enter => self.cancel_search(),
                KeyCode::Backspace => {
                    self.search_backspace();
                    return self.search_outcome();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.clear_search();
                    return self.search_outcome();
                }
                KeyCode::Char(ch) => {
                    self.search_char(ch);
                    return self.search_outcome();
                }
                _ => {}
            }
            return KeyOutcome::none();
        }
        match key.code {
            KeyCode::Char('s') => return self.toggle_view(),
            KeyCode::Char('m') if self.view == HistoryView::Records => {
                return self.load_more_outcome()
            }
            KeyCode::Char('p') if self.view == HistoryView::Analytics => {
                return self.next_period_outcome();
            }
            KeyCode::Char('[') if self.view == HistoryView::Analytics => {
                return self.shift_anchor_outcome(-1);
            }
            KeyCode::Char(']') if self.view == HistoryView::Analytics => {
                return self.shift_anchor_outcome(1);
            }
            KeyCode::Char('v') if self.view == HistoryView::Analytics => self.next_metric(),
            KeyCode::Char('R' | 'r') if self.view == HistoryView::Analytics => {
                self.toggle_analytics_metric(AnalyticsMetric::Records)
            }
            KeyCode::Char('W' | 'w') if self.view == HistoryView::Analytics => {
                self.toggle_analytics_metric(AnalyticsMetric::Words)
            }
            KeyCode::Char('D' | 'd') if self.view == HistoryView::Analytics => {
                self.toggle_analytics_metric(AnalyticsMetric::Duration)
            }
            KeyCode::Char('A' | 'a') if self.view == HistoryView::Analytics => {
                self.toggle_analytics_metric(AnalyticsMetric::AsrAudio)
            }
            KeyCode::Char('C') if self.view == HistoryView::Records => return self.open_cleanup(),
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_down();
                return self.auto_load_more_outcome();
            }
            KeyCode::Char('k') | KeyCode::Up => self.move_up(),
            KeyCode::Char('g') => self.move_top(),
            KeyCode::Char('G') => {
                self.move_bottom();
                return self.auto_load_more_outcome();
            }
            KeyCode::Char('l') | KeyCode::Right => self.next_detail(),
            KeyCode::Char('h') | KeyCode::Left => self.prev_detail(),
            // Scroll the right detail pane when its content overflows.
            KeyCode::PageDown => self.scroll_detail(4),
            KeyCode::PageUp => self.scroll_detail(-4),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll_detail(4)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll_detail(-4)
            }
            KeyCode::Esc => {
                if !self.search.is_empty() {
                    self.clear_search();
                    return self.search_outcome();
                }
                self.clear_search();
            }
            KeyCode::Enter | KeyCode::Char('y') => return self.copy_text_outcome(),
            KeyCode::Char('Y') => return self.copy_asr_outcome(),
            KeyCode::Char('o') => return KeyOutcome::status(self.open_selected_audio()),
            KeyCode::Char('r') => return KeyOutcome::status(self.reveal_selected_audio()),
            KeyCode::Char('d') => return KeyOutcome::status(self.request_delete_audio()),
            KeyCode::Char('x') => return KeyOutcome::status(self.request_delete_history()),
            _ => {}
        }
        KeyOutcome::none()
    }

    // Keep in sync with `on_key` above.
    fn key_hints(&self) -> Vec<KeyHint> {
        if self.confirm.is_some() {
            return vec![KeyHint::new("y/n", "tui.hint.confirm")];
        }
        if let Some(mode) = &self.cleanup {
            return match mode {
                CleanupMode::Selecting(select) => {
                    let mut hints = vec![KeyHint::new("j/k", "tui.hint.cleanup_age")];
                    if select.custom_editing {
                        hints = vec![
                            KeyHint::new("h/l", "tui.hint.cleanup_field"),
                            KeyHint::new("j/k", "tui.hint.cleanup_adjust"),
                            KeyHint::new("Enter", "tui.hint.cleanup_scan"),
                            KeyHint::new("Esc", "tui.hint.cleanup_close"),
                        ];
                    } else if select.scope_active {
                        hints.push(KeyHint::new("h/l", "tui.hint.cleanup_adjust"));
                    } else {
                        match select.choice {
                            WindowChoice::LastHours
                            | WindowChoice::LastDays
                            | WindowChoice::OlderThan => {
                                hints.push(KeyHint::new("h/l", "tui.hint.cleanup_adjust"))
                            }
                            WindowChoice::Custom => {
                                hints.push(KeyHint::new("h/l", "tui.hint.cleanup_field"));
                                hints.push(KeyHint::new("Enter", "tui.hint.cleanup_edit"));
                            }
                        }
                    }
                    if !select.custom_editing && select.choice != WindowChoice::Custom {
                        hints.push(KeyHint::new("Enter", "tui.hint.cleanup_scan"));
                        hints.push(KeyHint::new("Esc/n", "tui.hint.cleanup_cancel"));
                    } else if !select.custom_editing && select.choice == WindowChoice::Custom {
                        hints.push(KeyHint::new("Esc/n", "tui.hint.cleanup_cancel"));
                    }
                    hints
                }
                CleanupMode::Preview { .. } => vec![
                    KeyHint::new("h/l", "tui.hint.cleanup_choose"),
                    KeyHint::new("Enter", "tui.hint.cleanup_confirm"),
                    KeyHint::new("Esc", "tui.hint.cleanup_cancel"),
                ],
                CleanupMode::Scanning { .. } | CleanupMode::Executing { .. } => {
                    vec![KeyHint::new("Esc/n", "tui.hint.cleanup_cancel")]
                }
                CleanupMode::Done { .. } | CleanupMode::Failed { .. } => {
                    vec![KeyHint::new("Enter/Esc", "tui.hint.cleanup_close")]
                }
            };
        }
        if self.searching {
            return vec![
                KeyHint::new("Enter", "tui.hint.search_done"),
                KeyHint::new("Esc", "tui.hint.search_clear"),
            ];
        }
        match self.view {
            HistoryView::Records => {
                let mut hints = vec![
                    KeyHint::new("/", "tui.hint.search"),
                    KeyHint::new("j/k", "tui.hint.move"),
                    KeyHint::new("h/l", "tui.hint.detail"),
                ];
                // Surface the scroll keys only when the detail overflows.
                if self.detail_max_scroll.get() > 0 {
                    hints.push(KeyHint::new("PgUp/PgDn", "tui.hint.scroll"));
                }
                hints.extend([
                    KeyHint::new("Enter", "tui.hint.copy"),
                    KeyHint::new("Y", "tui.hint.copy_asr"),
                    KeyHint::new("o", "tui.hint.open_audio"),
                    KeyHint::new("r", "tui.hint.reveal"),
                    KeyHint::new("d", "tui.hint.del_audio"),
                    KeyHint::new("x", "tui.hint.del_history"),
                    KeyHint::new("C", "tui.hint.cleanup"),
                    KeyHint::new("m", "tui.hint.more"),
                    KeyHint::new("s", "tui.hint.analytics"),
                ]);
                hints
            }
            HistoryView::Analytics => vec![
                KeyHint::new("s", "tui.hint.records"),
                KeyHint::new("p", "tui.hint.period"),
                KeyHint::new("[ ]", "tui.hint.anchor"),
                KeyHint::new("v", "tui.hint.metric"),
                KeyHint::new("R/W/D/A", "tui.hint.metrics"),
            ],
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &TuiTheme, _footer_status: &str) {
        render_history(frame, self, area, theme);
    }
}

fn format_rfc3339(value: time::OffsetDateTime) -> String {
    value
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| value.to_string())
}

fn current_anchor(period: AnalyticsPeriod) -> String {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    match period {
        AnalyticsPeriod::Last7Days | AnalyticsPeriod::Last30Days => now
            .format(
                &time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
                    .expect("valid day format"),
            )
            .unwrap_or_else(|_| {
                format!(
                    "{:04}-{:02}-{:02}",
                    now.year(),
                    u8::from(now.month()),
                    now.day()
                )
            }),
        AnalyticsPeriod::Year => now
            .format(
                &time::format_description::parse_borrowed::<2>("[year]")
                    .expect("valid year format"),
            )
            .unwrap_or_else(|_| now.year().to_string()),
        AnalyticsPeriod::Month => now
            .format(
                &time::format_description::parse_borrowed::<2>("[year]-[month]")
                    .expect("valid month format"),
            )
            .unwrap_or_else(|_| format!("{:04}-{:02}", now.year(), u8::from(now.month()))),
        AnalyticsPeriod::Day => now
            .format(
                &time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
                    .expect("valid day format"),
            )
            .unwrap_or_else(|_| {
                format!(
                    "{:04}-{:02}-{:02}",
                    now.year(),
                    u8::from(now.month()),
                    now.day()
                )
            }),
    }
}

fn shift_anchor(period: AnalyticsPeriod, anchor: &str, delta: i32) -> String {
    match period {
        AnalyticsPeriod::Last7Days => {
            shift_day_anchor(anchor, delta * 7).unwrap_or_else(|| current_anchor(period))
        }
        AnalyticsPeriod::Last30Days => {
            shift_day_anchor(anchor, delta * 30).unwrap_or_else(|| current_anchor(period))
        }
        AnalyticsPeriod::Year => anchor
            .parse::<i32>()
            .map(|year| format!("{:04}", year + delta))
            .unwrap_or_else(|_| current_anchor(period)),
        AnalyticsPeriod::Month => {
            shift_month_anchor(anchor, delta).unwrap_or_else(|| current_anchor(period))
        }
        AnalyticsPeriod::Day => {
            shift_day_anchor(anchor, delta).unwrap_or_else(|| current_anchor(period))
        }
    }
}

fn shift_month_anchor(anchor: &str, delta: i32) -> Option<String> {
    let (year, month) = anchor.split_once('-')?;
    let year = year.parse::<i32>().ok()?;
    let month = month.parse::<i32>().ok()?;
    let zero_based = year.checked_mul(12)?.checked_add(month - 1 + delta)?;
    let new_year = zero_based.div_euclid(12);
    let new_month = zero_based.rem_euclid(12) + 1;
    Some(format!("{new_year:04}-{new_month:02}"))
}

fn shift_day_anchor(anchor: &str, delta: i32) -> Option<String> {
    let date = time::Date::parse(
        anchor,
        &time::format_description::parse_borrowed::<2>("[year]-[month]-[day]").ok()?,
    )
    .ok()?;
    let shifted = date.checked_add(time::Duration::days(delta as i64))?;
    Some(format!(
        "{:04}-{:02}-{:02}",
        shifted.year(),
        u8::from(shifted.month()),
        shifted.day()
    ))
}
