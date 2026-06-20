use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::ipc::protocol::{Command, Event};
use crate::state::history::HistoryRecord;
use crate::tui::audio::{
    audio_info_for_record, delete_audio_path, missing_audio_info_for_record, open_audio_path,
    reveal_audio_path, AudioInfo, DeleteAudioResult,
};
use crate::tui::page::{KeyOutcome, Page};
use crate::tui::ui;

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

fn render_history(frame: &mut Frame, page: &HistoryPage, area: Rect, theme: &TuiTheme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(0)])
        .split(area);
    let summary = HistorySummary::from(page);
    let search = if page.searching {
        format!("/{}_", page.search)
    } else if page.search.is_empty() {
        crate::t!("tui.search_prompt")
    } else {
        format!("/{}", page.search)
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(search),
            history_stats_line(&summary, theme),
        ])
        .block(
            Block::default()
                .title(crate::t!("tui.history_stats"))
                .borders(Borders::ALL),
        ),
        chunks[0],
    );

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(chunks[1]);
    let records = page.filtered();
    let visible = visible_range_for_selection(
        page.selected,
        records.len(),
        body[0].height.saturating_sub(2) as usize,
    );
    let items: Vec<ListItem> = records[visible.clone()]
        .iter()
        .enumerate()
        .map(|(idx, record)| {
            let absolute_idx = visible.start + idx;
            ListItem::new(history_list_line(
                page,
                theme,
                record,
                absolute_idx == page.selected,
            ))
        })
        .collect();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(crate::t!("tui.history_newest_first"))
                .borders(Borders::ALL),
        ),
        body[0],
    );

    let selected = records
        .get(page.selected)
        .map(|record| history_detail_text(page, theme, record, page.detail))
        .unwrap_or_else(|| vec![Line::from(crate::t!("tui.no_history_selected"))]);
    let selected = if let Some(confirm) = &page.confirm {
        let mut lines = vec![Line::styled(
            confirm_text(confirm),
            Style::default()
                .fg(ui::warning(theme))
                .add_modifier(Modifier::BOLD),
        )];
        lines.push(Line::from(""));
        lines.extend(selected);
        lines
    } else {
        selected
    };
    frame.render_widget(
        Paragraph::new(selected).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(history_detail_title(page.detail))
                .borders(Borders::ALL),
        ),
        body[1],
    );
}

fn history_stats_line(summary: &HistorySummary, theme: &TuiTheme) -> Line<'static> {
    Line::from(vec![
        label_span("records ", theme),
        value_span(summary.shown.to_string(), ui::accent(theme)),
        label_span(" shown / ", theme),
        value_span(summary.total.to_string(), ui::accent(theme)),
        label_span(" total    duration ", theme),
        value_span(
            format_duration(summary.total_duration_ms),
            ui::warning(theme),
        ),
        label_span("    words ", theme),
        value_span(summary.total_words.to_string(), ui::success(theme)),
        label_span("    avg ", theme),
        value_span(format_duration(summary.avg_duration_ms), ui::warning(theme)),
    ])
}

fn history_list_line(
    page: &HistoryPage,
    theme: &TuiTheme,
    record: &HistoryRecord,
    selected: bool,
) -> Line<'static> {
    let marker = if selected { "> " } else { "  " };
    let audio = history_audio_marker(page, record);
    let audio_color = if page.audio_info_for_record(record).exists() {
        ui::success(theme)
    } else {
        ui::muted(theme)
    };
    Line::from(vec![
        Span::styled(
            marker.to_string(),
            Style::default()
                .fg(if selected {
                    ui::accent(theme)
                } else {
                    ui::muted(theme)
                })
                .add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::styled(
            format!("{:<19}", format_local_time(record.started_at)),
            Style::default().fg(ui::muted(theme)),
        ),
        Span::raw(" "),
        Span::styled(
            format!(
                "{:<10}",
                truncate_display(&short_app_label(record.app.as_deref()), 10)
            ),
            Style::default().fg(ui::accent(theme)),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:>5}", format_duration(record.duration_ms)),
            Style::default().fg(ui::warning(theme)),
        ),
        Span::raw(" "),
        Span::styled(format!("{audio:<1}"), Style::default().fg(audio_color)),
        Span::raw(" "),
        Span::raw(record.text.replace('\n', " ")),
    ])
}

