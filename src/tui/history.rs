use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::SystemTime;

use anyhow::{bail, Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::ipc::protocol::Event;
use crate::state::history::{state_dir, HistoryRecord};
use crate::tui::page::{KeyOutcome, Page};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInfo {
    pub path: PathBuf,
    pub size_bytes: Option<u64>,
    pub modified: Option<SystemTime>,
}

impl AudioInfo {
    pub fn exists(&self) -> bool {
        self.size_bytes.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteAudioResult {
    Deleted,
    Missing,
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

    pub fn copy_selected_text(&self) -> Option<String> {
        self.selected_record().map(|record| record.text.clone())
    }

    pub fn copy_selected_asr(&self) -> Option<String> {
        self.selected_record().map(|record| record.asr.text.clone())
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
}

impl Page for HistoryPage {
    fn apply_event(&mut self, event: &Event, _active: bool) {
        match event {
            Event::HistoryAppended { record } => {
                self.refresh_audio_cache_for_record(record);
                self.records.insert(0, (**record).clone());
                self.selected = self.selected.min(self.records.len().saturating_sub(1));
            }
            Event::History { records } => {
                self.records = records.clone();
                self.refresh_audio_cache();
                self.selected = 0;
            }
            _ => {}
        }
    }

    fn on_key(&mut self, _key: KeyEvent) -> KeyOutcome {
        KeyOutcome::None
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &TuiTheme, _footer_status: &str) {
        render_history(frame, self, area, theme);
    }
}

mod ui {
    use ratatui::style::Color;

    use crate::config::theme::TuiTheme;

    fn rgb(value: u32) -> Color {
        Color::Rgb(
            ((value >> 16) & 0xff) as u8,
            ((value >> 8) & 0xff) as u8,
            (value & 0xff) as u8,
        )
    }

    pub fn fg(theme: &TuiTheme) -> Color {
        rgb(theme.foreground)
    }
    pub fn muted(theme: &TuiTheme) -> Color {
        rgb(theme.muted)
    }
    pub fn accent(theme: &TuiTheme) -> Color {
        rgb(theme.accent)
    }
    pub fn success(theme: &TuiTheme) -> Color {
        rgb(theme.success)
    }
    pub fn warning(theme: &TuiTheme) -> Color {
        rgb(theme.warning)
    }
    pub fn info(theme: &TuiTheme) -> Color {
        rgb(theme.info)
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
    let items: Vec<ListItem> = records
        .iter()
        .enumerate()
        .map(|(idx, record)| {
            ListItem::new(history_list_line(page, theme, record, idx == page.selected))
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
                return vec![Line::from("no pipeline steps")];
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
                return vec![Line::from("no ASR sessions")];
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
                .map(|error| format!("{}: {}", error.kind, error.msg))
                .unwrap_or_else(|| "no error".to_string()),
        ),
        HistoryDetail::Json => text_lines(
            serde_json::to_string_pretty(record)
                .unwrap_or_else(|e| format!("failed to render json: {e}")),
        ),
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
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars && max_chars > 0 {
        out.pop();
        out.push('…');
    }
    out
}

fn format_duration(ms: u64) -> String {
    let seconds = ms / 1000;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    if hours > 0 {
        format!("{hours}:{:02}:{:02}", minutes % 60, seconds % 60)
    } else {
        format!("{:02}:{:02}", minutes, seconds % 60)
    }
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

// ---------------------------------------------------------------------------
// Audio helpers (formerly tui/audio.rs). Only HistoryPage needs them.
// ---------------------------------------------------------------------------

pub fn audio_path_for_record_in_state_dir(state_dir: &Path, recording_id: &str) -> PathBuf {
    state_dir.join("audio").join(format!("{recording_id}.flac"))
}

pub fn audio_info_for_record(record: &HistoryRecord) -> AudioInfo {
    audio_info_for_recording_id_in_state_dir(&state_dir(), &record.id)
}

pub fn audio_info_for_recording_id_in_state_dir(state_dir: &Path, recording_id: &str) -> AudioInfo {
    let audio_dir = state_dir.join("audio");
    let flac = audio_dir.join(format!("{recording_id}.flac"));
    let m4a = audio_dir.join(format!("{recording_id}.m4a"));
    let flac_exists = flac.is_file();
    let m4a_exists = m4a.is_file();
    match (flac_exists, m4a_exists) {
        (true, false) => audio_info_for_path(flac),
        (false, true) => audio_info_for_path(m4a),
        (true, true) => {
            tracing::warn!(
                recording_id,
                flac = %flac.display(),
                m4a = %m4a.display(),
                "multiple retained audio files found"
            );
            missing_audio_info(flac)
        }
        (false, false) => missing_audio_info(flac),
    }
}

pub fn missing_audio_info_for_record(record: &HistoryRecord) -> AudioInfo {
    missing_audio_info(audio_path_for_record_in_state_dir(&state_dir(), &record.id))
}

fn missing_audio_info(path: PathBuf) -> AudioInfo {
    AudioInfo {
        path,
        size_bytes: None,
        modified: None,
    }
}

pub fn audio_info_for_path(path: PathBuf) -> AudioInfo {
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => AudioInfo {
            path,
            size_bytes: Some(metadata.len()),
            modified: metadata.modified().ok(),
        },
        _ => AudioInfo {
            path,
            size_bytes: None,
            modified: None,
        },
    }
}

pub fn delete_audio_path(path: &Path) -> Result<DeleteAudioResult> {
    ensure_audio_path(path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(DeleteAudioResult::Deleted),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DeleteAudioResult::Missing),
        Err(e) => Err(e).with_context(|| format!("delete audio {}", path.display())),
    }
}

fn open_audio_path(path: &Path) -> Result<()> {
    ensure_existing_audio(path)?;
    open_with_args(&[path.as_os_str()])
}

fn reveal_audio_path(path: &Path) -> Result<()> {
    ensure_existing_audio(path)?;
    open_with_args(&[std::ffi::OsStr::new("-R"), path.as_os_str()])
}

fn open_with_args(args: &[&std::ffi::OsStr]) -> Result<()> {
    ProcessCommand::new("/usr/bin/open")
        .args(args)
        .spawn()
        .context("launch open")?;
    Ok(())
}

fn ensure_existing_audio(path: &Path) -> Result<()> {
    ensure_audio_path(path)?;
    if !path.is_file() {
        bail!("audio file is missing: {}", path.display());
    }
    Ok(())
}

fn ensure_audio_path(path: &Path) -> Result<()> {
    if !matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("flac" | "m4a")
    ) {
        bail!(
            "refusing to operate on unsupported audio path: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_lossless_audio_by_recording_id() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-test-{}", ulid::Ulid::new()));
        let audio_dir = dir.join("audio");
        fs::create_dir_all(&audio_dir).unwrap();
        let path = audio_dir.join("01HXYZ.flac");
        fs::write(&path, [0u8; 12]).unwrap();

        let info = audio_info_for_recording_id_in_state_dir(&dir, "01HXYZ");

        assert_eq!(info.path, path);
        assert!(info.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resolves_compact_audio_by_recording_id() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-test-{}", ulid::Ulid::new()));
        let audio_dir = dir.join("audio");
        fs::create_dir_all(&audio_dir).unwrap();
        let path = audio_dir.join("01HXYZ.m4a");
        fs::write(&path, [0u8; 12]).unwrap();

        let info = audio_info_for_recording_id_in_state_dir(&dir, "01HXYZ");

        assert_eq!(info.path, path);
        assert!(info.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn duplicate_formats_are_reported_as_unavailable() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-test-{}", ulid::Ulid::new()));
        let audio_dir = dir.join("audio");
        fs::create_dir_all(&audio_dir).unwrap();
        fs::write(audio_dir.join("01HXYZ.flac"), [0u8; 12]).unwrap();
        fs::write(audio_dir.join("01HXYZ.m4a"), [0u8; 12]).unwrap();

        let info = audio_info_for_recording_id_in_state_dir(&dir, "01HXYZ");

        assert!(!info.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn audio_info_reports_existing_file_size() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("01HXYZ.wav");
        fs::write(&path, [0u8; 12]).unwrap();

        let info = audio_info_for_path(path.clone());

        assert_eq!(info.path, path);
        assert_eq!(info.size_bytes, Some(12));
        assert!(info.modified.is_some());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn audio_info_reports_missing_file() {
        let path =
            std::env::temp_dir().join(format!("shuohua-audio-missing-{}.wav", ulid::Ulid::new()));

        let info = audio_info_for_path(path.clone());

        assert_eq!(info.path, path);
        assert!(!info.exists());
        assert_eq!(info.size_bytes, None);
        assert_eq!(info.modified, None);
    }

    #[test]
    fn delete_audio_path_removes_supported_audio_file() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-delete-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("01HXYZ.flac");
        let jsonl = dir.join("2026-06.jsonl");
        fs::write(&audio, [0u8; 4]).unwrap();
        fs::write(&jsonl, "{}\n").unwrap();

        assert_eq!(
            delete_audio_path(&audio).unwrap(),
            DeleteAudioResult::Deleted
        );

        assert!(!audio.exists());
        assert!(jsonl.exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn delete_audio_path_refuses_unsupported_extension() {
        let path = std::env::temp_dir().join(format!("shuohua-audio-{}.wav", ulid::Ulid::new()));

        let err = delete_audio_path(&path).unwrap_err();

        assert!(err.to_string().contains("unsupported audio"));
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
}
