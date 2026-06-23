use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::ipc::protocol::WireState;
use crate::platform::capability::{CapabilityStatus, CapabilityStatusKind, PlatformKind};
use crate::state::{AudioMeter, SessionPhase};
use crate::tui::status::StatusPage;
use crate::tui::ui;

pub(super) fn render_status(frame: &mut Frame, page: &StatusPage, area: Rect, theme: &TuiTheme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Min(5),
        ])
        .split(area);

    let elapsed_ms = page.current_elapsed_ms();
    let app_label = page
        .app_name
        .clone()
        .or_else(|| page.app.clone())
        .unwrap_or_else(|| crate::t!("tui.no_active_app"));
    let bundle = page.app.clone().unwrap_or_else(|| "-".to_string());
    let provider = page
        .session_meta
        .as_ref()
        .map(|meta| meta.provider.as_str())
        .unwrap_or("-");
    let chain = page
        .session_meta
        .as_ref()
        .map(|meta| meta.chain.as_str())
        .unwrap_or("-");
    let header = status_header_lines(
        page, theme, &app_label, &bundle, provider, chain, elapsed_ms,
    );
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
            page,
            theme,
            chunks[1].width.saturating_sub(9) as usize,
        ))
        .block(
            Block::default()
                .title(crate::i18n::tr(
                    "tui.status.input_title",
                    &[
                        ("provider", provider.to_string()),
                        ("chain", chain.to_string()),
                    ],
                ))
                .borders(Borders::ALL),
        ),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(platform_capability_lines(
            &crate::overlay::renderer_capabilities(),
            theme,
            chunks[2].height.saturating_sub(2) as usize,
        ))
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Platform").borders(Borders::ALL)),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new(live_speech_lines(
            page,
            theme,
            chunks[3].width.saturating_sub(2) as usize,
            chunks[3].height.saturating_sub(2) as usize,
        ))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(crate::t!("tui.live_speech"))
                .borders(Borders::ALL),
        ),
        chunks[3],
    );
}

pub(super) fn status_header_lines(
    page: &StatusPage,
    theme: &TuiTheme,
    app_label: &str,
    bundle: &str,
    provider: &str,
    chain: &str,
    elapsed_ms: u64,
) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::styled(
                format!("{:<10}", status_label(page)),
                Style::default()
                    .fg(phase_color(page, theme))
                    .add_modifier(Modifier::BOLD),
            ),
            label_span(" app ", theme),
            value_span(app_label.to_string(), ui::accent(theme)),
            label_span(" bundle ", theme),
            value_span(bundle.to_string(), ui::muted(theme)),
        ]),
        Line::from(vec![
            label_span("id ", theme),
            value_span(recording_id_label(page), ui::info(theme)),
            label_span(" duration ", theme),
            value_span(format_duration(elapsed_ms), ui::warning(theme)),
            label_span(" words ", theme),
            value_span(page.words.to_string(), ui::success(theme)),
        ]),
        Line::from(vec![
            label_span("asr ", theme),
            value_span(provider.to_string(), ui::info(theme)),
            label_span(" chain ", theme),
            value_span(chain.to_string(), ui::highlight(theme)),
        ]),
    ]
}

fn recording_id_label(page: &StatusPage) -> String {
    page.recording_id
        .clone()
        .unwrap_or_else(|| crate::t!("tui.no_active_recording"))
}

pub(super) fn meter_lines(page: &StatusPage, theme: &TuiTheme, width: usize) -> Vec<Line<'static>> {
    if !matches!(page.state, WireState::Recording | WireState::Stopping) && page.meters.is_empty() {
        return vec![
            Line::from(vec![
                Span::styled("Audio  ", Style::default().fg(ui::muted(theme))),
                Span::styled("idle", Style::default().fg(ui::muted(theme))),
            ]),
            Line::from(vec![
                Span::styled("       ", Style::default().fg(ui::muted(theme))),
                Span::styled("────", Style::default().fg(ui::muted(theme))),
            ]),
            Line::from(vec![
                Span::styled("VAD    ", Style::default().fg(ui::muted(theme))),
                Span::styled("idle", Style::default().fg(ui::muted(theme))),
            ]),
        ];
    }
    let width = width.max(16);
    let start = page.meters.len().saturating_sub(width);
    let meters = &page.meters[start..];
    vec![
        Line::from(vec![
            Span::styled("Peak   ", Style::default().fg(ui::muted(theme))),
            meter_span(audio_upper(meters), ui::accent(theme)),
        ]),
        Line::from(vec![
            Span::styled("RMS    ", Style::default().fg(ui::muted(theme))),
            meter_span(audio_lower(meters), ui::info(theme)),
        ]),
        Line::from(vec![
            Span::styled("VAD    ", Style::default().fg(ui::muted(theme))),
            vad_spans(meters, theme),
        ]),
    ]
}

