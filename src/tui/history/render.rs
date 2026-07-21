use std::time::SystemTime;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::history::{
    AggregateStats, AnalyticsPeriod, AnalyticsPoint, CleanupPreview, CleanupResult, CleanupScope,
    CleanupWindow, HistoryRecord,
};
use crate::tui::history::{
    AnalyticsMetric, CleanupMode, CleanupSelect, Confirm, HistoryDetail, HistoryPage, HistoryView,
    ListHit, WindowChoice,
};
use crate::tui::ui;

pub(super) fn render_history(frame: &mut Frame, page: &HistoryPage, area: Rect, theme: &TuiTheme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(0)])
        .split(area);
    let summary = history_summary_text(page);
    let header = if page.view == HistoryView::Analytics {
        analytics_title(page)
    } else if page.searching {
        format!("/{}_", page.search)
    } else if page.search.is_empty() {
        crate::t!("tui.search_prompt")
    } else {
        format!("/{}", page.search)
    };
    frame.render_widget(
        Paragraph::new(vec![Line::from(header), Line::raw(summary)]).block(
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

    // The cleanup modal draws over the whole History page, forcing the user to
    // confirm or cancel before returning to the list/detail view.
    if let Some(mode) = &page.cleanup {
        render_cleanup_popup(frame, area, theme, mode);
    }
}

fn render_cleanup_popup(frame: &mut Frame, area: Rect, theme: &TuiTheme, mode: &CleanupMode) {
    let lines = cleanup_sheet_lines(theme, mode);
    let width = area.width.clamp(20, 64);
    // Body height = content + 2 borders, clamped to the available area.
    let height = (lines.len() as u16 + 2).clamp(3, area.height.max(3));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect::new(x, y, width, height);
    // Clear only the popup's own footprint so its border + content render on a
    // clean background, without wiping surrounding cells.
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .title(crate::t!("tui.history.cleanup.title"))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ui::warning(theme)));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    frame.render_widget(
        Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }),
        inner,
    );
}

