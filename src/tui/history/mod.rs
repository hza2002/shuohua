use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;
use std::collections::HashMap;
use std::path::Path;

use crate::config::theme::TuiTheme;
use crate::ipc::protocol::{Command, Event};
use crate::state::history::HistoryRecord;
use crate::tui::audio::{
    audio_info_for_record, delete_audio_path, missing_audio_info_for_record, open_audio_path,
    reveal_audio_path, AudioInfo, DeleteAudioResult,
};
use crate::tui::history::render::render_history;
use crate::tui::page::{KeyOutcome, Page};

mod render;

#[cfg(test)]
mod tests;

pub const HISTORY_PAGE_SIZE: usize = 50;

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
    }

    pub fn search_char(&mut self, ch: char) {
        self.search.push(ch);
        self.selected = 0;
    }

    pub fn search_backspace(&mut self) {
        self.search.pop();
        self.selected = 0;
    }

    fn copy_text_outcome(&self) -> KeyOutcome {
        let Some(text) = self.selected_record().map(|record| record.text.clone()) else {
            return KeyOutcome::none();
        };
        match crate::platform::macos::clipboard::write_string(&text) {
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
        match crate::platform::macos::clipboard::write_string(&text) {
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
        let info = self
            .selected_record()
            .map(|record| self.audio_info_for_record(record))
            .expect("selected record exists");
        if info.exists() {
            self.confirm = Some(Confirm::DeleteAudio { record_id });
            crate::t!("tui.confirm.delete_audio")
        } else {
            crate::t!("tui.history.audio.missing_status")
        }
    }

    pub fn feed_confirm_key(&mut self, key: KeyEvent) -> Option<String> {
        if key.kind != KeyEventKind::Press || self.confirm.is_none() {
            return None;
        }
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => Some(self.confirm_yes()),
            KeyCode::Char('n') | KeyCode::Esc => {
                self.confirm = None;
                Some(crate::t!("tui.confirm.cancelled"))
            }
            _ => Some(String::new()),
        }
    }

    fn confirm_yes(&mut self) -> String {
        let Some(confirm) = self.confirm.take() else {
            return String::new();
        };
        match confirm {
            Confirm::DeleteAudio { record_id } => {
                let Some(record) = self.records.iter().find(|r| r.id == record_id).cloned() else {
                    return crate::t!("tui.history.audio.record_missing");
                };
                let info = self.audio_info_for_record(&record);
                if !info.exists() {
                    return crate::t!("tui.history.audio.missing_status");
                }
                let path = info.path;
                match delete_audio_path(&path) {
                    Ok(DeleteAudioResult::Deleted) => {
                        let info = missing_audio_info_for_record(&record);
                        self.audio_cache.insert(record.id.clone(), info);
                        crate::t!("tui.history.audio.deleted", path = path.display())
                    }
                    Ok(DeleteAudioResult::Missing) => {
                        let info = missing_audio_info_for_record(&record);
                        self.audio_cache.insert(record.id.clone(), info);
                        crate::t!("tui.history.audio.missing_status")
                    }
                    Err(e) => crate::t!("tui.error.audio_action", error = e),
                }
            }
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
        if self.loading_more {
            return KeyOutcome::status(crate::t!("tui.history.loading_more"));
        }
        if !self.has_more {
            return KeyOutcome::status(crate::t!("tui.history.no_more"));
        }
        let Some(oldest) = self.records.last() else {
            self.loading_more = true;
            return KeyOutcome::command_and_status(
                Command::GetHistory {
                    limit: HISTORY_PAGE_SIZE,
                    before: None,
                    query: None,
                },
                crate::t!("tui.history.loading_more"),
            );
        };
        self.loading_more = true;
        KeyOutcome::command_and_status(
            Command::GetHistory {
                limit: HISTORY_PAGE_SIZE,
                before: Some(format_rfc3339(oldest.started_at)),
                query: None,
            },
            crate::t!("tui.history.loading_more"),
        )
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
            Event::History { records } => {
                if self.loading_more {
                    self.merge_older(records.clone());
                } else {
                    self.merge_newest(records.clone());
                    self.refresh_audio_cache();
                }
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
                KeyCode::Esc | KeyCode::Enter => self.cancel_search(),
                KeyCode::Backspace => self.search_backspace(),
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.clear_search();
                }
                KeyCode::Char(ch) => self.search_char(ch),
                _ => {}
            }
            return KeyOutcome::none();
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_down(),
            KeyCode::Char('k') | KeyCode::Up => self.move_up(),
            KeyCode::Char('g') => self.move_top(),
            KeyCode::Char('G') => self.move_bottom(),
            KeyCode::Char('l') | KeyCode::Right => self.next_detail(),
            KeyCode::Char('h') | KeyCode::Left => self.prev_detail(),
            KeyCode::Esc => self.clear_search(),
            KeyCode::Enter | KeyCode::Char('y') => return self.copy_text_outcome(),
            KeyCode::Char('Y') => return self.copy_asr_outcome(),
            KeyCode::Char('o') => return KeyOutcome::status(self.open_selected_audio()),
            KeyCode::Char('r') => return KeyOutcome::status(self.reveal_selected_audio()),
            KeyCode::Char('d') => return KeyOutcome::status(self.request_delete_audio()),
            KeyCode::Char('m') => return self.load_more_outcome(),
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
