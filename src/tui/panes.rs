use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Wrap};
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::ipc::protocol::WireState;
use crate::state::SessionPhase;
use crate::tui::{
    App, ConfigureFocus, ConfigureModule, Confirm, HistoryDetail, LlmWizardStep, Page,
};

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
    if app.llm_wizard.is_some() {
        render_configure_wizard(frame, app, area);
        return;
    }

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(24),
            Constraint::Percentage(44),
            Constraint::Percentage(56),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(configure_module_nav_lines(app)).block(
            configure_block(app, ConfigureFocus::Modules)
                .title(crate::t!("tui.configure.modules"))
                .borders(Borders::ALL),
        ),
        body[0],
    );

    if app.configure_module == ConfigureModule::Overview {
        render_configure_overview(
            frame,
            app,
            Rect::new(
                body[1].x,
                body[1].y,
                body[1].width + body[2].width,
                body[1].height,
            ),
        );
    } else if app.configure_module == ConfigureModule::Main {
        frame.render_widget(
            Paragraph::new(configure_item_lines(app))
                .wrap(Wrap { trim: false })
                .block(
                    configure_block(app, ConfigureFocus::Items)
                        .title(focused_title(
                            app,
                            ConfigureFocus::Items,
                            configure_module_title(app.configure_module),
                        ))
                        .borders(Borders::ALL),
                ),
            Rect::new(
                body[1].x,
                body[1].y,
                body[1].width + body[2].width,
                body[1].height,
            ),
        );
    } else {
        frame.render_widget(
            Paragraph::new(configure_item_lines(app))
                .wrap(Wrap { trim: false })
                .block(
                    configure_block(app, ConfigureFocus::Items)
                        .title(focused_title(
                            app,
                            ConfigureFocus::Items,
                            configure_module_title(app.configure_module),
                        ))
                        .borders(Borders::ALL),
                ),
            body[1],
        );
        frame.render_widget(
            Paragraph::new(configure_detail_lines(app))
                .wrap(Wrap { trim: false })
                .block(
                    configure_block(app, ConfigureFocus::Items)
                        .title(crate::t!("tui.configure.detail"))
                        .borders(Borders::ALL),
                ),
            body[2],
        );
    }
}

fn render_configure_wizard(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(4)])
        .split(area);
    frame.render_widget(
        Paragraph::new(configure_wizard_text(app))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(crate::t!("tui.configure.wizard.title"))
                    .borders(Borders::ALL),
            ),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(configure_status_lines(app))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(crate::t!("tui.configure.status"))
                    .borders(Borders::ALL),
            ),
        chunks[1],
    );
}

fn render_configure_overview(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(0)])
        .split(area);
    frame.render_widget(
        Paragraph::new(configure_overview_lines(app))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(crate::t!("tui.configure.main"))
                    .borders(Borders::ALL),
            ),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(configure_main_grouped_lines(app))
            .wrap(Wrap { trim: false })
            .block(
                configure_block(app, ConfigureFocus::Items)
                    .title(configure_module_title(ConfigureModule::Main))
                    .borders(Borders::ALL),
            ),
        chunks[1],
    );
}

fn configure_selected_row(app: &App) -> Option<&crate::tui::settings::SettingsRow> {
    let module = app.configure_module.inventory_module();
    app.settings_rows
        .iter()
        .filter(|row| row.group == module.label())
        .nth(app.selected_settings)
}

fn configure_hint_text(app: &App) -> String {
    if app.llm_wizard.is_some() {
        crate::t!("tui.configure.wizard.hint")
    } else if app.configure_module == ConfigureModule::PostProcessor {
        crate::t!("tui.configure.refresh_hint_post")
    } else {
        crate::t!("tui.configure.refresh_hint")
    }
}

fn configure_module_nav_lines(app: &App) -> Vec<Line<'static>> {
    all_configure_modules()
        .into_iter()
        .map(|module| {
            let selected = module == app.configure_module;
            let count = module_entry_count(app, module);
            let marker = if selected {
                if app.configure_focus == ConfigureFocus::Modules {
                    "> "
                } else {
                    "* "
                }
            } else {
                "  "
            };
            let style = if selected {
                Style::default()
                    .fg(ui::accent(&app.theme))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ui::segment(&app.theme))
            };
            Line::from(vec![
                Span::styled(marker, style),
                Span::styled(configure_module_title(module), style),
                Span::raw(" "),
                Span::styled(
                    format!("{count:>2}"),
                    Style::default().fg(ui::muted(&app.theme)),
                ),
            ])
        })
        .collect()
}