pub(super) fn history_summary_text(page: &HistoryPage) -> String {
    let loaded = page.records.len();
    let total = if page.view == HistoryView::Analytics {
        page.analytics
            .snapshot
            .as_ref()
            .filter(|snapshot| {
                snapshot.period == page.analytics.selection.period
                    && snapshot.anchor == page.analytics.selection.anchor
            })
            .map(|snapshot| aggregate_points(&snapshot.points))
    } else {
        None
    }
    .or_else(|| page.stats.as_ref().map(|stats| stats.total))
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

fn aggregate_points(points: &[AnalyticsPoint]) -> AggregateStats {
    points
        .iter()
        .fold(AggregateStats::default(), |mut stats, point| {
            stats.records = stats.records.saturating_add(point.stats.records);
            stats.words = stats.words.saturating_add(point.stats.words);
            stats.duration_ms = stats.duration_ms.saturating_add(point.stats.duration_ms);
            stats.asr_duration_ms = stats
                .asr_duration_ms
                .saturating_add(point.stats.asr_duration_ms);
            stats.asr_audio_ms = stats.asr_audio_ms.saturating_add(point.stats.asr_audio_ms);
            stats
        })
}

fn render_analytics(frame: &mut Frame, page: &HistoryPage, area: Rect, theme: &TuiTheme) {
    let title = analytics_title(page);
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
    let block = Block::default().title(title).borders(Borders::ALL);
    let inner = block.inner(area);
    if let Some(snapshot) = &page.analytics.snapshot {
        lines.extend(grouped_chart_lines(
            &snapshot.points,
            page.analytics.selection.metric,
            page.analytics.selection.visible_metrics,
            inner.width as usize,
            inner.height as usize,
            theme,
        ));
    } else {
        lines.push(Line::from(crate::t!("tui.history.analytics.empty")));
    }
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn analytics_title(page: &HistoryPage) -> String {
    crate::i18n::tr(
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
            ("anchor", page.analytics.selection.anchor.clone()),
        ],
    )
}

fn analytics_period_label(period: AnalyticsPeriod) -> String {
    match period {
        AnalyticsPeriod::Last7Days => crate::t!("tui.history.analytics.period_last_7_days"),
        AnalyticsPeriod::Last30Days => crate::t!("tui.history.analytics.period_last_30_days"),
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

pub(super) fn grouped_chart_lines(
    points: &[AnalyticsPoint],
    focus: AnalyticsMetric,
    visible_metrics: [bool; 4],
    width: usize,
    height: usize,
    theme: &TuiTheme,
) -> Vec<Line<'static>> {
    if points.is_empty() || width == 0 || height == 0 {
        return Vec::new();
    }

    let metrics = AnalyticsMetric::ALL
        .into_iter()
        .filter(|metric| visible_metrics[metric.index()])
        .collect::<Vec<_>>();
    if metrics.is_empty() {
        return Vec::new();
    }

    let min_group_width = metrics.len().max(1) + 1;
    let bucket_count = points.len().min((width / min_group_width).max(1));
    let points = &points[..bucket_count];
    let group_width = (width / points.len().max(1)).max(min_group_width);
    let bar_width = ((group_width.saturating_sub(1)) / metrics.len().max(1)).max(1);

    let legend_rows = 1usize;
    let value_rows = 1usize;
    let axis_rows = 1usize;
    let chart_height = height
        .saturating_sub(legend_rows + value_rows + axis_rows)
        .max(1);
    let metric_max = metrics
        .iter()
        .copied()
        .map(|metric| {
            points
                .iter()
                .map(|point| metric_value(&point.stats, metric))
                .max()
                .unwrap_or(0)
                .max(1)
        })
        .collect::<Vec<_>>();
    let mut lines = Vec::with_capacity(height);
    lines.push(legend_line(&metrics, focus, theme));
    lines.push(value_line(points, focus, group_width));
    for row in 0..chart_height {
        let level = chart_height - row;
        let mut spans = Vec::with_capacity(points.len() * (metrics.len() * bar_width + 1));
        for point in points {
            let bars_width = metrics.len() * bar_width;
            let left_padding = group_width.saturating_sub(bars_width) / 2;
            if left_padding > 0 {
                spans.push(Span::raw(" ".repeat(left_padding)));
            }
            let mut used = left_padding;
            for (idx, metric) in metrics.iter().copied().enumerate() {
                let value = metric_value(&point.stats, metric);
                let filled = scaled_height(value, metric_max[idx], chart_height);
                let glyph = if filled >= level { '█' } else { ' ' };
                for _ in 0..bar_width {
                    spans.push(Span::styled(
                        glyph.to_string(),
                        metric_style(metric, focus, theme),
                    ));
                    used += 1;
                }
            }
            let padding = group_width.saturating_sub(used);
            if padding > 0 {
                spans.push(Span::raw(" ".repeat(padding)));
            }
        }
        lines.push(Line::from(spans));
    }
    lines.push(axis_line(points, group_width));
    lines.truncate(height);
    lines
}

fn metric_value(stats: &AggregateStats, metric: AnalyticsMetric) -> u64 {
    match metric {
        AnalyticsMetric::Records => stats.records,
        AnalyticsMetric::Words => stats.words,
        AnalyticsMetric::Duration => stats.duration_ms / 1000,
        AnalyticsMetric::AsrAudio => stats.asr_audio_ms / 1000,
    }
}

fn legend_line(
    metrics: &[AnalyticsMetric],
    focus: AnalyticsMetric,
    theme: &TuiTheme,
) -> Line<'static> {
    let mut spans = Vec::with_capacity(metrics.len() * 2);
    for metric in metrics.iter().copied() {
        spans.push(Span::styled(
            metric_short_label(metric),
            metric_style(metric, focus, theme),
        ));
    }
    spans.push(Span::raw(" "));
    spans.push(Span::raw(crate::i18n::tr(
        "tui.history.analytics.legend",
        &[
            (
                "records",
                metric_short_label(AnalyticsMetric::Records).to_string(),
            ),
            (
                "words",
                metric_short_label(AnalyticsMetric::Words).to_string(),
            ),
            (
                "duration",
                metric_short_label(AnalyticsMetric::Duration).to_string(),
            ),
            (
                "asr_audio",
                metric_short_label(AnalyticsMetric::AsrAudio).to_string(),
            ),
        ],
    )));
    Line::from(spans)
}

fn value_line(
    points: &[AnalyticsPoint],
    focus: AnalyticsMetric,
    group_width: usize,
) -> Line<'static> {
    let mut line = String::with_capacity(points.len() * group_width);
    for point in points {
        line.push_str(&format!(
            "{:^group_width$}",
            truncate_display(&compact_metric_value(&point.stats, focus), group_width)
        ));
    }
    Line::raw(line)
}