fn history_detail_title(detail: HistoryDetail) -> String {
    match detail {
        HistoryDetail::Details => crate::t!("tui.history.detail.details"),
        HistoryDetail::Asr => crate::t!("tui.history.detail.asr"),
        HistoryDetail::Pipeline => crate::t!("tui.history.detail.pipeline"),
        HistoryDetail::Sessions => crate::t!("tui.history.detail.sessions"),
        HistoryDetail::Error => crate::t!("tui.history.detail.error"),
        HistoryDetail::Json => crate::t!("tui.history.detail.json"),
    }
}

fn history_detail_text(
    page: &HistoryPage,
    theme: &TuiTheme,
    record: &HistoryRecord,
    detail: HistoryDetail,
) -> Vec<Line<'static>> {
    match detail {
        HistoryDetail::Details => history_details_lines(page, theme, record),
        HistoryDetail::Asr => text_lines(format!(
            "provider: {}\naudio: {}\nstarted: {}\n\n{}",
            record.asr.provider,
            format_duration(record.asr.audio_ms),
            format_local_time(record.started_at),
            record.asr.text
        )),
        HistoryDetail::Pipeline => {
            if record.pipeline.is_empty() {
                return vec![Line::from(crate::t!("tui.history.pipeline.empty"))];
            }
            text_lines(
                record
                    .pipeline
                    .iter()
                    .map(|step| {
                        let body = step.text.as_deref().or(step.error.as_deref()).unwrap_or("");
                        format!(
                            "{}  {:?}  {:.1}ms\n{}",
                            step.name, step.status, step.duration_ms, body
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            )
        }
        HistoryDetail::Sessions => {
            if record.asr.sessions.is_empty() {
                return vec![Line::from(crate::t!("tui.history.sessions.empty"))];
            }
            text_lines(
                record
                    .asr
                    .sessions
                    .iter()
                    .enumerate()
                    .map(|(idx, session)| {
                        format!(
                            "#{}  {} -> {}  audio {}\n{}",
                            idx + 1,
                            format_local_time(session.started_at),
                            format_local_time(session.ended_at),
                            format_duration(session.audio_ms),
                            session.text
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            )
        }
        HistoryDetail::Error => text_lines(
            record
                .error
                .as_ref()
                .map(|error| {
                    crate::i18n::tr(
                        "tui.history.detail.error_message",
                        &[("kind", error.kind.clone()), ("error", error.msg.clone())],
                    )
                })
                .unwrap_or_else(|| crate::t!("tui.history.error.empty")),
        ),
        HistoryDetail::Json => {
            text_lines(serde_json::to_string_pretty(record).unwrap_or_else(|e| {
                crate::i18n::tr(
                    "tui.history.json.render_failed",
                    &[("error", e.to_string())],
                )
            }))
        }
    }
}

fn history_audio_marker(page: &HistoryPage, record: &HistoryRecord) -> String {
    if page.audio_info_for_record(record).exists() {
        crate::t!("tui.history.audio.present_short")
    } else {
        crate::t!("tui.history.audio.missing_short")
    }
}

fn history_details_lines(
    page: &HistoryPage,
    theme: &TuiTheme,
    record: &HistoryRecord,
) -> Vec<Line<'static>> {
    let info = page.audio_info_for_record(record);
    let status = if info.exists() {
        crate::t!("tui.history.audio.present")
    } else {
        crate::t!("tui.history.audio.missing")
    };
    let size = info
        .size_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "-".to_string());
    let modified = info
        .modified
        .map(format_system_time)
        .unwrap_or_else(|| "-".to_string());
    let mut lines = vec![
        kv_line("status", format!("{:?}", record.status), ui::success(theme)),
        kv_line(
            "app",
            short_app_label(record.app.as_deref()),
            ui::accent(theme),
        ),
        kv_line(
            "started",
            format_local_time(record.started_at),
            ui::fg(theme),
        ),
        kv_line(
            "duration",
            format_duration(record.duration_ms),
            ui::warning(theme),
        ),
        kv_line(
            "words",
            record.text_stats().words.to_string(),
            ui::accent(theme),
        ),
        kv_line("asr", record.asr.provider.clone(), ui::info(theme)),
        kv_line("pipeline", pipeline_summary(record), ui::fg(theme)),
        kv_line(
            "audio",
            status,
            if info.exists() {
                ui::success(theme)
            } else {
                ui::muted(theme)
            },
        ),
        kv_line(crate::t!("tui.history.audio.size"), size, ui::fg(theme)),
        kv_line(
            crate::t!("tui.history.audio.mtime"),
            modified,
            ui::fg(theme),
        ),
        Line::from(""),
        kv_line("text", "", ui::fg(theme)),
    ];
    lines.extend(text_lines(record.text.clone()));
    lines
}

fn confirm_text(confirm: &Confirm) -> String {
    match confirm {
        Confirm::DeleteAudio { record_id } => {
            crate::t!("tui.confirm.delete_audio_detail", id = record_id)
        }
    }
}

fn kv_line(label: impl Into<String>, value: impl Into<String>, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{}: ", label.into()),
            Style::default().fg(Color::DarkGray),
        ),
        value_span(value.into(), color),
    ])
}

