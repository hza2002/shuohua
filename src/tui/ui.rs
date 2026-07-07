use ratatui::style::Color;
use ratatui::text::{Line, Span};

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

/// Terminal cell width of one char via unicode-width (CJK/wide = 2, most
/// symbols = 1, zero-width/control = 0). Correct for narrow non-ASCII glyphs
/// like `•`/`—`/`…` where a naive "non-ASCII = 2" would overcount and misalign.
pub fn char_width(ch: char) -> usize {
    unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0)
}

/// Terminal display width in cells.
pub fn display_width(value: &str) -> usize {
    value.chars().map(char_width).sum()
}

pub fn truncate_display(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars && max_chars > 0 {
        out.pop();
        out.push('…');
    }
    out
}

/// Visible item range for a scrollable list, keeping the selected item near the
/// middle when possible and clamping at the top/bottom edges.
pub fn visible_range_for_selection(
    selected: usize,
    total: usize,
    visible_len: usize,
) -> std::ops::Range<usize> {
    if total == 0 || visible_len == 0 {
        return 0..0;
    }
    let visible_len = visible_len.min(total);
    let half = visible_len / 2;
    let start = selected.saturating_sub(half).min(total - visible_len);
    start..start + visible_len
}

/// Greedy width-aware wrap: split `text` into lines whose display width does
/// not exceed `width` (CJK chars count as 2). Explicit `\n` is a hard break and
/// blank lines are preserved. Char-based (not word-based) so it wraps CJK — the
/// caller renders the result without ratatui's own `Wrap`, so scroll offsets map
/// 1:1 to these lines.
pub fn wrap_to_width(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return text.split('\n').map(str::to_string).collect();
    }
    let mut lines = Vec::new();
    for raw in text.split('\n') {
        let mut cur = String::new();
        let mut w = 0usize;
        for ch in raw.chars() {
            let cw = char_width(ch);
            if w + cw > width && !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
                w = 0;
            }
            cur.push(ch);
            w += cw;
        }
        lines.push(cur);
    }
    lines
}

/// Char-based, width-aware wrap of already-styled lines into rows no wider than
/// `width` display cells, preserving each char's style. Rendered without
/// ratatui's own `Wrap`, so the resulting line count is exact — scroll offsets
/// map 1:1 to these rows.
pub fn wrap_styled_lines(lines: &[Line<'static>], width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return lines.to_vec();
    }
    let mut out: Vec<Line<'static>> = Vec::new();
    for line in lines {
        let mut row: Vec<Span<'static>> = Vec::new();
        let mut w = 0usize;
        for span in &line.spans {
            let style = span.style;
            for ch in span.content.chars() {
                let cw = char_width(ch);
                if w + cw > width && w > 0 {
                    out.push(Line::from(std::mem::take(&mut row)));
                    w = 0;
                }
                match row.last_mut() {
                    Some(last) if last.style == style => last.content.to_mut().push(ch),
                    _ => row.push(Span::styled(ch.to_string(), style)),
                }
                w += cw;
            }
        }
        out.push(Line::from(row));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_ascii_at_width() {
        assert_eq!(wrap_to_width("hello world", 5), ["hello", " worl", "d"]);
    }

    #[test]
    fn visible_range_keeps_selected_near_middle() {
        assert_eq!(visible_range_for_selection(0, 100, 9), 0..9);
        assert_eq!(visible_range_for_selection(4, 100, 9), 0..9);
        assert_eq!(visible_range_for_selection(20, 100, 9), 16..25);
        assert_eq!(visible_range_for_selection(98, 100, 9), 91..100);
        assert_eq!(visible_range_for_selection(0, 0, 9), 0..0);
        assert_eq!(visible_range_for_selection(0, 10, 0), 0..0);
        assert_eq!(visible_range_for_selection(5, 10, 20), 0..10);
    }

    #[test]
    fn wrap_counts_cjk_as_two_cells() {
        assert_eq!(wrap_to_width("你好世界", 4), ["你好", "世界"]);
        // A width of 3 fits only one 2-cell char per line.
        assert_eq!(wrap_to_width("你好", 3), ["你", "好"]);
    }

    #[test]
    fn wrap_preserves_hard_newlines_and_blanks() {
        assert_eq!(wrap_to_width("a\nb", 5), ["a", "b"]);
        assert_eq!(wrap_to_width("a\n\nb", 5), ["a", "", "b"]);
    }

    #[test]
    fn wrap_width_zero_returns_lines_verbatim() {
        assert_eq!(wrap_to_width("abc\ndef", 0), ["abc", "def"]);
    }

    #[test]
    fn wrap_styled_lines_splits_and_preserves_style() {
        use ratatui::style::Style;
        let line = Line::from(vec![Span::styled("abcd", Style::default().fg(Color::Red))]);
        let out = wrap_styled_lines(&[line], 2);
        assert_eq!(out.len(), 2, "4 cells at width 2 -> 2 rows");
        let text: String = out
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(text, "abcd");
        assert!(out
            .iter()
            .flat_map(|l| l.spans.iter())
            .all(|s| s.style.fg == Some(Color::Red)));
    }

    #[test]
    fn wrap_styled_lines_keeps_each_source_line_separate() {
        use ratatui::style::Style;
        let a = Line::from(Span::styled("hi", Style::default()));
        let b = Line::from(Span::styled("yo", Style::default()));
        // Wide enough that neither wraps: two source lines -> two rows.
        assert_eq!(wrap_styled_lines(&[a, b], 80).len(), 2);
    }
}
