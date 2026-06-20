use ratatui::style::Color;

use crate::config::theme::TuiTheme;

pub fn rgb(value: u32) -> Color {
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

pub fn truncate_display(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars && max_chars > 0 {
        out.pop();
        out.push('…');
    }
    out
}

pub fn format_duration(ms: u64) -> String {
    let seconds = ms / 1000;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    if hours > 0 {
        format!("{hours}:{:02}:{:02}", minutes % 60, seconds % 60)
    } else {
        format!("{:02}:{:02}", minutes, seconds % 60)
    }
}