fn axis_line(points: &[AnalyticsPoint], group_width: usize) -> Line<'static> {
    let mut line = String::with_capacity(points.len() * group_width);
    let stride = label_stride(points.len());
    for (idx, point) in points.iter().enumerate() {
        let label = if should_show_axis_label(idx, points.len(), stride) {
            compact_axis_label(&point.key)
        } else {
            String::new()
        };
        line.push_str(&format!(
            "{:^group_width$}",
            truncate_display(&label, group_width)
        ));
    }
    Line::raw(line)
}

fn label_stride(count: usize) -> usize {
    if count <= 8 {
        1
    } else if count <= 14 {
        2
    } else if count <= 31 {
        5
    } else {
        10
    }
}

fn should_show_axis_label(index: usize, count: usize, stride: usize) -> bool {
    index == 0 || index + 1 == count || index.is_multiple_of(stride)
}

fn compact_axis_label(key: &str) -> String {
    if let Some((_, day)) = key.split_once('-') {
        day.trim_start_matches('0').to_string()
    } else {
        key.to_string()
    }
}

fn metric_short_label(metric: AnalyticsMetric) -> &'static str {
    match metric {
        AnalyticsMetric::Records => "R",
        AnalyticsMetric::Words => "W",
        AnalyticsMetric::Duration => "D",
        AnalyticsMetric::AsrAudio => "A",
    }
}