fn focused_title(app: &App, focus: ConfigureFocus, title: String) -> String {
    if app.configure_focus == focus {
        format!("> {title}")
    } else {
        title
    }
}

fn configure_block(app: &App, focus: ConfigureFocus) -> Block<'static> {
    if app.configure_focus == focus {
        Block::default().border_style(Style::default().fg(ui::accent(&app.theme)))
    } else {
        Block::default()
            .border_style(Style::default().fg(ui::muted(&app.theme)))
            .title_style(Style::default().fg(ui::muted(&app.theme)))
    }
}

fn all_configure_modules() -> Vec<ConfigureModule> {
    vec![
        ConfigureModule::Overview,
        ConfigureModule::Profile,
        ConfigureModule::AsrProvider,
        ConfigureModule::PostProcessor,
    ]
}

fn module_entry_count(app: &App, module: ConfigureModule) -> usize {
    let label = if module == ConfigureModule::Overview {
        ConfigureModule::Main.inventory_module().label()
    } else {
        module.inventory_module().label()
    };
    app.settings_rows
        .iter()
        .filter(|row| row.group == label)
        .count()
}

fn configure_wizard_text(app: &App) -> String {
    let Some(wizard) = &app.llm_wizard else {
        return String::new();
    };
    let template_lines = wizard
        .templates
        .iter()
        .enumerate()
        .map(|(idx, id)| {
            let marker =
                if wizard.step == LlmWizardStep::Template && idx == wizard.selected_template {
                    ">"
                } else {
                    " "
                };
            format!("{marker} {id}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{}\n{}\n\n{}\n{}\n{}\n{}\n{}\n{}\n\n{}",
        crate::t!("tui.configure.wizard.title"),
        wizard_step_label(wizard.step),
        template_lines,
        wizard_field_line(
            LlmWizardStep::FileId,
            wizard.step,
            crate::t!("tui.configure.wizard.file_id"),
            &wizard.draft.file_id
        ),
        wizard_field_line(
            LlmWizardStep::ProviderName,
            wizard.step,
            crate::t!("tui.configure.wizard.provider_name"),
            &wizard.draft.provider_name
        ),
        wizard_field_line(
            LlmWizardStep::Format,
            wizard.step,
            crate::t!("tui.configure.wizard.format"),
            &wizard.draft.format
        ),
        wizard_field_line(
            LlmWizardStep::BaseUrl,
            wizard.step,
            crate::t!("tui.configure.wizard.base_url"),
            &wizard.draft.base_url
        ),
        wizard_field_line(
            LlmWizardStep::Model,
            wizard.step,
            crate::t!("tui.configure.wizard.model"),
            &wizard.draft.model
        ),
        crate::t!("tui.configure.wizard.no_profile_attach")
    )
}

fn wizard_step_label(step: LlmWizardStep) -> String {
    let key = match step {
        LlmWizardStep::Template => "tui.configure.wizard.step_template",
        LlmWizardStep::FileId => "tui.configure.wizard.step_file_id",
        LlmWizardStep::ProviderName => "tui.configure.wizard.step_provider_name",
        LlmWizardStep::Format => "tui.configure.wizard.step_format",
        LlmWizardStep::BaseUrl => "tui.configure.wizard.step_base_url",
        LlmWizardStep::Model => "tui.configure.wizard.step_model",
    };
    crate::i18n::tr(key, &[])
}

fn wizard_field_line(
    field: LlmWizardStep,
    current: LlmWizardStep,
    label: String,
    value: &str,
) -> String {
    let marker = if field == current { ">" } else { " " };
    format!("{marker} {label}: {value}")
}

fn configure_item_lines(app: &App) -> Vec<Line<'static>> {
    if matches!(
        app.configure_module,
        ConfigureModule::Profile | ConfigureModule::PostProcessor | ConfigureModule::AsrProvider
    ) {
        return configure_source_lines(app);
    }
    if app.configure_module == ConfigureModule::Main {
        return configure_main_grouped_lines(app);
    }
    configure_field_lines(app.configure_rows_for_current_module(), None, &app.theme)
}

fn configure_main_grouped_lines(app: &App) -> Vec<Line<'static>> {
    let rows = app
        .settings_rows
        .iter()
        .filter(|row| row.group == ConfigureModule::Main.inventory_module().label())
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return vec![Line::from(crate::t!("tui.configure.no_entries"))];
    }
    let mut lines = Vec::new();
    let mut current_section = String::new();
    for row in rows {
        let (section, item_key) = split_main_display_key(&row.display_key);
        if section != current_section {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            current_section = section.clone();
            lines.push(Line::styled(
                section,
                Style::default()
                    .fg(ui::accent(&app.theme))
                    .add_modifier(Modifier::BOLD),
            ));
        }
        lines.push(configure_field_line(
            row,
            Some(&item_key),
            false,
            &app.theme,
        ));
    }
    lines
}