pub(super) fn platform_capability_lines(
    renderer_capabilities: &[CapabilityStatus],
    theme: &TuiTheme,
    max_lines: usize,
) -> Vec<Line<'static>> {
    let capabilities = merged_platform_capabilities(renderer_capabilities);
    platform_capability_lines_from(&capabilities, theme, max_lines)
}

pub(super) fn platform_capability_lines_from(
    capabilities: &[CapabilityStatus],
    theme: &TuiTheme,
    max_lines: usize,
) -> Vec<Line<'static>> {
    let platform = capabilities
        .first()
        .map(|status| status.platform)
        .unwrap_or_else(PlatformKind::current);
    let counts = capability_counts(capabilities);

    let mut lines = vec![Line::from(vec![
        label_span("platform ", theme),
        value_span(platform.as_str().to_string(), ui::info(theme)),
        label_span(" available=", theme),
        value_span(counts.available.to_string(), ui::success(theme)),
        label_span(" unsupported=", theme),
        value_span(
            counts.unsupported.to_string(),
            status_count_color(counts.unsupported, theme),
        ),
        label_span(" unavailable=", theme),
        value_span(
            counts.unavailable.to_string(),
            status_count_color(counts.unavailable, theme),
        ),
        label_span(" partial=", theme),
        value_span(
            counts.partial.to_string(),
            status_count_color(counts.partial, theme),
        ),
        label_span(" degraded=", theme),
        value_span(
            counts.degraded.to_string(),
            status_count_color(counts.degraded, theme),
        ),
        label_span(" unknown=", theme),
        value_span(
            counts.unknown.to_string(),
            status_count_color(counts.unknown, theme),
        ),
    ])];

    lines.extend(
        capabilities
            .iter()
            .filter(|capability| capability.status != CapabilityStatusKind::Available)
            .map(|capability| capability_detail_line(capability, theme)),
    );

    lines.truncate(max_lines.max(1));
    lines
}

fn merged_platform_capabilities(
    renderer_capabilities: &[CapabilityStatus],
) -> Vec<CapabilityStatus> {
    let mut capabilities = crate::platform::capability::current_platform_capabilities();
    for renderer_capability in renderer_capabilities {
        if let Some(existing) = capabilities
            .iter_mut()
            .find(|capability| capability.id == renderer_capability.id)
        {
            *existing = renderer_capability.clone();
        } else {
            capabilities.push(renderer_capability.clone());
        }
    }
    capabilities
}

#[derive(Default)]
struct CapabilityCounts {
    available: usize,
    unsupported: usize,
    unavailable: usize,
    partial: usize,
    degraded: usize,
    unknown: usize,
}

fn capability_counts(capabilities: &[CapabilityStatus]) -> CapabilityCounts {
    let mut counts = CapabilityCounts::default();
    for capability in capabilities {
        match capability.status {
            CapabilityStatusKind::Available => counts.available += 1,
            CapabilityStatusKind::Unsupported => counts.unsupported += 1,
            CapabilityStatusKind::Unavailable => counts.unavailable += 1,
            CapabilityStatusKind::Partial => counts.partial += 1,
            CapabilityStatusKind::Degraded => counts.degraded += 1,
            CapabilityStatusKind::Unknown => counts.unknown += 1,
        }
    }
    counts
}

fn capability_detail_line(capability: &CapabilityStatus, theme: &TuiTheme) -> Line<'static> {
    let mut spans = vec![
        value_span(capability.id.as_str().to_string(), ui::accent(theme)),
        label_span(" ", theme),
        value_span(
            capability.status.as_str().to_string(),
            capability_status_color(capability.status, theme),
        ),
        label_span(" backend=", theme),
        value_span(capability.backend.to_string(), ui::info(theme)),
        label_span(" reason=", theme),
        value_span(capability.reason.to_string(), ui::muted(theme)),
    ];
    if let Some(next_step) = capability.next_step {
        spans.push(label_span(" next=", theme));
        spans.push(value_span(next_step.to_string(), ui::warning(theme)));
    }
    Line::from(spans)
}