fn metric_style(metric: AnalyticsMetric, focus: AnalyticsMetric, theme: &TuiTheme) -> Style {
    let color = match metric {
        AnalyticsMetric::Records => ui::info(theme),
        AnalyticsMetric::Words => ui::success(theme),
        AnalyticsMetric::Duration => ui::warning(theme),
        AnalyticsMetric::AsrAudio => ui::accent(theme),
    };
    let style = Style::default().fg(color);
    if metric == focus {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

fn compact_metric_value(stats: &AggregateStats, metric: AnalyticsMetric) -> String {
    match metric {
        AnalyticsMetric::Records => compact_u64(stats.records),
        AnalyticsMetric::Words => compact_u64(stats.words),
        AnalyticsMetric::Duration => compact_seconds(stats.duration_ms / 1000),
        AnalyticsMetric::AsrAudio => compact_seconds(stats.asr_audio_ms / 1000),
    }
}

fn compact_u64(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{}m", value / 1_000_000)
    } else if value >= 1_000 {
        format!("{}k", value / 1_000)
    } else {
        value.to_string()
    }
}

fn compact_seconds(seconds: u64) -> String {
    if seconds >= 3600 {
        format!("{}h", seconds / 3600)
    } else if seconds >= 60 {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

fn scaled_height(value: u64, max: u64, chart_height: usize) -> usize {
    if value == 0 {
        0
    } else {
        ((value as usize).saturating_mul(chart_height)).div_ceil(max as usize)
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

fn cleanup_sheet_lines(theme: &TuiTheme, mode: &CleanupMode) -> Vec<Line<'static>> {
    match mode {
        CleanupMode::Selecting(select) => cleanup_selecting_lines(theme, select),
        CleanupMode::Scanning { filter } => vec![
            cleanup_scope_line(theme, filter.scope),
            cleanup_window_line(theme, filter.window),
            Line::from(""),
            Line::from(crate::t!("tui.history.cleanup.loading")),
        ],
        CleanupMode::Preview { preview, confirm } => {
            let mut lines = vec![
                cleanup_scope_line(theme, preview.filter.scope),
                cleanup_window_line(theme, preview.filter.window),
                Line::from(""),
            ];
            lines.extend(cleanup_preview_lines(theme, preview));
            lines.push(Line::from(""));
            lines.push(cleanup_confirm_line(theme, preview, *confirm));
            lines
        }
        CleanupMode::Executing { preview } => vec![
            cleanup_scope_line(theme, preview.filter.scope),
            cleanup_window_line(theme, preview.filter.window),
            Line::from(""),
            Line::from(cleanup_executing_label(preview.filter.scope)),
        ],
        CleanupMode::Done { result } => cleanup_done_lines(theme, result),
        CleanupMode::Failed { message } => vec![
            Line::styled(
                crate::t!("tui.history.cleanup.failed"),
                Style::default().fg(ui::warning(theme)),
            ),
            Line::from(message.clone()),
        ],
    }
}

fn cleanup_selecting_lines(theme: &TuiTheme, select: &CleanupSelect) -> Vec<Line<'static>> {
    use crate::tui::history::{
        CLEANUP_DAY_CHOICES, CLEANUP_HOUR_CHOICES, CLEANUP_OLDER_DAY_CHOICES, WINDOW_CHOICES,
    };
    let mut lines = vec![
        Line::from(crate::t!("tui.history.cleanup.select")),
        Line::from(""),
    ];
    let scope_base = if select.scope_active {
        Style::default()
            .fg(ui::accent(theme))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(ui::muted(theme))
    };
    lines.push(Line::from(vec![
        Span::styled(if select.scope_active { "▶ " } else { "  " }, scope_base),
        Span::styled(crate::t!("tui.history.cleanup.scope"), scope_base),
        Span::styled(": ", scope_base),
        Span::styled(cleanup_scope_label(select.scope), scope_base),
        option_comment(
            theme,
            format!(
                "{} / {}",
                crate::t!("tui.history.cleanup.scope_audio"),
                crate::t!("tui.history.cleanup.scope_record")
            ),
        ),
    ]));
    lines.push(Line::from(""));
    for choice in WINDOW_CHOICES {
        let active = !select.scope_active && choice == select.choice;
        let base = if active {
            Style::default()
                .fg(ui::accent(theme))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ui::muted(theme))
        };
        let marker = if active { "▶ " } else { "  " };
        let mut spans = vec![Span::styled(marker, base)];
        match choice {
            WindowChoice::Custom => {
                spans.push(Span::styled(
                    format!("{} ", crate::t!("tui.history.cleanup.window_custom")),
                    base,
                ));
                // Reverse-highlight the active date field only on the active row.
                let focus = (active && select.custom_editing).then_some(select.field);
                spans.extend(date_field_spans(theme, select.from, focus, true, base));
                spans.push(Span::styled(" ~ ", base));
                spans.extend(date_field_spans(theme, select.to, focus, false, base));
                spans.push(option_comment(
                    theme,
                    if select.custom_editing {
                        crate::t!("tui.hint.cleanup_adjust")
                    } else {
                        crate::t!("tui.hint.cleanup_edit")
                    },
                ));
            }
            WindowChoice::LastHours => spans.extend(window_choice_spans(
                theme,
                choice,
                select,
                base,
                &CLEANUP_HOUR_CHOICES,
            )),
            WindowChoice::LastDays => spans.extend(window_choice_spans(
                theme,
                choice,
                select,
                base,
                &CLEANUP_DAY_CHOICES,
            )),
            WindowChoice::OlderThan => spans.extend(window_choice_spans(
                theme,
                choice,
                select,
                base,
                &CLEANUP_OLDER_DAY_CHOICES,
            )),
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn cleanup_scope_label(scope: CleanupScope) -> String {
    match scope {
        CleanupScope::AudioOnly => crate::t!("tui.history.cleanup.scope_audio"),
        CleanupScope::RecordAndAudio => crate::t!("tui.history.cleanup.scope_record"),
    }
}

fn window_choice_spans(
    theme: &TuiTheme,
    choice: WindowChoice,
    select: &CleanupSelect,
    base: Style,
    options: &[u32],
) -> Vec<Span<'static>> {
    let value_style = Style::default().fg(ui::fg(theme));
    match choice {
        WindowChoice::LastHours => editable_number_spans(
            crate::t!("tui.history.cleanup.window_last_hours_prefix"),
            select.hours(),
            crate::t!("tui.history.cleanup.window_hours_suffix"),
            base,
            value_style,
            option_comment(theme, format_options(options)),
        ),
        WindowChoice::LastDays => editable_number_spans(
            crate::t!("tui.history.cleanup.window_last_days_prefix"),
            select.days(),
            crate::t!("tui.history.cleanup.window_days_suffix"),
            base,
            value_style,
            option_comment(theme, format_options(options)),
        ),
        WindowChoice::OlderThan => editable_number_spans(
            crate::t!("tui.history.cleanup.window_older_prefix"),
            select.older_days(),
            crate::t!("tui.history.cleanup.window_days_suffix"),
            base,
            value_style,
            option_comment(theme, format_options(options)),
        ),
        WindowChoice::Custom => vec![Span::styled(
            crate::t!("tui.history.cleanup.window_custom"),
            base,
        )],
    }
}

fn editable_number_spans(
    prefix: String,
    value: u32,
    suffix: String,
    base: Style,
    value_style: Style,
    comment: Span<'static>,
) -> Vec<Span<'static>> {
    vec![
        Span::styled(prefix, base),
        Span::styled(value.to_string(), value_style),
        Span::styled(suffix, base),
        comment,
    ]
}

fn option_comment(theme: &TuiTheme, text: String) -> Span<'static> {
    Span::styled(
        format!("  [{text}]"),
        Style::default().fg(ui::accent(theme)),
    )
}

fn format_options(options: &[u32]) -> String {
    options
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join("/")
}

/// Spans for one date as `YYYY-MM-DD`, reverse-highlighting the field that
/// matches `focus` (when this date is the `from`/`to` side being edited).
fn date_field_spans(
    _theme: &TuiTheme,
    date: time::Date,
    focus: Option<crate::tui::history::RangeField>,
    is_from: bool,
    base: Style,
) -> Vec<Span<'static>> {
    use crate::tui::history::DateField;
    let hl = |part: DateField, text: String| {
        let active = focus.is_some_and(|f| f.is_from() == is_from && f.part() == part);
        let style = if active {
            base.add_modifier(Modifier::REVERSED)
        } else {
            base
        };
        Span::styled(text, style)
    };
    vec![
        hl(DateField::Year, format!("{:04}", date.year())),
        Span::styled("-", base),
        hl(DateField::Month, format!("{:02}", u8::from(date.month()))),
        Span::styled("-", base),
        hl(DateField::Day, format!("{:02}", date.day())),
    ]
}

fn cleanup_window_line(theme: &TuiTheme, window: CleanupWindow) -> Line<'static> {
    kv_line(
        theme,
        crate::t!("tui.history.cleanup.window"),
        cleanup_window_label(window),
        ui::fg(theme),
    )
}