fn split_main_display_key(key: &str) -> (String, String) {
    key.split_once('.')
        .map(|(section, rest)| (section.to_string(), rest.to_string()))
        .unwrap_or_else(|| ("root".to_string(), key.to_string()))
}

fn configure_source_lines(app: &App) -> Vec<Line<'static>> {
    let sources = app.configure_sources_for_current_module();
    if sources.is_empty() {
        return vec![Line::from(crate::t!("tui.configure.no_entries"))];
    }
    sources
        .iter()
        .enumerate()
        .map(|(idx, source)| {
            let selected = idx == app.selected_settings;
            let row_count = app
                .configure_rows_for_current_module()
                .into_iter()
                .filter(|row| std::path::Path::new(&row.source) == source)
                .count();
            let marker_style = if selected {
                Style::default()
                    .fg(ui::accent(&app.theme))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ui::muted(&app.theme))
            };
            Line::from(vec![
                Span::styled(if selected { "> " } else { "  " }, marker_style),
                Span::styled(
                    format!("{row_count:>2}"),
                    Style::default().fg(ui::success(&app.theme)),
                ),
                Span::raw(" "),
                Span::styled(
                    source_name(source),
                    if selected {
                        Style::default()
                            .fg(ui::accent(&app.theme))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(ui::fg(&app.theme))
                    },
                ),
            ])
        })
        .collect()
}

fn configure_field_lines(
    rows: Vec<&crate::tui::settings::SettingsRow>,
    selected: Option<usize>,
    theme: &TuiTheme,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return vec![Line::from(crate::t!("tui.configure.no_entries"))];
    }
    rows.iter()
        .enumerate()
        .map(|(idx, row)| {
            let is_selected = selected.is_some_and(|selected| selected == idx);
            configure_field_line(row, None, is_selected, theme)
        })
        .collect()
}

fn configure_field_line(
    row: &crate::tui::settings::SettingsRow,
    key_override: Option<&str>,
    selected: bool,
    theme: &TuiTheme,
) -> Line<'static> {
    let display_key = key_override.unwrap_or(&row.display_key);
    Line::from(vec![
        Span::styled(
            if selected { "> " } else { "" }.to_string(),
            Style::default().fg(if selected {
                ui::accent(theme)
            } else {
                ui::muted(theme)
            }),
        ),
        Span::styled(
            status_glyph(row.status),
            Style::default().fg(status_color(row.status, theme)),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:<24}", truncate_display(display_key, 24)),
            Style::default().fg(if selected {
                ui::accent(theme)
            } else {
                ui::fg(theme)
            }),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:<24}", truncate_display(&compact_value(&row.value), 24)),
            Style::default().fg(ui::segment(theme)),
        ),
        Span::raw("  "),
        Span::styled(
            row.description_key
                .map(|key| crate::i18n::tr(key, &[]))
                .unwrap_or_default(),
            Style::default().fg(ui::muted(theme)),
        ),
    ])
}

fn compact_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn configure_detail_lines(app: &App) -> Vec<Line<'static>> {
    let rows = match app.configure_module {
        ConfigureModule::Profile
        | ConfigureModule::PostProcessor
        | ConfigureModule::AsrProvider => configure_selected_source_rows(app),
        _ => configure_selected_row(app)
            .map(|row| vec![row])
            .unwrap_or_default(),
    };
    if rows.is_empty() {
        return vec![Line::from(crate::t!("tui.configure.no_config_selected"))];
    }
    let source = rows
        .first()
        .map(|row| row.source.clone())
        .unwrap_or_else(|| "-".to_string());
    let mut lines = vec![
        kv_line("source path", source, ui::warning(&app.theme)),
        Line::from(""),
    ];
    lines.extend(configure_detail_field_lines(rows.clone(), &app.theme));
    lines
}