fn label_span(text: impl Into<String>, theme: &TuiTheme) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(ui::muted(theme)))
}

fn value_span(text: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(color))
}

fn text_lines(text: String) -> Vec<Line<'static>> {
    text.lines()
        .map(|line| Line::from(line.to_string()))
        .collect()
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes_f = bytes as f64;
    if bytes_f >= GIB {
        format!("{:.1} GiB", bytes_f / GIB)
    } else if bytes_f >= MIB {
        format!("{:.1} MiB", bytes_f / MIB)
    } else if bytes_f >= KIB {
        format!("{:.1} KiB", bytes_f / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn format_system_time(value: SystemTime) -> String {
    let Ok(duration) = value.duration_since(std::time::UNIX_EPOCH) else {
        return "-".to_string();
    };
    let Ok(datetime) = time::OffsetDateTime::from_unix_timestamp(duration.as_secs() as i64) else {
        return "-".to_string();
    };
    format_local_time(datetime)
}

fn visible_range_for_selection(
    selected: usize,
    total: usize,
    visible_len: usize,
) -> std::ops::Range<usize> {
    if total == 0 || visible_len == 0 {
        return 0..0;
    }
    let visible_len = visible_len.min(total);
    let half = visible_len / 2;
    let mut start = selected.saturating_sub(half);
    start = start.min(total - visible_len);
    start..start + visible_len
}

fn pipeline_summary(record: &HistoryRecord) -> String {
    if record.pipeline.is_empty() {
        return "-".to_string();
    }
    record
        .pipeline
        .iter()
        .map(|step| format!("{}:{:?}", step.name, step.status))
        .collect::<Vec<_>>()
        .join(" -> ")
}

fn short_app_label(app: Option<&str>) -> String {
    let Some(app) = app else {
        return "-".to_string();
    };
    app.rsplit('.').next().unwrap_or(app).to_string()
}

fn truncate_display(value: &str, max_chars: usize) -> String {
    ui::truncate_display(value, max_chars)
}

fn format_duration(ms: u64) -> String {
    ui::format_duration(ms)
}

fn format_local_time(value: time::OffsetDateTime) -> String {
    let value = match time::UtcOffset::current_local_offset() {
        Ok(offset) => value.to_offset(offset),
        Err(_) => value,
    };
    value
        .format(
            &time::format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]")
                .expect("valid static time format"),
        )
        .unwrap_or_else(|_| value.to_string())
}

fn format_rfc3339(value: time::OffsetDateTime) -> String {
    value
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| value.to_string())
}