fn cleanup_scope_line(theme: &TuiTheme, scope: CleanupScope) -> Line<'static> {
    kv_line(
        theme,
        crate::t!("tui.history.cleanup.scope"),
        cleanup_scope_label(scope),
        ui::fg(theme),
    )
}

fn cleanup_window_label(window: CleanupWindow) -> String {
    match window {
        CleanupWindow::All => crate::t!("tui.history.cleanup.window_all"),
        CleanupWindow::LastHours(h) => crate::i18n::tr(
            "tui.history.cleanup.window_last_hours",
            &[("n", h.to_string())],
        ),
        CleanupWindow::LastDays(d) => crate::i18n::tr(
            "tui.history.cleanup.window_last_days",
            &[("n", d.to_string())],
        ),
        CleanupWindow::OlderThanDays(n) => crate::i18n::tr(
            "tui.history.cleanup.window_older",
            &[("days", n.to_string())],
        ),
        CleanupWindow::Range { from, to } => crate::i18n::tr(
            "tui.history.cleanup.window_range",
            &[("from", iso_date(from)), ("to", iso_date(to))],
        ),
    }
}

/// `YYYY-MM-DD` for display.
pub(super) fn iso_date(date: time::Date) -> String {
    time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
        .ok()
        .and_then(|fmt| date.format(&fmt).ok())
        .unwrap_or_else(|| date.to_string())
}

/// The [Cancel] / [Delete] button row on the preview screen; the focused button
/// is reverse-highlighted. Delete is styled with the warning color.
fn cleanup_confirm_line(
    theme: &TuiTheme,
    preview: &CleanupPreview,
    confirm: crate::tui::history::CleanupConfirm,
) -> Line<'static> {
    use crate::tui::history::CleanupConfirm;
    let focused = |target: CleanupConfirm, base: Color| {
        if confirm == target {
            Style::default()
                .fg(base)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default().fg(base)
        }
    };
    // With no deletable audio there is nothing to delete; offer only Cancel.
    if preview.ids.is_empty() {
        return Line::styled(
            format!(" {} ", crate::t!("tui.hint.cleanup_cancel")),
            focused(CleanupConfirm::Cancel, ui::fg(theme)),
        );
    }
    Line::from(vec![
        Span::styled(
            format!(" {} ", crate::t!("tui.hint.cleanup_cancel")),
            focused(CleanupConfirm::Cancel, ui::fg(theme)),
        ),
        Span::raw("   "),
        Span::styled(
            format!(" {} ", cleanup_confirm_label(preview.filter.scope)),
            focused(CleanupConfirm::Delete, ui::warning(theme)),
        ),
    ])
}

