use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Wrap};
use ratatui::Frame;

use crate::ipc::protocol::WireState;
use crate::tui::{App, HistoryDetail, Page};

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
    .style(Style::default().fg(Color::Gray))
    .highlight_style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
    .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(tabs, root[0]);

    match app.page {
        Page::Status => render_status(frame, app, root[1]),
        Page::History => render_history(frame, app, root[1]),
        Page::Settings => render_settings(frame, app, root[1]),
    }

    frame.render_widget(
        Paragraph::new(crate::t!("tui.footer", status = app.status.clone())),
        root[2],
    );
}

fn render_status(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Min(5),
            Constraint::Length(5),
        ])
        .split(area);

    let state_color = match app.state {
        WireState::Idle => Color::Green,
        WireState::Recording => Color::Red,
        WireState::Stopping => Color::Yellow,
        WireState::Error => Color::LightRed,
    };
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
    let header = vec![
        Line::from(vec![
            Span::styled(
                state_label(app.state),
                Style::default()
                    .fg(state_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::raw(format!("{}  ", format_duration(elapsed_ms))),
            Span::raw(format!("{} words  ", app.words)),
            Span::raw(format!("{app_label} ({bundle})")),
        ]),
        Line::from(vec![
            Span::styled("id ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!("{}  ", recording_id_label(app))),
            Span::styled("asr ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!("{provider}  ")),
            Span::styled("chain ", Style::default().fg(Color::DarkGray)),
            Span::raw(chain.to_string()),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(header).block(
            Block::default()
                .title(crate::t!("tui.current"))
                .borders(Borders::ALL),
        ),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(meter_lines(
            app,
            chunks[1].width.saturating_sub(10) as usize,
        ))
        .block(Block::default().title("Input").borders(Borders::ALL)),
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

    frame.render_widget(
        Paragraph::new(pipeline_lines(app))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(crate::t!("tui.pipeline"))
                    .borders(Borders::ALL),
            ),
        chunks[3],
    );
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
                Span::styled("Audio  ", Style::default().fg(Color::DarkGray)),
                Span::styled("idle", Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::styled("       ", Style::default().fg(Color::DarkGray)),
                Span::styled("────", Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::styled("VAD    ", Style::default().fg(Color::DarkGray)),
                Span::styled("idle", Style::default().fg(Color::DarkGray)),
            ]),
        ];
    }
    let width = width.clamp(16, 200);
    let start = app.meters.len().saturating_sub(width);
    let meters = &app.meters[start..];
    vec![
        Line::from(vec![
            Span::styled("Audio  ", Style::default().fg(Color::DarkGray)),
            meter_span(audio_upper(meters), Color::Cyan),
            Span::styled("  peak", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled("       ", Style::default().fg(Color::DarkGray)),
            meter_span(audio_lower(meters), Color::Blue),
            Span::styled("  rms", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled("VAD    ", Style::default().fg(Color::DarkGray)),
            vad_spans(meters),
            Span::styled("  probability", Style::default().fg(Color::DarkGray)),
        ]),
    ]
}

fn live_speech_lines(app: &App, width: usize, max_lines: usize) -> Vec<Line<'static>> {
    let max_chars = width.max(16) * max_lines.max(1);
    let segments = app.segments.join("");
    let combined = format!("{segments}{}", app.partial);
    let total_chars = combined.chars().count();
    let prefix = if total_chars > max_chars { "... " } else { "" };
    let keep = max_chars.saturating_sub(prefix.chars().count());
    let drop_chars = total_chars.saturating_sub(keep);
    let segment_chars = segments.chars().count();
    let segment_tail = if drop_chars < segment_chars {
        take_char_range(&segments, drop_chars, segment_chars)
    } else {
        String::new()
    };
    let partial_drop = drop_chars.saturating_sub(segment_chars);
    let partial_tail = take_char_range(&app.partial, partial_drop, app.partial.chars().count());
    vec![Line::from(vec![
        Span::styled(prefix.to_string(), Style::default().fg(Color::DarkGray)),
        Span::styled(segment_tail, Style::default().fg(Color::Gray)),
        Span::styled(partial_tail, Style::default().fg(Color::Cyan)),
    ])]
}

fn take_char_range(value: &str, start: usize, end: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    let start = start.min(chars.len());
    let end = end.min(chars.len()).max(start);
    chars[start..end].iter().collect()
}

fn pipeline_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(meta) = app.session_meta.as_ref() {
        lines.push(Line::from(vec![
            Span::styled("ASR  ", Style::default().fg(Color::DarkGray)),
            Span::styled(meta.provider.clone(), Style::default().fg(Color::Cyan)),
        ]));
        for step in meta.chain.split(" → ").flat_map(|part| part.split(" -> ")) {
            lines.push(Line::from(vec![
                Span::styled("POST ", Style::default().fg(Color::DarkGray)),
                Span::raw(step.to_string()),
            ]));
        }
    }
    if !app.pipeline.is_empty() {
        lines.push(Line::from(""));
        lines.extend(app.pipeline.iter().map(|step| {
            Line::from(Span::styled(
                step.clone(),
                Style::default().fg(Color::Green),
            ))
        }));
    }
    if lines.is_empty() {
        lines.push(Line::from(crate::t!("tui.no_pipeline_steps")));
    }
    lines
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

fn vad_spans(meters: &[crate::state::AudioMeter]) -> Span<'static> {
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
        Color::Green
    } else {
        Color::DarkGray
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
            Line::from(crate::t!(
                "tui.history_stats_line",
                shown = summary.shown,
                total = summary.total,
                duration = format_duration(summary.total_duration_ms),
                words = summary.total_words,
                avg = format_duration(summary.avg_duration_ms),
            )),
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
            let marker = if idx == app.selected_history {
                "> "
            } else {
                "  "
            };
            let app_name = short_app_label(record.app.as_deref());
            ListItem::new(format!(
                "{marker}{}  {}  {}  {}",
                format_local_time(record.started_at),
                app_name,
                format_duration(record.duration_ms),
                record.text.replace('\n', " ")
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
        .get(app.selected_history)
        .map(|record| history_detail_text(record, app.history_detail))
        .unwrap_or_else(|| crate::t!("tui.no_history_selected"));
    frame.render_widget(
        Paragraph::new(selected).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(history_detail_title(app.history_detail))
                .borders(Borders::ALL),
        ),
        body[1],
    );
}

fn history_detail_title(detail: HistoryDetail) -> &'static str {
    match detail {
        HistoryDetail::Final => "final",
        HistoryDetail::Asr => "asr raw",
        HistoryDetail::Pipeline => "pipeline",
        HistoryDetail::Sessions => "sessions",
        HistoryDetail::Error => "error",
        HistoryDetail::Json => "json",
    }
}

fn history_detail_text(
    record: &crate::state::history::HistoryRecord,
    detail: HistoryDetail,
) -> String {
    match detail {
        HistoryDetail::Final => format!(
            "status: {:?}\napp: {}\nstarted: {}\nduration: {}\nwords: {}\nasr: {}  audio {}\npipeline: {}\n\n{}",
            record.status,
            short_app_label(record.app.as_deref()),
            format_local_time(record.started_at),
            format_duration(record.duration_ms),
            record.text_stats().words,
            record.asr.provider,
            format_duration(record.asr.audio_ms),
            pipeline_summary(record),
            record.text
        ),
        HistoryDetail::Asr => format!(
            "provider: {}\naudio: {}\nstarted: {}\n\n{}",
            record.asr.provider,
            format_duration(record.asr.audio_ms),
            format_local_time(record.started_at),
            record.asr.text
        ),
        HistoryDetail::Pipeline => {
            if record.pipeline.is_empty() {
                return "no pipeline steps".to_string();
            }
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
                .join("\n\n")
        }
        HistoryDetail::Sessions => {
            if record.asr.sessions.is_empty() {
                return "no ASR sessions".to_string();
            }
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
                .join("\n\n")
        }
        HistoryDetail::Error => record
            .error
            .as_ref()
            .map(|error| format!("{}: {}", error.kind, error.msg))
            .unwrap_or_else(|| "no error".to_string()),
        HistoryDetail::Json => serde_json::to_string_pretty(record)
            .unwrap_or_else(|e| format!("failed to render json: {e}")),
    }
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
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(5),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(app.config_path.as_str()).block(
            Block::default()
                .title(crate::t!("tui.config_path"))
                .borders(Borders::ALL),
        ),
        chunks[0],
    );
    let rows = if app.settings_rows.is_empty() {
        "no config rows found".to_string()
    } else {
        app.settings_rows
            .iter()
            .map(|row| {
                format!(
                    "{:<7} {:<28} {:<28} {}",
                    row.group, row.key, row.value, row.source
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    frame.render_widget(
        Paragraph::new(rows)
            .wrap(Wrap { trim: false })
            .block(Block::default().title("settings").borders(Borders::ALL)),
        chunks[1],
    );
    frame.render_widget(
        Paragraph::new(format!(
            "{}\n{}",
            crate::t!("tui.doctor_m5"),
            "read-only: edit TOML files directly for prompts, API keys, and complex chains"
        ))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(crate::t!("tui.doctor"))
                .borders(Borders::ALL),
        ),
        chunks[2],
    );
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
    }
}
