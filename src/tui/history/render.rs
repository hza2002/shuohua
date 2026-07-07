use std::time::SystemTime;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::history::{AggregateStats, AnalyticsPeriod, AnalyticsPoint, HistoryRecord};
use crate::tui::history::{
    AnalyticsChart, AnalyticsMetric, Confirm, HistoryDetail, HistoryPage, HistoryView, ListHit,
};
use crate::tui::ui;

pub(super) fn render_history(frame: &mut Frame, page: &HistoryPage, area: Rect, theme: &TuiTheme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(0)])
        .split(area);
    let summary = history_summary_text(page);
    let search = if page.searching {
        format!("/{}_", page.search)
    } else if page.search.is_empty() {
        crate::t!("tui.search_prompt")
    } else {
        format!("/{}", page.search)
    };
    frame.render_widget(
        Paragraph::new(vec![Line::from(search), Line::raw(summary)]).block(
            Block::default()
                .title(crate::t!("tui.history_stats"))
                .borders(Borders::ALL),
        ),
        chunks[0],
    );

    if page.view == HistoryView::Analytics {
        // No clickable record list or detail tabs in analytics view.
        *page.list_hit.borrow_mut() = None;
        page.detail_tabs.borrow_mut().clear();
        *page.detail_hit.borrow_mut() = None;
        render_analytics(frame, page, chunks[1], theme);
        return;
    }

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(chunks[1]);
    let records = page.filtered();
    let visible = ui::visible_range_for_selection(
        page.selected,
        records.len(),
        body[0].height.saturating_sub(2) as usize,
    );
    // Capture the list geometry so on_mouse can map a click to a record row.
    *page.list_hit.borrow_mut() = Some(ListHit {
        x: body[0].x + 1,
        y: body[0].y + 1,
        width: body[0].width.saturating_sub(2),
        height: (visible.end - visible.start) as u16,
        first: visible.start,
    });
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

    // Clickable detail sub-tab bar on top, detail content below.
    let detail_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(body[1]);
    render_detail_tabs(frame, page, detail_area[0], theme);

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
    // Pre-wrap so scroll offsets map 1:1 to rows (no ratatui `Wrap`), then clamp
    // and record the scroll bounds + pane rect for keyboard/wheel scrolling.
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(detail_area[1]);
    let wrapped = ui::wrap_styled_lines(&selected, (inner.width as usize).max(1));
    let max_scroll = (wrapped.len() as u16).saturating_sub(inner.height);
    page.detail_max_scroll.set(max_scroll);
    *page.detail_hit.borrow_mut() = Some(detail_area[1]);
    let scroll = page.detail_scroll.min(max_scroll);
    frame.render_widget(
        Paragraph::new(wrapped).scroll((scroll, 0)).block(block),
        detail_area[1],
    );
}

pub(super) fn history_summary_text(page: &HistoryPage) -> String {
    let loaded = page.records.len();
    let total = page
        .stats
        .as_ref()
        .map(|stats| stats.total)
        .unwrap_or_else(|| {
            page.records.iter().fold(
                AggregateStats {
                    records: loaded as u64,
                    ..AggregateStats::default()
                },
                |mut stats, record| {
                    stats.words += record.text_stats().words as u64;
                    stats.duration_ms += record.duration_ms;
                    stats.asr_duration_ms += record.asr.duration_ms;
                    stats.asr_audio_ms += record.asr.audio_ms;
                    stats
                },
            )
        });
    let query = page.search.trim();
    let (record_count, words, total_duration, speech_duration, effective_duration) =
        if query.is_empty() {
            (
                total.records.to_string(),
                total.words.to_string(),
                format_duration(total.duration_ms),
                format_duration(total.asr_duration_ms),
                format_duration(total.asr_audio_ms),
            )
        } else if let Some(search_stats) = page
            .search_stats
            .as_ref()
            .filter(|search_stats| search_stats.query == query)
        {
            (
                format!("{}/{}", search_stats.matched, total.records),
                search_stats.stats.words.to_string(),
                format_duration(search_stats.stats.duration_ms),
                format_duration(search_stats.stats.asr_duration_ms),
                format_duration(search_stats.stats.asr_audio_ms),
            )
        } else {
            (
                format!("?/{}", total.records),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
            )
        };
    crate::i18n::tr(
        "tui.history.summary",
        &[
            ("records", record_count),
            ("words", words),
            ("total_duration", total_duration),
            ("speech_duration", speech_duration),
            ("effective_duration", effective_duration),
        ],
    )
}

