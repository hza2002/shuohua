use std::time::SystemTime;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::state::history::HistoryRecord;
use crate::tui::history::{Confirm, HistoryDetail, HistoryPage};
use crate::tui::ui;

pub(super) fn render_history(frame: &mut Frame, page: &HistoryPage, area: Rect, theme: &TuiTheme) {
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

pub(super) fn visible_range_for_selection(
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

pub(super) fn short_app_label(app: Option<&str>) -> String {
    let Some(app) = app else {
        return "-".to_string();
    };
    app.rsplit('.').next().unwrap_or(app).to_string()
}

pub(super) fn truncate_display(value: &str, max_chars: usize) -> String {
    ui::truncate_display(value, max_chars)
}

pub(super) fn format_duration(ms: u64) -> String {
    ui::format_duration(ms)
}

pub(super) fn format_local_time(value: time::OffsetDateTime) -> String {
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

pub(super) struct HistorySummary {
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