fn configure_detail_field_lines(
    rows: Vec<&crate::tui::settings::SettingsRow>,
    theme: &TuiTheme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for row in rows {
        let description = row
            .description_key
            .map(|key| crate::i18n::tr(key, &[]))
            .unwrap_or_default();
        if row.value.contains('\n') || display_width(&row.value) > 56 {
            lines.push(Line::from(vec![
                Span::styled(
                    row.display_key.clone(),
                    Style::default()
                        .fg(ui::accent(theme))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(description, Style::default().fg(ui::muted(theme))),
            ]));
            lines.extend(text_lines(row.value.clone()));
        } else {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:<24}", truncate_display(&row.display_key, 24)),
                    Style::default().fg(ui::accent(theme)),
                ),
                Span::styled(
                    format!("{:<24}", row.value.clone()),
                    Style::default().fg(ui::segment(theme)),
                ),
                Span::raw("  "),
                Span::styled(description, Style::default().fg(ui::muted(theme))),
            ]));
        }
    }
    lines
}

fn configure_selected_source_rows(app: &App) -> Vec<&crate::tui::settings::SettingsRow> {
    let Some(source) = app.selected_config_source() else {
        return Vec::new();
    };
    let module = app.configure_module.inventory_module();
    app.settings_rows
        .iter()
        .filter(|row| row.group == module.label() && std::path::Path::new(&row.source) == source)
        .collect()
}

fn configure_overview_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = vec![
        kv_line(
            "config root",
            app.config_path.clone(),
            ui::warning(&app.theme),
        ),
        Line::from(""),
        Line::from(vec![
            label_span("module", &app.theme),
            Span::raw("        "),
            label_span("items", &app.theme),
            Span::raw("  "),
            label_span("errors", &app.theme),
            Span::raw("  "),
            label_span("missing", &app.theme),
        ]),
    ];
    for module in all_configure_modules()
        .into_iter()
        .filter(|module| *module != ConfigureModule::Overview)
    {
        let label = module.inventory_module().label();
        let rows = app
            .settings_rows
            .iter()
            .filter(|row| row.group == label)
            .collect::<Vec<_>>();
        let errors = rows
            .iter()
            .filter(|row| row.status == crate::config::inventory::InventoryStatus::Error)
            .count();
        let missing = rows
            .iter()
            .filter(|row| row.status == crate::config::inventory::InventoryStatus::Missing)
            .count();
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<13}", configure_module_title(module)),
                Style::default().fg(ui::accent(&app.theme)),
            ),
            Span::styled(
                format!("{:>5}", rows.len()),
                Style::default().fg(ui::fg(&app.theme)),
            ),
            Span::styled(
                format!("{:>8}", errors),
                Style::default().fg(status_count_color(errors, &app.theme)),
            ),
            Span::styled(
                format!("{:>9}", missing),
                Style::default().fg(status_count_color(missing, &app.theme)),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.extend(configure_status_lines(app));
    lines
}

fn configure_status_lines(app: &App) -> Vec<Line<'static>> {
    vec![
        kv_line("doctor", doctor_status_value(app), ui::success(&app.theme)),
        kv_line("reload/status", app.status.clone(), ui::fg(&app.theme)),
        kv_line("actions", configure_hint_text(app), ui::muted(&app.theme)),
    ]
}

fn doctor_status_value(app: &App) -> String {
    match &app.doctor.status {
        Some(status) => status.clone(),
        None => crate::t!("tui.configure.doctor_not_run"),
    }
}

fn status_glyph(status: crate::config::inventory::InventoryStatus) -> &'static str {
    match status {
        crate::config::inventory::InventoryStatus::Ok => "ok",
        crate::config::inventory::InventoryStatus::Warning => "!!",
        crate::config::inventory::InventoryStatus::Error => "xx",
        crate::config::inventory::InventoryStatus::Missing => "--",
    }
}

fn status_color(status: crate::config::inventory::InventoryStatus, theme: &TuiTheme) -> Color {
    match status {
        crate::config::inventory::InventoryStatus::Ok => ui::success(theme),
        crate::config::inventory::InventoryStatus::Warning => ui::warning(theme),
        crate::config::inventory::InventoryStatus::Error => ui::error(theme),
        crate::config::inventory::InventoryStatus::Missing => ui::muted(theme),
    }
}

