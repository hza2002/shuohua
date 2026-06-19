use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Wrap};
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::ipc::protocol::WireState;
use crate::state::SessionPhase;
use crate::tui::page::Page as _;
use crate::tui::{App, Confirm, HistoryDetail, Page};

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
    pub fn error(theme: &TuiTheme) -> Color {
        rgb(theme.error)
    }
    pub fn info(theme: &TuiTheme) -> Color {
        rgb(theme.info)
    }
    pub fn highlight(theme: &TuiTheme) -> Color {
        rgb(theme.highlight)
    }
    pub fn segment(theme: &TuiTheme) -> Color {
        rgb(theme.segment)
    }
}

pub fn render(frame: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let tabs = Tabs::new([
        crate::t!("tui.tab_status"),
        crate::t!("tui.tab_history"),
        crate::t!("tui.tab_settings"),
    ])
    .select(app.page.index())
    .style(Style::default().fg(ui::muted(&app.theme)))
    .highlight_style(
        Style::default()
            .fg(ui::highlight(&app.theme))
            .add_modifier(Modifier::BOLD),
    )
    .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(tabs, root[0]);

    match app.page {
        Page::Status => render_status(frame, app, root[1]),
        Page::History => render_history(frame, app, root[1]),
        Page::Settings => render_settings(frame, app, root[1]),
    }

    frame.render_widget(Paragraph::new(footer_text(app)), root[2]);
}

fn footer_text(app: &App) -> String {
    let page_keys = match app.page {
        Page::Status => crate::t!("tui.footer_status"),
        Page::History => crate::t!("tui.footer_history"),
        Page::Settings => crate::t!("tui.footer_settings"),
    };
    crate::t!(
        "tui.footer",
        page_keys = page_keys,
        status = app.status.clone()
    )
}

fn render_status(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Min(5),
        ])
        .split(area);

    let elapsed_ms = app.current_elapsed_ms();
    let app_label = app
        .app_name
        .clone()
        .or_else(|| app.app.clone())
        .unwrap_or_else(|| crate::t!("tui.no_active_app"));
    let bundle = app.app.clone().unwrap_or_else(|| "-".to_string());
    let provider = app
        .session_meta
        .as_ref()
        .map(|meta| meta.provider.as_str())
        .unwrap_or("-");
    let chain = app
        .session_meta
        .as_ref()
        .map(|meta| meta.chain.as_str())
        .unwrap_or("-");
    let header = status_header_lines(app, &app_label, &bundle, provider, chain, elapsed_ms);
    frame.render_widget(
        Paragraph::new(header).block(
            Block::default()
                .title(crate::t!("tui.current"))
                .borders(Borders::ALL),
        ),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(meter_lines(app, chunks[1].width.saturating_sub(9) as usize)).block(
            Block::default()
                .title(format!("Input  ASR: {provider} -> {chain}"))
                .borders(Borders::ALL),
        ),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(live_speech_lines(
            app,
            chunks[2].width.saturating_sub(2) as usize,
            chunks[2].height.saturating_sub(2) as usize,
        ))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(crate::t!("tui.live_speech"))
                .borders(Borders::ALL),
        ),
        chunks[2],
    );
}

fn status_header_lines(
    app: &App,
    app_label: &str,
    bundle: &str,
    provider: &str,
    chain: &str,
    elapsed_ms: u64,
) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::styled(
                format!("{:<10}", status_label(app)),
                Style::default()
                    .fg(phase_color(app))
                    .add_modifier(Modifier::BOLD),
            ),
            label_span(" app ", &app.theme),
            value_span(app_label.to_string(), ui::accent(&app.theme)),
            label_span(" bundle ", &app.theme),
            value_span(bundle.to_string(), ui::muted(&app.theme)),
        ]),
        Line::from(vec![
            label_span("id ", &app.theme),
            value_span(recording_id_label(app), ui::info(&app.theme)),
            label_span(" duration ", &app.theme),
            value_span(format_duration(elapsed_ms), ui::warning(&app.theme)),
            label_span(" words ", &app.theme),
            value_span(app.words.to_string(), ui::success(&app.theme)),
        ]),
        Line::from(vec![
            label_span("asr ", &app.theme),
            value_span(provider.to_string(), ui::info(&app.theme)),
            label_span(" chain ", &app.theme),
            value_span(chain.to_string(), ui::highlight(&app.theme)),
        ]),
    ]
}

