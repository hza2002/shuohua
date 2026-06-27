use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;
use std::collections::{HashMap, VecDeque};
use std::path::Path;

use crate::config::theme::TuiTheme;
use crate::history::{
    AggregateStats, AnalyticsPeriod, AnalyticsSnapshot, HistoryRecord, HistoryStatsSnapshot,
    HistoryStatsStatus,
};
use crate::ipc::protocol::{Command, Event};
use crate::tui::audio::{
    audio_info_for_record, missing_audio_info_for_record, open_audio_path, reveal_audio_path,
    AudioInfo,
};
use crate::tui::history::render::render_history;
use crate::tui::page::{KeyOutcome, Page};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyticsChart {
    Bar,
    Line,
}

impl AnalyticsChart {
    fn toggle(self) -> Self {
        match self {
            Self::Bar => Self::Line,
            Self::Line => Self::Bar,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalyticsSelection {
    pub period: AnalyticsPeriod,
    pub anchor: String,
    pub metric: AnalyticsMetric,
    pub chart: AnalyticsChart,
}

impl Default for AnalyticsSelection {
    fn default() -> Self {
        Self {
            period: AnalyticsPeriod::Month,
            anchor: current_anchor(AnalyticsPeriod::Month),
            metric: AnalyticsMetric::Records,
            chart: AnalyticsChart::Bar,
        }
    }
}

#[derive(Debug, Default)]
pub struct HistoryAnalyticsState {
    pub selection: AnalyticsSelection,
    pub snapshot: Option<AnalyticsSnapshot>,
    pub warning: Option<String>,
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
        }
    }

    pub fn filtered(&self) -> Vec<&HistoryRecord> {
        if self.search.is_empty() {
            return self.records.iter().collect();
        }
        let query = self.search.to_lowercase();
        self.records
            .iter()
            .filter(|record| {
                [
                    record.id.as_str(),
                    record.app.as_deref().unwrap_or_default(),
                    record.asr.text.as_str(),
                    &record.text,
                ]
                .join("\n")
                .to_lowercase()
                .contains(&query)
            })
            .collect()
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
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_top(&mut self) {
        self.selected = 0;
    }

    pub fn move_bottom(&mut self) {
        let len = self.filtered().len();
        self.selected = len.saturating_sub(1);
    }

    pub fn next_detail(&mut self) {
        self.detail = self.detail.next();
    }

    pub fn prev_detail(&mut self) {
        self.detail = self.detail.prev();
    }

    pub fn start_search(&mut self) {
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
        self.refresh_responses_pending = 3;
        self.refresh_needed = false;
        self.initial_loaded = true;
        self.pending_history_requests
            .push_back(HistoryRequestKind::RefreshPage {
                query: self.query(),
            });
        self.pending_analytics_requests
            .push_back(AnalyticsRequestKind::Refresh);
        self.refresh_batch_commands()
    }

    pub fn refresh_commands(&mut self) -> Vec<Command> {
        if !self.refresh_needed || self.refresh_in_flight {
            return Vec::new();
        }
        self.refresh_in_flight = true;
        self.refresh_responses_pending = 3;
        self.refresh_needed = false;
        self.pending_history_requests
            .push_back(HistoryRequestKind::RefreshPage {
                query: self.query(),
            });
        self.pending_analytics_requests
            .push_back(AnalyticsRequestKind::Refresh);
        self.refresh_batch_commands()
    }

    fn refresh_batch_commands(&self) -> Vec<Command> {
        vec![
            self.first_page_command(),
            Command::GetHistoryStats,
            self.analytics_command(),
        ]
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
        match crate::platform::desktop::write_clipboard_string(&text) {
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
        match crate::platform::desktop::write_clipboard_string(&text) {
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

    fn run_audio_action<F>(&self, action: F, status_key: &str) -> String
    where
        F: FnOnce(&Path) -> Result<()>,
    {
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

    fn toggle_view(&mut self) {
        self.view = match self.view {
            HistoryView::Records => HistoryView::Analytics,
            HistoryView::Analytics => HistoryView::Records,
        };
    }

    fn next_period_outcome(&mut self) -> KeyOutcome {
        self.analytics.selection.period = match self.analytics.selection.period {
            AnalyticsPeriod::Month => AnalyticsPeriod::Day,
            AnalyticsPeriod::Day => AnalyticsPeriod::Year,
            AnalyticsPeriod::Year => AnalyticsPeriod::Month,
        };
        self.analytics.selection.anchor = current_anchor(self.analytics.selection.period);
        KeyOutcome::command_and_status(
            self.analytics_command(),
            crate::t!("tui.history.analytics.loading"),
        )
    }

    fn shift_anchor_outcome(&mut self, delta: i32) -> KeyOutcome {
        self.analytics.selection.anchor = shift_anchor(
            self.analytics.selection.period,
            &self.analytics.selection.anchor,
            delta,
        );
        KeyOutcome::command_and_status(
            self.analytics_command(),
            crate::t!("tui.history.analytics.loading"),
        )
    }

    fn next_metric(&mut self) {
        self.analytics.selection.metric = self.analytics.selection.metric.next();
    }

    fn toggle_chart(&mut self) {
        self.analytics.selection.chart = self.analytics.selection.chart.toggle();
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
            Event::Error { kind, .. } => self.finish_history_error(kind),
            Event::AudioDeleted { id, .. } => {
                if let Some(record) = self.records.iter().find(|record| &record.id == id) {
                    self.audio_cache
                        .insert(record.id.clone(), missing_audio_info_for_record(record));
                }
            }
            Event::HistoryDeleted { .. } => {}
            Event::HistoryChanged if !self.refresh_needed => {
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
            KeyCode::Char('s') => self.toggle_view(),
            KeyCode::Char('p') if self.view == HistoryView::Analytics => {
                self.pending_analytics_requests
                    .push_back(AnalyticsRequestKind::Standalone);
                return self.next_period_outcome();
            }
            KeyCode::Char('[') if self.view == HistoryView::Analytics => {
                self.pending_analytics_requests
                    .push_back(AnalyticsRequestKind::Standalone);
                return self.shift_anchor_outcome(-1);
            }
            KeyCode::Char(']') if self.view == HistoryView::Analytics => {
                self.pending_analytics_requests
                    .push_back(AnalyticsRequestKind::Standalone);
                return self.shift_anchor_outcome(1);
            }
            KeyCode::Char('v') if self.view == HistoryView::Analytics => self.next_metric(),
            KeyCode::Char('c') if self.view == HistoryView::Analytics => self.toggle_chart(),
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
        AnalyticsPeriod::Year => now
            .format(&time::format_description::parse("[year]").expect("valid year format"))
            .unwrap_or_else(|_| now.year().to_string()),
        AnalyticsPeriod::Month => now
            .format(&time::format_description::parse("[year]-[month]").expect("valid month format"))
            .unwrap_or_else(|_| format!("{:04}-{:02}", now.year(), u8::from(now.month()))),
        AnalyticsPeriod::Day => now
            .format(
                &time::format_description::parse("[year]-[month]-[day]").expect("valid day format"),
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
        &time::format_description::parse("[year]-[month]-[day]").ok()?,
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