fn status_count_color(count: usize, theme: &TuiTheme) -> Color {
    if count == 0 {
        ui::muted(theme)
    } else {
        ui::error(theme)
    }
}

fn source_name(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path.display().to_string())
}

fn configure_module_title(module: ConfigureModule) -> String {
    match module {
        ConfigureModule::Overview => crate::t!("tui.configure.main"),
        ConfigureModule::Main => crate::t!("tui.configure.main"),
        ConfigureModule::Profile => crate::t!("tui.configure.profile"),
        ConfigureModule::AsrProvider => crate::t!("tui.configure.asr"),
        ConfigureModule::PostProcessor => crate::t!("tui.configure.post"),
    }
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

    #[test]
    fn configure_navigation_shows_module_counts() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.settings_rows = vec![crate::tui::settings::SettingsRow {
            group: "asr".to_string(),
            key: "apple.idle_pause".to_string(),
            display_key: "idle_pause".to_string(),
            value: "true".to_string(),
            source: "/tmp/shuohua/asr/apple.toml".to_string(),
            status: crate::config::inventory::InventoryStatus::Ok,
            description_key: Some("config.field.idle_pause.description"),
        }];
        app.configure_module = ConfigureModule::AsrProvider;

        let text = configure_module_nav_lines(&app)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(text.contains("> ASR"));
        assert!(text.contains("1"));
    }

    #[test]
    fn configure_item_list_keeps_source_out_of_dense_rows() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.configure_module = ConfigureModule::AsrProvider;
        app.settings_rows = vec![crate::tui::settings::SettingsRow {
            group: "asr".to_string(),
            key: "apple.idle_pause".to_string(),
            display_key: "idle_pause".to_string(),
            value: "true".to_string(),
            source: "/tmp/shuohua/asr/apple.toml".to_string(),
            status: crate::config::inventory::InventoryStatus::Ok,
            description_key: Some("config.field.idle_pause.description"),
        }];

        let text = configure_item_lines(&app)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(text.contains("apple"));
        assert!(!text.contains("apple.idle_pause"));
        assert!(!text.contains("true"));
        assert!(!text.contains("/tmp/shuohua/asr/apple.toml"));
    }

    #[test]
    fn configure_detail_uses_schema_description_and_source() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.configure_module = ConfigureModule::AsrProvider;
        app.settings_rows = vec![crate::tui::settings::SettingsRow {
            group: "asr".to_string(),
            key: "apple.idle_pause".to_string(),
            display_key: "idle_pause".to_string(),
            value: "true".to_string(),
            source: "/tmp/shuohua/asr/apple.toml".to_string(),
            status: crate::config::inventory::InventoryStatus::Ok,
            description_key: Some("config.field.idle_pause.description"),
        }];

        let text = configure_detail_lines(&app)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(text.contains("/tmp/shuohua/asr/apple.toml"));
        assert!(text.contains("pause and reopen ASR sessions"));
    }

    #[test]
    fn configure_main_uses_single_field_list() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.configure_module = ConfigureModule::Main;
        app.settings_rows = vec![crate::tui::settings::SettingsRow {
            group: "main".to_string(),
            key: "config.hotkey.trigger".to_string(),
            display_key: "hotkey.trigger".to_string(),
            value: "f16".to_string(),
            source: "/tmp/shuohua/config.toml".to_string(),
            status: crate::config::inventory::InventoryStatus::Ok,
            description_key: Some("config.field.hotkey.trigger.description"),
        }];

        let text = configure_item_lines(&app)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(text.contains("hotkey"));
        assert!(text.contains("trigger"));
        assert!(!text.contains("hotkey.trigger"));
        assert!(!text.contains("config.hotkey.trigger"));
        assert!(text.contains("f16"));
        assert!(!text.contains("/tmp/shuohua/config.toml"));
    }

    #[test]
    fn configure_main_groups_fields_by_section() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.configure_module = ConfigureModule::Main;
        app.settings_rows = vec![
            crate::tui::settings::SettingsRow {
                group: "main".to_string(),
                key: "config.overlay.position".to_string(),
                display_key: "overlay.position".to_string(),
                value: "bottom".to_string(),
                source: "/tmp/shuohua/config.toml".to_string(),
                status: crate::config::inventory::InventoryStatus::Ok,
                description_key: Some("config.field.overlay.position.description"),
            },
            crate::tui::settings::SettingsRow {
                group: "main".to_string(),
                key: "config.overlay.max_text_lines".to_string(),
                display_key: "overlay.max_text_lines".to_string(),
                value: "5".to_string(),
                source: "/tmp/shuohua/config.toml".to_string(),
                status: crate::config::inventory::InventoryStatus::Ok,
                description_key: Some("config.field.overlay.max_text_lines.description"),
            },
        ];

        let text = configure_main_grouped_lines(&app)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(text.contains("overlay"));
        assert!(text.contains("position"));
        assert!(text.contains("max_text_lines"));
        assert!(!text.contains("overlay.position"));
        assert!(!text.contains("overlay.max_text_lines"));
    }

    #[test]
    fn configure_main_still_renders_module_nav() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.configure_module = ConfigureModule::Overview;

        let text = configure_module_nav_lines(&app)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(text.contains("Main"));
        assert!(text.contains("Profile"));
        assert_eq!(text.matches("Main").count(), 1);
    }

    #[test]
    fn configure_overview_can_render_main_fields() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.configure_module = ConfigureModule::Overview;
        app.settings_rows = vec![crate::tui::settings::SettingsRow {
            group: "main".to_string(),
            key: "config.hotkey.trigger".to_string(),
            display_key: "hotkey.trigger".to_string(),
            value: "f16".to_string(),
            source: "/tmp/shuohua/config.toml".to_string(),
            status: crate::config::inventory::InventoryStatus::Ok,
            description_key: Some("config.field.hotkey.trigger.description"),
        }];

        let text = configure_field_lines(
            app.settings_rows
                .iter()
                .filter(|row| row.group == "main")
                .collect(),
            None,
            &app.theme,
        )
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

        assert!(text.contains("hotkey.trigger"));
        assert!(text.contains("f16"));
    }

    #[test]
    fn configure_profile_list_is_file_selection_and_detail_expands_fields() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.configure_module = ConfigureModule::Profile;
        app.settings_rows = vec![
            crate::tui::settings::SettingsRow {
                group: "profile".to_string(),
                key: "default.name".to_string(),
                display_key: "name".to_string(),
                value: "default".to_string(),
                source: "/tmp/shuohua/profile/default.toml".to_string(),
                status: crate::config::inventory::InventoryStatus::Ok,
                description_key: Some("config.field.name.description"),
            },
            crate::tui::settings::SettingsRow {
                group: "profile".to_string(),
                key: "coding.asr.provider".to_string(),
                display_key: "asr.provider".to_string(),
                value: "doubao".to_string(),
                source: "/tmp/shuohua/profile/coding.toml".to_string(),
                status: crate::config::inventory::InventoryStatus::Ok,
                description_key: Some("config.field.asr.provider.description"),
            },
            crate::tui::settings::SettingsRow {
                group: "profile".to_string(),
                key: "coding.post.chain".to_string(),
                display_key: "post.chain".to_string(),
                value: "[\"llm:deepseek\"]".to_string(),
                source: "/tmp/shuohua/profile/coding.toml".to_string(),
                status: crate::config::inventory::InventoryStatus::Ok,
                description_key: Some("config.field.post.chain.description"),
            },
        ];
        app.selected_settings = 0;

        let list = configure_item_lines(&app)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        let detail = configure_detail_lines(&app)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(list.contains("coding"));
        assert!(!list.contains("llm:deepseek"));
        assert!(detail.contains("/tmp/shuohua/profile/coding.toml"));
        assert!(detail.contains("asr.provider"));
        assert!(!detail.contains("coding.asr.provider"));
        assert!(detail.contains("llm:deepseek"));
        assert!(detail.contains("Provider name matching"));
        assert!(!detail.contains("reload/status"));
        assert!(!detail.contains("actions"));
    }

    #[test]
    fn configure_detail_preserves_multiline_values() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.configure_module = ConfigureModule::PostProcessor;
        app.settings_rows = vec![crate::tui::settings::SettingsRow {
            group: "post".to_string(),
            key: "cleanup.prompt".to_string(),
            display_key: "prompt".to_string(),
            value: "line one\nline two".to_string(),
            source: "/tmp/shuohua/post/llm/cleanup.toml".to_string(),
            status: crate::config::inventory::InventoryStatus::Ok,
            description_key: Some("config.field.prompt.description"),
        }];

        let detail = configure_detail_lines(&app)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(detail.contains("line one"));
        assert!(detail.contains("line two"));
    }
}