fn recording_id_label(app: &App) -> String {
    app.recording_id
        .clone()
        .unwrap_or_else(|| crate::t!("tui.no_active_recording"))
}

fn meter_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    if !matches!(app.state, WireState::Recording | WireState::Stopping) && app.meters.is_empty() {
        return vec![
            Line::from(vec![
                Span::styled("Audio  ", Style::default().fg(ui::muted(&app.theme))),
                Span::styled("idle", Style::default().fg(ui::muted(&app.theme))),
            ]),
            Line::from(vec![
                Span::styled("       ", Style::default().fg(ui::muted(&app.theme))),
                Span::styled("────", Style::default().fg(ui::muted(&app.theme))),
            ]),
            Line::from(vec![
                Span::styled("VAD    ", Style::default().fg(ui::muted(&app.theme))),
                Span::styled("idle", Style::default().fg(ui::muted(&app.theme))),
            ]),
        ];
    }
    let width = width.max(16);
    let start = app.meters.len().saturating_sub(width);
    let meters = &app.meters[start..];
    vec![
        Line::from(vec![
            Span::styled("Peak   ", Style::default().fg(ui::muted(&app.theme))),
            meter_span(audio_upper(meters), ui::accent(&app.theme)),
        ]),
        Line::from(vec![
            Span::styled("RMS    ", Style::default().fg(ui::muted(&app.theme))),
            meter_span(audio_lower(meters), ui::info(&app.theme)),
        ]),
        Line::from(vec![
            Span::styled("VAD    ", Style::default().fg(ui::muted(&app.theme))),
            vad_spans(meters, &app.theme),
        ]),
    ]
}

fn live_speech_lines(app: &App, width: usize, max_lines: usize) -> Vec<Line<'static>> {
    let width = width.max(16);
    let max_lines = max_lines.max(1);
    let segments = app.segments.join("");
    let mut all_lines = wrap_spans(
        vec![
            Span::styled(
                segments.clone(),
                Style::default().fg(ui::segment(&app.theme)),
            ),
            Span::styled(
                app.partial.clone(),
                Style::default().fg(ui::accent(&app.theme)),
            ),
        ],
        width,
    );
    let truncated = all_lines.len() > max_lines;
    if truncated {
        let prefix_width = 4;
        let first_width = width.saturating_sub(prefix_width).max(1);
        let keep_width = first_width + width * max_lines.saturating_sub(1);
        let partial_width = display_width(&app.partial);
        let (segment_tail, partial_tail) = if partial_width >= keep_width {
            (String::new(), take_display_suffix(&app.partial, keep_width))
        } else {
            (
                take_display_suffix(&segments, keep_width - partial_width),
                app.partial.clone(),
            )
        };
        all_lines = wrap_spans_with_widths(
            vec![
                Span::styled(segment_tail, Style::default().fg(ui::segment(&app.theme))),
                Span::styled(partial_tail, Style::default().fg(ui::accent(&app.theme))),
            ],
            first_width,
            width,
        );
        let first = all_lines.first_mut().expect("tail has at least one line");
        first.spans.insert(
            0,
            Span::styled(
                "... ".to_string(),
                Style::default().fg(ui::muted(&app.theme)),
            ),
        );
    }
    all_lines
}

fn take_display_suffix(value: &str, max_width: usize) -> String {
    let mut width = 0usize;
    let mut chars = Vec::new();
    for ch in value.chars().rev() {
        let ch_width = char_display_width(ch);
        if width + ch_width > max_width {
            break;
        }
        width += ch_width;
        chars.push(ch);
    }
    chars.into_iter().rev().collect()
}