fn cleanup_confirm_label(scope: CleanupScope) -> String {
    match scope {
        CleanupScope::AudioOnly => crate::t!("tui.history.cleanup.confirm_audio"),
        CleanupScope::RecordAndAudio => crate::t!("tui.history.cleanup.confirm_record"),
    }
}

fn cleanup_executing_label(scope: CleanupScope) -> String {
    match scope {
        CleanupScope::AudioOnly => crate::t!("tui.history.cleanup.executing_audio"),
        CleanupScope::RecordAndAudio => crate::t!("tui.history.cleanup.executing_record"),
    }
}

fn cleanup_preview_lines(theme: &TuiTheme, preview: &CleanupPreview) -> Vec<Line<'static>> {
    if preview.ids.is_empty() && preview.warnings.is_empty() {
        return vec![Line::from(crate::t!("tui.history.cleanup.empty"))];
    }
    let mut lines = vec![
        kv_line(
            theme,
            cleanup_records_label(preview.filter.scope),
            preview.ids.len().to_string(),
            ui::fg(theme),
        ),
        kv_line(
            theme,
            crate::t!("tui.history.cleanup.speech"),
            ui::format_duration(preview.audio_ms),
            ui::fg(theme),
        ),
        kv_line(
            theme,
            crate::t!("tui.history.cleanup.size"),
            format_bytes(preview.audio_bytes),
            ui::fg(theme),
        ),
    ];
    if let Some(oldest) = preview.oldest {
        lines.push(kv_line(
            theme,
            crate::t!("tui.history.cleanup.oldest"),
            format_local_time(oldest),
            ui::fg(theme),
        ));
    }
    if let Some(newest) = preview.newest {
        lines.push(kv_line(
            theme,
            crate::t!("tui.history.cleanup.newest"),
            format_local_time(newest),
            ui::fg(theme),
        ));
    }
    if !preview.warnings.is_empty() {
        lines.push(Line::styled(
            crate::i18n::tr(
                "tui.history.cleanup.warnings",
                &[("count", preview.warnings.len().to_string())],
            ),
            Style::default().fg(ui::warning(theme)),
        ));
    }
    lines
}

fn cleanup_records_label(scope: CleanupScope) -> String {
    match scope {
        CleanupScope::AudioOnly => crate::t!("tui.history.cleanup.records_audio"),
        CleanupScope::RecordAndAudio => crate::t!("tui.history.cleanup.records"),
    }
}