struct HistorySummary {
    total: usize,
    shown: usize,
    total_duration_ms: u64,
    avg_duration_ms: u64,
    total_words: usize,
}

impl HistorySummary {
    fn from(page: &HistoryPage) -> Self {
        let filtered = page.filtered();
        let total_duration_ms = page
            .records
            .iter()
            .map(|record| record.duration_ms)
            .sum::<u64>();
        let total_words = page
            .records
            .iter()
            .map(|record| record.text_stats().words)
            .sum::<usize>();
        let avg_duration_ms = if page.records.is_empty() {
            0
        } else {
            total_duration_ms / page.records.len() as u64
        };
        Self {
            total: page.records.len(),
            shown: filtered.len(),
            total_duration_ms,
            avg_duration_ms,
            total_words,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::history::{
        AsrHistory, AsrSessionHistory, HistoryStatus, PipelineStepHistory, PipelineStepStatus,
    };

    fn sample_record(id: &str, day: u8) -> HistoryRecord {
        let started_at = time::Date::from_calendar_date(2026, time::Month::June, day)
            .unwrap()
            .with_hms(12, 0, 0)
            .unwrap()
            .assume_utc();
        HistoryRecord {
            version: 1,
            id: id.to_string(),
            started_at,
            ended_at: started_at + time::Duration::seconds(3),
            duration_ms: 3000,
            status: HistoryStatus::Submitted,
            app: Some("com.example.App".to_string()),
            text: format!("text {id}"),
            text_stats: crate::text_stats::compute(&format!("text {id}")),
            asr: AsrHistory {
                provider: "apple".to_string(),
                text: format!("asr {id}"),
                duration_ms: 3000,
                audio_ms: 3000,
                sessions: vec![AsrSessionHistory {
                    text: format!("asr {id}"),
                    started_at,
                    ended_at: started_at + time::Duration::seconds(3),
                    audio_ms: 3000,
                }],
            },
            pipeline: vec![PipelineStepHistory {
                name: "filler".to_string(),
                status: PipelineStepStatus::Ok,
                duration_ms: 1.0,
                text: Some(format!("text {id}")),
                error: None,
            }],
            error: None,
        }
    }

    #[test]
    fn local_time_format_omits_fraction_and_offset() {
        let value = time::macros::datetime!(2026-06-17 12:34:56.789 UTC);
        let text = format_local_time(value);

        assert!(!text.contains('.'));
        assert!(!text.ends_with('Z'));
        assert_eq!(text.len(), "2026-06-17 12:34:56".len());
    }

    #[test]
    fn short_app_label_uses_bundle_tail() {
        assert_eq!(short_app_label(Some("com.mitchellh.ghostty")), "ghostty");
        assert_eq!(short_app_label(None), "-");
    }

    #[test]
    fn truncate_display_marks_long_values() {
        assert_eq!(truncate_display("Ghostty", 9), "Ghostty");
        assert_eq!(truncate_display("Ghostty", 10), "Ghostty");
        assert_eq!(truncate_display("VeryLongApp", 9), "VeryLong…");
    }

    #[test]
    fn visible_range_keeps_selected_near_middle() {
        assert_eq!(visible_range_for_selection(0, 100, 9), 0..9);
        assert_eq!(visible_range_for_selection(4, 100, 9), 0..9);
        assert_eq!(visible_range_for_selection(20, 100, 9), 16..25);
        assert_eq!(visible_range_for_selection(98, 100, 9), 91..100);
    }

    #[test]
    fn page_requests_older_history_from_oldest_loaded_record() {
        let mut page = HistoryPage::new();
        page.records = vec![
            sample_record("01HXYZABCDEF0123456789AAA1", 3),
            sample_record("01HXYZABCDEF0123456789AAA0", 2),
        ];

        let outcome = page.load_more_outcome();

        assert_eq!(
            outcome.command,
            Some(crate::ipc::protocol::Command::GetHistory {
                limit: HISTORY_PAGE_SIZE,
                before: Some("2026-06-02T12:00:00Z".to_string()),
                query: None,
            })
        );
    }

    #[test]
    fn appending_history_deduplicates_existing_records() {
        let mut page = HistoryPage::new();
        let record = sample_record("01HXYZABCDEF0123456789AAA1", 3);
        page.apply_event(
            &Event::History {
                records: vec![record.clone()],
            },
            true,
        );

        page.apply_event(
            &Event::HistoryAppended {
                record: Box::new(record),
            },
            true,
        );

        assert_eq!(page.records.len(), 1);
    }

    #[test]
    fn appending_history_keeps_existing_selection_on_same_record() {
        let mut page = HistoryPage::new();
        page.records = vec![
            sample_record("01HXYZABCDEF0123456789AAA2", 4),
            sample_record("01HXYZABCDEF0123456789AAA1", 3),
        ];
        page.selected = 1;

        page.apply_event(
            &Event::HistoryAppended {
                record: Box::new(sample_record("01HXYZABCDEF0123456789AAA3", 5)),
            },
            true,
        );

        assert_eq!(page.records[page.selected].id, "01HXYZABCDEF0123456789AAA1");
    }

    #[test]
    fn appending_history_preserves_filtered_selection_when_new_record_does_not_match() {
        let mut page = HistoryPage::new();
        page.records = vec![
            sample_record("01HXYZABCDEF0123456789AAA2", 4),
            sample_record("01HXYZABCDEF0123456789AAA1", 3),
        ];
        page.search = "AAA1".to_string();
        page.selected = 0;

        page.apply_event(
            &Event::HistoryAppended {
                record: Box::new(sample_record("01HXYZABCDEF0123456789AAA3", 5)),
            },
            true,
        );

        assert_eq!(
            page.selected_record().unwrap().id,
            "01HXYZABCDEF0123456789AAA1"
        );
    }

    #[test]
    fn initial_history_preserves_server_order() {
        let mut page = HistoryPage::new();
        page.apply_event(
            &Event::History {
                records: vec![
                    sample_record("01HXYZABCDEF0123456789AAA2", 4),
                    sample_record("01HXYZABCDEF0123456789AAA1", 3),
                    sample_record("01HXYZABCDEF0123456789AAA0", 2),
                ],
            },
            true,
        );

        assert_eq!(page.records[0].id, "01HXYZABCDEF0123456789AAA2");
        assert_eq!(page.records[2].id, "01HXYZABCDEF0123456789AAA0");
    }

    #[test]
    fn load_more_from_empty_history_marks_request_in_flight() {
        let mut page = HistoryPage::new();

        let outcome = page.load_more_outcome();

        assert!(page.loading_more);
        assert!(matches!(
            outcome.command,
            Some(crate::ipc::protocol::Command::GetHistory { before: None, .. })
        ));
    }

    #[test]
    fn loading_more_appends_and_deduplicates_older_records() {
        let mut page = HistoryPage::new();
        let newest = sample_record("01HXYZABCDEF0123456789AAA2", 4);
        let duplicate = sample_record("01HXYZABCDEF0123456789AAA1", 3);
        page.records = vec![newest, duplicate.clone()];
        page.loading_more = true;

        page.apply_event(
            &Event::History {
                records: vec![duplicate, sample_record("01HXYZABCDEF0123456789AAA0", 2)],
            },
            true,
        );

        assert_eq!(page.records.len(), 3);
        assert_eq!(page.records[2].id, "01HXYZABCDEF0123456789AAA0");
    }
}