fn capability_status_color(status: CapabilityStatusKind, theme: &TuiTheme) -> Color {
    match status {
        CapabilityStatusKind::Available => ui::success(theme),
        CapabilityStatusKind::Unsupported | CapabilityStatusKind::Unavailable => ui::error(theme),
        CapabilityStatusKind::Partial | CapabilityStatusKind::Degraded => ui::warning(theme),
        CapabilityStatusKind::Unknown => ui::muted(theme),
    }
}

fn status_count_color(count: usize, theme: &TuiTheme) -> Color {
    if count == 0 {
        ui::success(theme)
    } else {
        ui::warning(theme)
    }
}

pub(super) fn live_speech_lines(
    page: &StatusPage,
    theme: &TuiTheme,
    width: usize,
    max_lines: usize,
) -> Vec<Line<'static>> {
    let width = width.max(16);
    let max_lines = max_lines.max(1);
    let segments = page.segments.join("");
    let mut all_lines = wrap_spans(
        vec![
            Span::styled(segments.clone(), Style::default().fg(ui::segment(theme))),
            Span::styled(page.partial.clone(), Style::default().fg(ui::accent(theme))),
        ],
        width,
    );
    let truncated = all_lines.len() > max_lines;
    if truncated {
        let prefix_width = 4;
        let first_width = width.saturating_sub(prefix_width).max(1);
        let keep_width = first_width + width * max_lines.saturating_sub(1);
        let partial_width = display_width(&page.partial);
        let (segment_tail, partial_tail) = if partial_width >= keep_width {
            (
                String::new(),
                take_display_suffix(&page.partial, keep_width),
            )
        } else {
            (
                take_display_suffix(&segments, keep_width - partial_width),
                page.partial.clone(),
            )
        };
        all_lines = wrap_spans_with_widths(
            vec![
                Span::styled(segment_tail, Style::default().fg(ui::segment(theme))),
                Span::styled(partial_tail, Style::default().fg(ui::accent(theme))),
            ],
            first_width,
            width,
        );
        let first = all_lines.first_mut().expect("tail has at least one line");
        first.spans.insert(
            0,
            Span::styled("... ".to_string(), Style::default().fg(ui::muted(theme))),
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

fn status_label(page: &StatusPage) -> String {
    match page.session_phase {
        Some(SessionPhase::Active) => crate::t!("tui.state_recording"),
        Some(SessionPhase::Idle) => crate::t!("tui.state_idle"),
        Some(SessionPhase::Stopping) => crate::t!("tui.state_stopping"),
        None => state_label(page.state),
    }
}

fn phase_color(page: &StatusPage, theme: &TuiTheme) -> Color {
    match page.session_phase {
        Some(SessionPhase::Active) => ui::error(theme),
        Some(SessionPhase::Idle) => ui::info(theme),
        Some(SessionPhase::Stopping) => ui::warning(theme),
        None => match page.state {
            WireState::Idle => ui::success(theme),
            WireState::Recording => ui::error(theme),
            WireState::Stopping => ui::warning(theme),
            WireState::Error => ui::error(theme),
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

pub(super) fn display_width(value: &str) -> usize {
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

pub(super) fn audio_upper(meters: &[AudioMeter]) -> String {
    meters.iter().map(|meter| upper_level(meter.peak)).collect()
}

pub(super) fn audio_lower(meters: &[AudioMeter]) -> String {
    meters.iter().map(|meter| lower_level(meter.rms)).collect()
}

fn vad_spans(meters: &[AudioMeter], theme: &TuiTheme) -> Span<'static> {
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

pub(super) fn upper_level(value: f32) -> char {
    const LEVELS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    level_char(value, LEVELS)
}

pub(super) fn lower_level(value: f32) -> char {
    const LEVELS: &[char] = &['▔', '▇', '▆', '▅', '▄', '▃', '▂', '▁'];
    level_char(value, LEVELS)
}

fn level_char(value: f32, levels: &[char]) -> char {
    let value = value.clamp(0.0, 1.0);
    let idx = (value * (levels.len() - 1) as f32).round() as usize;
    levels[idx]
}

fn label_span(text: impl Into<String>, theme: &TuiTheme) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(ui::muted(theme)))
}

fn value_span(text: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(color))
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
