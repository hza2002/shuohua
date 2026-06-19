use crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::ipc::protocol::{Event, WireState};
use crate::state::{AudioMeter, SessionMeta, SessionPhase};
use crate::tui::page::{KeyOutcome, Page};

pub const MAX_METER_HISTORY: usize = 1024;

#[derive(Debug)]
pub struct StatusPage {
    pub state: WireState,
    pub recording_id: Option<String>,
    pub started_at: Option<time::OffsetDateTime>,
    pub app: Option<String>,
    pub app_name: Option<String>,
    pub dur_ms: u64,
    pub words: u32,
    pub segments: Vec<String>,
    pub partial: String,
    pub pipeline: Vec<String>,
    pub session_meta: Option<SessionMeta>,
    pub session_phase: Option<SessionPhase>,
    pub meters: Vec<AudioMeter>,
    pub meter_width: usize,
}

impl StatusPage {
    pub fn new() -> Self {
        Self {
            state: WireState::Idle,
            recording_id: None,
            started_at: None,
            app: None,
            app_name: None,
            dur_ms: 0,
            words: 0,
            segments: Vec::new(),
            partial: String::new(),
            pipeline: Vec::new(),
            session_meta: None,
            session_phase: None,
            meters: Vec::new(),
            meter_width: 160,
        }
    }

    pub fn current_elapsed_ms(&self) -> u64 {
        if matches!(self.state, WireState::Recording | WireState::Stopping) {
            if let Some(started_at) = self.started_at {
                if let Ok(duration) = (time::OffsetDateTime::now_utc() - started_at).try_into() {
                    let duration: std::time::Duration = duration;
                    return duration.as_millis() as u64;
                }
            }
        }
        self.dur_ms
    }

    pub fn meter_capacity_for_terminal_width(width: u16) -> usize {
        (width.saturating_sub(11).max(16) as usize).min(MAX_METER_HISTORY)
    }

    fn trim_meters_to_capacity(&mut self) {
        if self.meters.len() > MAX_METER_HISTORY {
            self.meters.drain(..self.meters.len() - MAX_METER_HISTORY);
        }
    }
}

impl Page for StatusPage {
    fn apply_event(&mut self, event: &Event, active: bool) {
        match event {
            Event::Snapshot {
                state,
                recording,
                started_at,
                app,
                app_name,
                dur_ms,
                words,
                segments,
                partial,
                ..
            } => {
                self.state = *state;
                self.recording_id = recording.clone();
                self.started_at = parse_time(started_at.as_deref());
                self.app = app.clone();
                self.app_name = app_name.clone();
                self.dur_ms = *dur_ms;
                self.words = *words;
                self.segments = segments.clone();
                self.partial = partial.clone();
            }
            Event::StateChanged {
                state,
                recording_id,
                started_at,
            } => {
                self.state = *state;
                self.recording_id = recording_id.clone();
                self.started_at = parse_time(started_at.as_deref());
                if *state == WireState::Idle {
                    self.segments.clear();
                    self.partial.clear();
                    self.pipeline.clear();
                    self.session_meta = None;
                    self.session_phase = None;
                    self.meters.clear();
                    self.app = None;
                    self.app_name = None;
                    self.dur_ms = 0;
                    self.words = 0;
                }
            }
            Event::AppChanged { app, app_name } => {
                self.app = app.clone();
                self.app_name = app_name.clone();
            }
            Event::StatsChanged { dur_ms, words } => {
                self.dur_ms = *dur_ms;
                self.words = *words;
            }
            Event::Partial { text, .. } => self.partial = text.clone(),
            Event::Segment { text, .. } => {
                self.segments.push(text.clone());
                self.partial.clear();
            }
            Event::PipelineStep {
                name,
                status,
                duration_ms,
                text,
                error,
                ..
            } => {
                let detail = text.clone().or_else(|| error.clone()).unwrap_or_default();
                self.pipeline
                    .push(format!("{name} {status} {duration_ms:.1}ms  {detail}"));
            }
            Event::AudioMeter { meter, .. } => {
                if active {
                    self.meters.push(*meter);
                    self.trim_meters_to_capacity();
                }
            }
            Event::SessionMeta { meta, .. } => {
                self.session_meta = Some(meta.clone());
            }
            Event::SessionPhase { phase, .. } => {
                self.session_phase = Some(*phase);
            }
            _ => {}
        }
    }

    fn on_key(&mut self, _key: KeyEvent) -> KeyOutcome {
        KeyOutcome::none()
    }

    fn on_enter(&mut self) {
        self.meters.clear();
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &TuiTheme, _footer_status: &str) {
        render_status(frame, self, area, theme);
    }
}

fn parse_time(value: Option<&str>) -> Option<time::OffsetDateTime> {
    value.and_then(|value| {
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
    })
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

fn render_status(frame: &mut Frame, page: &StatusPage, area: Rect, theme: &TuiTheme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
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
                .title(format!("Input  ASR: {provider} -> {chain}"))
                .borders(Borders::ALL),
        ),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(live_speech_lines(
            page,
            theme,
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

fn meter_lines(page: &StatusPage, theme: &TuiTheme, width: usize) -> Vec<Line<'static>> {
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

fn live_speech_lines(
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

fn audio_upper(meters: &[AudioMeter]) -> String {
    meters.iter().map(|meter| upper_level(meter.peak)).collect()
}

fn audio_lower(meters: &[AudioMeter]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn meter(peak: f32) -> AudioMeter {
        AudioMeter {
            rms: peak,
            peak,
            clipped: false,
            vad_probability: None,
            vad_speech: None,
        }
    }

    #[test]
    fn trim_meters_to_capacity_keeps_large_tail() {
        let mut page = StatusPage::new();
        page.meters = (0..1100).map(|idx| meter(idx as f32)).collect::<Vec<_>>();

        page.trim_meters_to_capacity();

        assert_eq!(page.meters.len(), MAX_METER_HISTORY);
        assert_eq!(page.meters.first().unwrap().peak, 76.0);
        assert_eq!(page.meters.last().unwrap().peak, 1099.0);
    }

    #[test]
    fn meter_capacity_tracks_terminal_width_with_minimum_and_4k_cap() {
        assert_eq!(StatusPage::meter_capacity_for_terminal_width(200), 189);
        assert_eq!(StatusPage::meter_capacity_for_terminal_width(20), 16);
        assert_eq!(
            StatusPage::meter_capacity_for_terminal_width(3840),
            MAX_METER_HISTORY
        );
    }

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
            AudioMeter {
                rms: 0.0,
                peak: 0.0,
                clipped: false,
                vad_probability: Some(0.0),
                vad_speech: Some(false),
            },
            AudioMeter {
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
    fn status_header_includes_colored_asr_metadata() {
        crate::i18n::init("en-US");
        let page = StatusPage::new();
        let theme = TuiTheme::default();

        let lines = status_header_lines(
            &page,
            &theme,
            "Ghostty",
            "com.mitchellh.ghostty",
            "apple",
            "-",
            0,
        );
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
        let mut page = StatusPage::new();
        page.segments = vec!["abcdefghijklmnopqrstuvwxyz".to_string()];
        page.partial = "0123456789".to_string();
        let theme = TuiTheme::default();

        let line = live_speech_lines(&page, &theme, 10, 1);
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
        let mut page = StatusPage::new();
        page.segments = vec!["这是很长很长的一段已经定型的语音识别文本".to_string()];
        page.partial = "最新的部分".to_string();
        let theme = TuiTheme::default();

        let line = live_speech_lines(&page, &theme, 16, 1);
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