fn status_label(app: &App) -> String {
    match app.session_phase {
        Some(SessionPhase::Active) => crate::t!("tui.state_recording"),
        Some(SessionPhase::Idle) => crate::t!("tui.state_idle"),
        Some(SessionPhase::Stopping) => crate::t!("tui.state_stopping"),
        None => state_label(app.state),
    }
}

fn phase_color(app: &App) -> Color {
    match app.session_phase {
        Some(SessionPhase::Active) => ui::error(&app.theme),
        Some(SessionPhase::Idle) => ui::info(&app.theme),
        Some(SessionPhase::Stopping) => ui::warning(&app.theme),
        None => match app.state {
            WireState::Idle => ui::success(&app.theme),
            WireState::Recording => ui::error(&app.theme),
            WireState::Stopping => ui::warning(&app.theme),
            WireState::Error => ui::error(&app.theme),
        },
    }
}
fn wrap_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Line<'static>> {
    wrap_spans_with_widths(spans, width, width)
}

fn wrap_spans_with_widths(
    spans: Vec<Span<'static>>,
    first_width: usize,
    next_width: usize,
) -> Vec<Line<'static>> {
    let mut lines = vec![Vec::<Span<'static>>::new()];
    let mut col = 0usize;
    let mut line_width = first_width.max(1);
    for span in spans {
        let style = span.style;
        for ch in span.content.chars() {
            let ch_width = char_display_width(ch);
            if col + ch_width > line_width && col > 0 {
                lines.push(Vec::new());
                col = 0;
                line_width = next_width.max(1);
            }
            lines
                .last_mut()
                .expect("at least one line")
                .push(Span::styled(ch.to_string(), style));
            col += ch_width;
        }
    }
    lines.into_iter().map(Line::from).collect()
}

fn display_width(value: &str) -> usize {
    value.chars().map(char_display_width).sum()
}

fn char_display_width(ch: char) -> usize {
    if ch.is_ascii() {
        1
    } else {
        2
    }
}

fn meter_span(text: String, color: Color) -> Span<'static> {
    Span::styled(text, Style::default().fg(color))
}

fn audio_upper(meters: &[crate::state::AudioMeter]) -> String {
    meters.iter().map(|meter| upper_level(meter.peak)).collect()
}

fn audio_lower(meters: &[crate::state::AudioMeter]) -> String {
    meters.iter().map(|meter| lower_level(meter.rms)).collect()
}

fn vad_spans(meters: &[crate::state::AudioMeter], theme: &TuiTheme) -> Span<'static> {
    let mut text = String::with_capacity(meters.len());
    let mut active = false;
    for meter in meters {
        let probability = meter.vad_probability.unwrap_or_else(|| {
            if meter.vad_speech.unwrap_or(false) {
                1.0
            } else {
                0.0
            }
        });
        active |= meter.vad_speech.unwrap_or(probability >= 0.5);
        text.push(upper_level(probability));
    }
    let color = if active {
        ui::success(theme)
    } else {
        ui::muted(theme)
    };
    Span::styled(text, Style::default().fg(color))
}

fn upper_level(value: f32) -> char {
    const LEVELS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    level_char(value, LEVELS)
}

fn lower_level(value: f32) -> char {
    const LEVELS: &[char] = &['▔', '▇', '▆', '▅', '▄', '▃', '▂', '▁'];
    level_char(value, LEVELS)
}

fn level_char(value: f32, levels: &[char]) -> char {
    let value = value.clamp(0.0, 1.0);
    let idx = (value * (levels.len() - 1) as f32).round() as usize;
    levels[idx]
}