fn render_analytics(frame: &mut Frame, page: &HistoryPage, area: Rect, theme: &TuiTheme) {
    let title = crate::i18n::tr(
        "tui.history.analytics.title",
        &[
            (
                "period",
                analytics_period_label(page.analytics.selection.period),
            ),
            (
                "metric",
                analytics_metric_label(page.analytics.selection.metric),
            ),
            (
                "chart",
                analytics_chart_label(page.analytics.selection.chart),
            ),
            ("anchor", page.analytics.selection.anchor.clone()),
        ],
    );
    let mut lines = Vec::new();
    if let Some(warning) = &page.analytics.warning {
        lines.push(Line::styled(
            crate::i18n::tr(
                "tui.history.analytics.warning",
                &[("warning", warning.clone())],
            ),
            Style::default().fg(ui::warning(theme)),
        ));
    }
    if let Some(snapshot) = &page.analytics.snapshot {
        lines.extend(chart_lines(
            &snapshot.points,
            page.analytics.selection.metric,
            page.analytics.selection.chart,
            area.width.saturating_sub(4) as usize,
        ));
    } else {
        lines.push(Line::from(crate::t!("tui.history.analytics.empty")));
    }
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}

fn analytics_period_label(period: AnalyticsPeriod) -> String {
    match period {
        AnalyticsPeriod::Year => crate::t!("tui.history.analytics.period_year"),
        AnalyticsPeriod::Month => crate::t!("tui.history.analytics.period_month"),
        AnalyticsPeriod::Day => crate::t!("tui.history.analytics.period_day"),
    }
}

fn analytics_metric_label(metric: AnalyticsMetric) -> String {
    match metric {
        AnalyticsMetric::Records => crate::t!("tui.history.analytics.metric_records"),
        AnalyticsMetric::Words => crate::t!("tui.history.analytics.metric_words"),
        AnalyticsMetric::Duration => crate::t!("tui.history.analytics.metric_duration"),
        AnalyticsMetric::AsrAudio => crate::t!("tui.history.analytics.metric_asr_audio"),
    }
}

fn analytics_chart_label(chart: AnalyticsChart) -> String {
    match chart {
        AnalyticsChart::Bar => crate::t!("tui.history.analytics.chart_bar"),
        AnalyticsChart::Line => crate::t!("tui.history.analytics.chart_line"),
    }
}

fn chart_lines(
    points: &[AnalyticsPoint],
    metric: AnalyticsMetric,
    chart: AnalyticsChart,
    width: usize,
) -> Vec<Line<'static>> {
    let label_width = 10usize.min(width);
    let bar_width = width.saturating_sub(label_width + 3).max(1);
    let values = points
        .iter()
        .map(|point| metric_value(&point.stats, metric))
        .collect::<Vec<_>>();
    let max = values.iter().copied().max().unwrap_or(0).max(1);
    points
        .iter()
        .zip(values)
        .map(|(point, value)| {
            let filled = ((value as usize * bar_width) / max as usize).min(bar_width);
            let glyph = match chart {
                AnalyticsChart::Bar => "█",
                AnalyticsChart::Line => "─",
            };
            let bar = glyph.repeat(filled.max((value > 0) as usize));
            Line::raw(format!(
                "{:<label_width$} {:>6} {}",
                truncate_display(&point.key, label_width),
                value,
                bar
            ))
        })
        .collect()
}