fn cleanup_done_lines(theme: &TuiTheme, result: &CleanupResult) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(crate::t!("tui.history.cleanup.done")),
        Line::from(""),
        kv_line(
            theme,
            crate::t!("tui.history.cleanup.deleted"),
            result.deleted.to_string(),
            ui::fg(theme),
        ),
    ];
    if result.missing > 0 {
        lines.push(kv_line(
            theme,
            crate::t!("tui.history.cleanup.missing"),
            result.missing.to_string(),
            ui::muted(theme),
        ));
    }
    if !result.errors.is_empty() {
        lines.push(Line::styled(
            crate::i18n::tr(
                "tui.history.cleanup.errors",
                &[("count", result.errors.len().to_string())],
            ),
            Style::default().fg(ui::warning(theme)),
        ));
    }
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
            &time::format_description::parse_borrowed::<2>(
                "[year]-[month]-[day] [hour]:[minute]:[second]",
            )
            .expect("valid static time format"),
        )
        .unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod cleanup_render_tests {
    use super::*;
    use crate::history::{
        CleanupFilter, CleanupIssue, CleanupResult, CleanupScope, CleanupWarning,
    };
    use crate::tui::history::{CleanupConfirm, CleanupMode, CleanupSelect, WindowChoice};

    fn theme() -> TuiTheme {
        TuiTheme::default()
    }

    fn text_of(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn preview(ids: Vec<&str>, bytes: u64, warnings: usize) -> CleanupPreview {
        CleanupPreview {
            filter: CleanupFilter {
                scope: CleanupScope::AudioOnly,
                window: CleanupWindow::OlderThanDays(30),
            },
            ids: ids.into_iter().map(str::to_string).collect(),
            audio_bytes: bytes,
            audio_ms: 65_000,
            oldest: None,
            newest: None,
            warnings: (0..warnings)
                .map(|i| CleanupWarning {
                    id: format!("01H{i}"),
                    issue: CleanupIssue::Conflict,
                })
                .collect(),
        }
    }

    #[test]
    fn window_labels_are_localized() {
        assert!(!cleanup_window_label(CleanupWindow::All).is_empty());
        assert!(cleanup_window_label(CleanupWindow::OlderThanDays(30)).contains("30"));
    }

    #[test]
    fn preview_sheet_shows_count_size_warnings_and_buttons() {
        let lines = cleanup_sheet_lines(
            &theme(),
            &CleanupMode::Preview {
                preview: preview(vec!["01HA", "01HB"], 2048, 1),
                confirm: CleanupConfirm::Cancel,
            },
        );
        let text = text_of(&lines);
        assert!(text.contains('2'), "record count: {text}");
        assert!(text.contains("KiB"), "byte size: {text}");
        assert!(
            text.contains("(unsafe audio)") || text.contains("危险音频"),
            "warnings: {text}"
        );
        assert!(
            text.contains(&crate::t!("tui.hint.cleanup_cancel")),
            "cancel: {text}"
        );
        assert!(
            text.contains(&crate::t!("tui.hint.cleanup_confirm")),
            "delete: {text}"
        );
    }

    #[test]
    fn selecting_sheet_shows_all_windows_with_custom_having_dates() {
        let mut select = CleanupSelect::new(&[]);
        select.choice = WindowChoice::Custom;
        let lines = cleanup_sheet_lines(&theme(), &CleanupMode::Selecting(select));
        let text = text_of(&lines);
        assert!(text.contains("▶"), "selection marker: {text}");
        // The custom row should contain a date-like pattern (YYYY-MM-DD).
        assert!(
            text.matches('-').count() >= 2,
            "date dashes in custom row: {text}"
        );
    }

    #[test]
    fn older_than_shows_day_count() {
        let select = CleanupSelect::new(&[]); // OlderThan, days=30
        let lines = cleanup_sheet_lines(&theme(), &CleanupMode::Selecting(select));
        assert!(text_of(&lines).contains("30"));
    }

    #[test]
    fn selecting_sheet_shows_options_as_comments_not_value_color() {
        let select = CleanupSelect::new(&[]);
        let theme = theme();
        let lines = cleanup_sheet_lines(&theme, &CleanupMode::Selecting(select));
        let text = text_of(&lines);

        assert!(text.contains("14/30/60/90/180"), "{text}");
        assert!(
            lines.iter().flat_map(|line| line.spans.iter()).any(|span| {
                span.content.as_ref() == "30" && span.style.fg != Some(ui::accent(&theme))
            }),
            "current value should not use accent color: {text}"
        );
    }

    #[test]
    fn empty_preview_shows_no_match_message() {
        let lines = cleanup_sheet_lines(
            &theme(),
            &CleanupMode::Preview {
                preview: preview(vec![], 0, 0),
                confirm: crate::tui::history::CleanupConfirm::Cancel,
            },
        );
        assert!(text_of(&lines).contains(&crate::t!("tui.history.cleanup.empty")));
    }

    #[test]
    fn done_sheet_reports_deleted_and_errors() {
        let lines = cleanup_sheet_lines(
            &theme(),
            &CleanupMode::Done {
                result: CleanupResult {
                    requested: 3,
                    deleted: 2,
                    missing: 1,
                    errors: vec![crate::history::CleanupError {
                        id: "01HX".to_string(),
                        issue: CleanupIssue::Symlink,
                    }],
                },
            },
        );
        let text = text_of(&lines);
        assert!(text.contains(&crate::t!("tui.history.cleanup.done")));
        assert!(text.contains('2'));
    }
}