fn render_history(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(0)])
        .split(area);
    let summary = HistorySummary::from(app);
    let search = if app.searching {
        format!("/{}_", app.search)
    } else if app.search.is_empty() {
        crate::t!("tui.search_prompt")
    } else {
        format!("/{}", app.search)
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(search),
            history_stats_line(&summary, &app.theme),
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
    let records = app.filtered_history();
    let items: Vec<ListItem> = records
        .iter()
        .enumerate()
        .map(|(idx, record)| {
            ListItem::new(history_list_line(app, record, idx == app.selected_history))
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
        .get(app.selected_history)
        .map(|record| history_detail_text(app, record, app.history_detail))
        .unwrap_or_else(|| vec![Line::from(crate::t!("tui.no_history_selected"))]);
    let selected = if let Some(confirm) = &app.confirm {
        let mut lines = vec![Line::styled(
            confirm_text(confirm),
            Style::default()
                .fg(ui::warning(&app.theme))
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
                .title(history_detail_title(app.history_detail))
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
    app: &App,
    record: &crate::state::history::HistoryRecord,
    selected: bool,
) -> Line<'static> {
    let marker = if selected { "> " } else { "  " };
    let audio = history_audio_marker(app, record);
    let audio_color = if app.audio_info_for_record(record).exists() {
        ui::success(&app.theme)
    } else {
        ui::muted(&app.theme)
    };
    Line::from(vec![
        Span::styled(
            marker.to_string(),
            Style::default()
                .fg(if selected {
                    ui::accent(&app.theme)
                } else {
                    ui::muted(&app.theme)
                })
                .add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::styled(
            format!("{:<19}", format_local_time(record.started_at)),
            Style::default().fg(ui::muted(&app.theme)),
        ),
        Span::raw(" "),
        Span::styled(
            format!(
                "{:<10}",
                truncate_display(&short_app_label(record.app.as_deref()), 10)
            ),
            Style::default().fg(ui::accent(&app.theme)),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:>5}", format_duration(record.duration_ms)),
            Style::default().fg(ui::warning(&app.theme)),
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
    app: &App,
    record: &crate::state::history::HistoryRecord,
    detail: HistoryDetail,
) -> Vec<Line<'static>> {
    match detail {
        HistoryDetail::Details => history_details_lines(app, record),
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

fn history_audio_marker(app: &App, record: &crate::state::history::HistoryRecord) -> String {
    if app.audio_info_for_record(record).exists() {
        crate::t!("tui.history.audio.present_short")
    } else {
        crate::t!("tui.history.audio.missing_short")
    }
}

fn history_details_lines(
    app: &App,
    record: &crate::state::history::HistoryRecord,
) -> Vec<Line<'static>> {
    let info = app.audio_info_for_record(record);
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
            "status",
            format!("{:?}", record.status),
            ui::success(&app.theme),
        ),
        kv_line(
            "app",
            short_app_label(record.app.as_deref()),
            ui::accent(&app.theme),
        ),
        kv_line(
            "started",
            format_local_time(record.started_at),
            ui::fg(&app.theme),
        ),
        kv_line(
            "duration",
            format_duration(record.duration_ms),
            ui::warning(&app.theme),
        ),
        kv_line(
            "words",
            record.text_stats().words.to_string(),
            ui::accent(&app.theme),
        ),
        kv_line("asr", record.asr.provider.clone(), ui::info(&app.theme)),
        kv_line("pipeline", pipeline_summary(record), ui::fg(&app.theme)),
        kv_line(
            "audio",
            status,
            if info.exists() {
                ui::success(&app.theme)
            } else {
                ui::muted(&app.theme)
            },
        ),
        kv_line(
            crate::t!("tui.history.audio.size"),
            size,
            ui::fg(&app.theme),
        ),
        kv_line(
            crate::t!("tui.history.audio.mtime"),
            modified,
            ui::fg(&app.theme),
        ),
        Line::from(""),
        kv_line("text", "", ui::fg(&app.theme)),
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

fn format_system_time(value: std::time::SystemTime) -> String {
    let Ok(duration) = value.duration_since(std::time::UNIX_EPOCH) else {
        return "-".to_string();
    };
    let Ok(datetime) = time::OffsetDateTime::from_unix_timestamp(duration.as_secs() as i64) else {
        return "-".to_string();
    };
    format_local_time(datetime)
}

fn pipeline_summary(record: &crate::state::history::HistoryRecord) -> String {
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

struct HistorySummary {
    total: usize,
    shown: usize,
    total_duration_ms: u64,
    avg_duration_ms: u64,
    total_words: usize,
}

impl HistorySummary {
    fn from(app: &App) -> Self {
        let filtered = app.filtered_history();
        let total_duration_ms = app
            .history
            .iter()
            .map(|record| record.duration_ms)
            .sum::<u64>();
        let total_words = app
            .history
            .iter()
            .map(|record| record.text_stats().words)
            .sum::<usize>();
        let avg_duration_ms = if app.history.is_empty() {
            0
        } else {
            total_duration_ms / app.history.len() as u64
        };
        Self {
            total: app.history.len(),
            shown: filtered.len(),
            total_duration_ms,
            avg_duration_ms,
            total_words,
        }
    }
}

fn state_label(state: WireState) -> String {
    match state {
        WireState::Idle => crate::t!("tui.state_idle"),
        WireState::Recording => crate::t!("tui.state_recording"),
        WireState::Stopping => crate::t!("tui.state_stopping"),
        WireState::Error => crate::t!("tui.state_error"),
    }
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

fn render_settings(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    app.configure.render(frame, area, &app.theme, &app.status);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform_levels_use_low_and_high_blocks() {
        assert_eq!(upper_level(0.0), '▁');
        assert_eq!(upper_level(1.0), '█');
        assert_eq!(lower_level(0.0), '▔');
        assert_eq!(lower_level(1.0), '▁');
    }

    #[test]
    fn audio_lines_render_one_char_per_meter() {
        let meters = vec![
            crate::state::AudioMeter {
                rms: 0.0,
                peak: 0.0,
                clipped: false,
                vad_probability: Some(0.0),
                vad_speech: Some(false),
            },
            crate::state::AudioMeter {
                rms: 1.0,
                peak: 1.0,
                clipped: true,
                vad_probability: Some(1.0),
                vad_speech: Some(true),
            },
        ];

        assert_eq!(audio_upper(&meters).chars().count(), 2);
        assert_eq!(audio_lower(&meters).chars().count(), 2);
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
    fn footer_only_shows_history_actions_on_history_page() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.page = Page::Status;
        assert!(!footer_text(&app).contains("open audio"));

        app.page = Page::History;
        let footer = footer_text(&app);
        assert!(footer.contains("open audio") || footer.contains("打开音频"));
    }

    #[test]
    fn status_header_includes_colored_asr_metadata() {
        crate::i18n::init("en-US");
        let app = App::new();

        let lines = status_header_lines(&app, "Ghostty", "com.mitchellh.ghostty", "apple", "-", 0);
        let text = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(text.contains("Ghostty"));
        assert!(text.contains("asr apple"));
        assert!(text.contains("chain -"));
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn live_speech_keeps_tail_when_space_is_limited() {
        let mut app = App::new();
        app.segments = vec!["abcdefghijklmnopqrstuvwxyz".to_string()];
        app.partial = "0123456789".to_string();

        let line = live_speech_lines(&app, 10, 1);
        let text = line[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(text.starts_with("... "));
        assert!(text.ends_with("456789"));
        assert!(display_width(&text) <= 16);
    }

    #[test]
    fn live_speech_keeps_tail_for_wide_cjk_text() {
        let mut app = App::new();
        app.segments = vec!["这是很长很长的一段已经定型的语音识别文本".to_string()];
        app.partial = "最新的部分".to_string();

        let line = live_speech_lines(&app, 16, 1);
        let text = line[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(text.starts_with("... "));
        assert!(text.ends_with("最新的部分"));
        assert!(display_width(&text) <= 16);
    }
}