fn metric_value(stats: &AggregateStats, metric: AnalyticsMetric) -> u64 {
    match metric {
        AnalyticsMetric::Records => stats.records,
        AnalyticsMetric::Words => stats.words,
        AnalyticsMetric::Duration => stats.duration_ms / 1000,
        AnalyticsMetric::AsrAudio => stats.asr_audio_ms / 1000,
    }
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

/// Render the clickable detail sub-tab bar and register a hit rect per tab so
/// `on_mouse` can switch views on click. The active tab is bracketed and
/// accent-highlighted; `h/l` still cycles the same set.
fn render_detail_tabs(frame: &mut Frame, page: &HistoryPage, area: Rect, theme: &TuiTheme) {
    let mut tabs = page.detail_tabs.borrow_mut();
    tabs.clear();
    let mut spans = Vec::new();
    let mut x = area.x;
    let right = area.x + area.width;
    for detail in HistoryDetail::ALL {
        let label = history_detail_title(detail);
        let selected = detail == page.detail;
        let text = if selected {
            format!("[{label}]")
        } else {
            format!(" {label} ")
        };
        let w = ui::display_width(&text) as u16;
        if x < right {
            tabs.push((Rect::new(x, area.y, w.min(right - x), 1), detail));
        }
        let style = if selected {
            Style::default()
                .fg(ui::accent(theme))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ui::muted(theme))
        };
        spans.push(Span::styled(text, style));
        spans.push(Span::raw(" "));
        x += w + 1;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
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

pub(super) fn history_detail_text(
    page: &HistoryPage,
    theme: &TuiTheme,
    record: &HistoryRecord,
    detail: HistoryDetail,
) -> Vec<Line<'static>> {
    match detail {
        HistoryDetail::Details => history_details_lines(page, theme, record),
        HistoryDetail::Asr => text_lines(format!(
            "provider: {}\nspeech: {}\neffective: {}\nstarted: {}\n\n{}",
            record.asr.provider,
            format_duration(record.asr.duration_ms),
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
        kv_line(
            theme,
            "status",
            format!("{:?}", record.status),
            ui::success(theme),
        ),
        kv_line(
            theme,
            "app",
            short_app_label(record.app.as_deref()),
            ui::accent(theme),
        ),
        kv_line(
            theme,
            "started",
            format_local_time(record.started_at),
            ui::fg(theme),
        ),
        kv_line(
            theme,
            "total",
            format_duration(record.duration_ms),
            ui::warning(theme),
        ),
        kv_line(
            theme,
            "speech",
            format_duration(record.asr.duration_ms),
            ui::warning(theme),
        ),
        kv_line(
            theme,
            "effective",
            format_duration(record.asr.audio_ms),
            ui::warning(theme),
        ),
        kv_line(
            theme,
            "words",
            record.text_stats().words.to_string(),
            ui::accent(theme),
        ),
        kv_line(theme, "asr", record.asr.provider.clone(), ui::info(theme)),
        kv_line(theme, "pipeline", pipeline_summary(record), ui::fg(theme)),
        kv_line(
            theme,
            "audio",
            status,
            if info.exists() {
                ui::success(theme)
            } else {
                ui::muted(theme)
            },
        ),
        kv_line(
            theme,
            crate::t!("tui.history.audio.size"),
            size,
            ui::fg(theme),
        ),
        kv_line(
            theme,
            crate::t!("tui.history.audio.mtime"),
            modified,
            ui::fg(theme),
        ),
        Line::from(""),
        kv_line(theme, "text", "", ui::fg(theme)),
    ];
    lines.extend(text_lines(record.text.clone()));
    lines
}

fn confirm_text(confirm: &Confirm) -> String {
    match confirm {
        Confirm::DeleteAudio { record_id } => {
            crate::t!("tui.confirm.delete_audio_detail", id = record_id)
        }
        Confirm::DeleteHistory { record_id } => {
            crate::t!("tui.confirm.delete_history_detail", id = record_id)
        }
    }
}

fn kv_line(
    theme: &TuiTheme,
    label: impl Into<String>,
    value: impl Into<String>,
    color: Color,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{}: ", label.into()),
            Style::default().fg(ui::muted(theme)),
        ),
        value_span(value.into(), color),
    ])
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
