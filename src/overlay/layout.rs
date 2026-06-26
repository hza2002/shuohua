//! 平台无关的 overlay 布局。
//! 所有几何输出使用 `LayoutFrame`，平台 view 自行转成平台原生矩形类型。

use crate::config::OverlayPosition;

/// 平台中立 frame：原点（左下，跟 macOS 一致）+ size。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutFrame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl LayoutFrame {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }
}

pub mod constants {
    pub const WIDTH: f64 = 572.0;
    pub const BASE_HEIGHT: f64 = 64.0;
    pub const WINDOW_MARGIN: f64 = 16.0;
    pub const H_PAD: f64 = 10.0;
    pub const BOTTOM_PAD: f64 = 8.0;
    pub const HEADER_BODY_GAP: f64 = 2.0;
    pub const BODY_LINE_H: f64 = 21.0;
    pub const BODY_W: f64 = WIDTH - H_PAD * 2.0;
    pub const CHARS_PER_LINE: usize = 38;
    pub const HEADER_CENTER_Y: f64 = BOTTOM_PAD + BODY_LINE_H + HEADER_BODY_GAP + 12.0;
    pub const ICON_BOX: f64 = 24.0;
    pub const STATE_BOX_H: f64 = 20.0;
    pub const META_BOX_H: f64 = 18.0;
    pub const ICON_OPTICAL_Y: f64 = -0.5;
    pub const STATE_OPTICAL_Y: f64 = 0.0;
    pub const META_OPTICAL_Y: f64 = 0.0;
    pub const ICON_STATE_GAP: f64 = 4.0;
    pub const STATE_W: f64 = 68.0;
    pub const STATE_STATS_GAP: f64 = 5.0;
    pub const STATS_W: f64 = 220.0;
    pub const META_GAP: f64 = 8.0;
    pub const META_MIN_W: f64 = 180.0;
    pub const MIN_TEXT_SCALE: f64 = 0.8;
    pub const MAX_TEXT_SCALE: f64 = 2.4;
}

pub fn text_scale(value: f64) -> f64 {
    value.clamp(constants::MIN_TEXT_SCALE, constants::MAX_TEXT_SCALE)
}

pub fn body_width(width: f64) -> f64 {
    (width - constants::H_PAD * 2.0).max(120.0)
}

pub fn body_line_height(text_scale: f64) -> f64 {
    constants::BODY_LINE_H * self::text_scale(text_scale)
}

pub fn header_row_height(text_scale: f64) -> f64 {
    constants::ICON_BOX
        .max(constants::STATE_BOX_H * self::text_scale(text_scale))
        .max(constants::META_BOX_H * self::text_scale(text_scale))
}

pub fn base_height(text_scale: f64) -> f64 {
    let top_pad = constants::BASE_HEIGHT
        - constants::BOTTOM_PAD
        - constants::BODY_LINE_H
        - constants::HEADER_BODY_GAP
        - constants::ICON_BOX;
    top_pad
        + header_row_height(text_scale)
        + constants::HEADER_BODY_GAP
        + body_line_height(text_scale)
        + constants::BOTTOM_PAD
}

pub fn chars_per_line(width: f64, text_scale: f64) -> usize {
    let default_body = constants::BODY_W;
    let scale = self::text_scale(text_scale);
    let estimate = constants::CHARS_PER_LINE as f64 * body_width(width) / default_body / scale;
    estimate.floor().clamp(12.0, 96.0) as usize
}

pub fn frame_y_for_visual_center(center_y: f64, height: f64, optical_y: f64) -> f64 {
    center_y - height / 2.0 - optical_y
}

#[derive(Debug, Clone, Copy)]
pub struct FirstRow {
    pub icon: LayoutFrame,
    pub status: LayoutFrame,
    pub stats: LayoutFrame,
    pub meta: LayoutFrame,
}

pub fn first_row_frames(width: f64, text_scale: f64, top_offset: f64) -> FirstRow {
    use constants::*;
    let row_h = header_row_height(text_scale);
    let center_y =
        BOTTOM_PAD + body_line_height(text_scale) + HEADER_BODY_GAP + row_h / 2.0 + top_offset;
    let mut x = H_PAD;
    let icon = LayoutFrame::new(
        x,
        frame_y_for_visual_center(center_y, ICON_BOX, ICON_OPTICAL_Y),
        ICON_BOX,
        ICON_BOX,
    );
    x += ICON_BOX + ICON_STATE_GAP;
    let state_h = STATE_BOX_H * self::text_scale(text_scale);
    let status = LayoutFrame::new(
        x,
        frame_y_for_visual_center(center_y, state_h, STATE_OPTICAL_Y),
        STATE_W,
        state_h,
    );
    x += STATE_W + STATE_STATS_GAP;
    let meta_h = META_BOX_H * self::text_scale(text_scale);
    let stats = LayoutFrame::new(
        x,
        frame_y_for_visual_center(center_y, meta_h, META_OPTICAL_Y),
        STATS_W,
        meta_h,
    );
    x += STATS_W + META_GAP;
    let right = width - H_PAD;
    let meta_w = (right - x).max(META_MIN_W);
    FirstRow {
        icon,
        status,
        stats,
        meta: LayoutFrame::new(
            x,
            frame_y_for_visual_center(center_y, meta_h, META_OPTICAL_Y),
            meta_w,
            meta_h,
        ),
    }
}

pub fn scaled_font_size(base: f64, text_scale: f64) -> f64 {
    base * self::text_scale(text_scale)
}

#[derive(Debug, Clone, Copy)]
pub struct OverlayFrames {
    pub height: f64,
    pub row: FirstRow,
    pub body: LayoutFrame,
}

pub fn overlay_frames(width: f64, text_scale: f64, lines: usize) -> OverlayFrames {
    let lines = lines.clamp(1, 8);
    let line_h = body_line_height(text_scale);
    let height = base_height(text_scale) + (lines.saturating_sub(1) as f64 * line_h);
    let top_offset = height - base_height(text_scale);
    OverlayFrames {
        height,
        row: first_row_frames(width, text_scale, top_offset),
        body: LayoutFrame::new(
            constants::H_PAD,
            constants::BOTTOM_PAD,
            body_width(width),
            line_h * lines as f64,
        ),
    }
}

pub fn display_text_plan(text: &str, max_lines: usize, chars_per_line: usize) -> (String, usize) {
    let max_lines = max_lines.clamp(1, 8);
    let chars_per_line = chars_per_line.max(8);
    let chars = text.chars().count().max(1);
    let lines = chars.div_ceil(chars_per_line).clamp(1, max_lines);
    let capacity = chars_per_line * max_lines;
    if chars <= capacity {
        return (text.to_string(), lines);
    }

    let keep = capacity.saturating_sub(1);
    let tail: String = text
        .chars()
        .rev()
        .take(keep)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    (format!("…{tail}"), max_lines)
}

#[derive(Debug, PartialEq, Eq)]
pub struct LiveTextPlan {
    pub segments: String,
    pub partial: String,
    pub lines: usize,
}

impl LiveTextPlan {
    pub fn full_text(&self) -> String {
        let mut text = self.segments.clone();
        text.push_str(&self.partial);
        text
    }
}

pub fn live_text_plan(
    segments: &[String],
    partial: &str,
    max_lines: usize,
    chars_per_line: usize,
) -> LiveTextPlan {
    let full_segments = segments.join("");
    let mut full = full_segments.clone();
    full.push_str(partial);
    let (display, lines) = display_text_plan(&full, max_lines, chars_per_line);
    if display == full {
        return LiveTextPlan {
            segments: full_segments,
            partial: partial.to_string(),
            lines,
        };
    }

    let visible_chars = display
        .strip_prefix('…')
        .unwrap_or(&display)
        .chars()
        .count();
    let partial_chars = partial.chars().count();
    let visible_partial_chars = partial_chars.min(visible_chars);
    let visible_segment_chars = visible_chars.saturating_sub(visible_partial_chars);
    let segment_tail = tail_chars(&full_segments, visible_segment_chars);
    let partial_tail = tail_chars(partial, visible_partial_chars);
    LiveTextPlan {
        segments: if segment_tail.is_empty() {
            "…".to_string()
        } else {
            format!("…{segment_tail}")
        },
        partial: partial_tail,
        lines,
    }
}

pub fn tail_chars(text: &str, count: usize) -> String {
    if count == 0 {
        return String::new();
    }
    text.chars()
        .rev()
        .take(count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

#[derive(Debug, PartialEq, Eq)]
pub struct HeaderParts {
    pub state: String,
    pub duration: String,
    pub words: String,
    pub app: String,
    pub meta: String,
}

pub fn header_parts(
    state: &str,
    duration: &str,
    words: &str,
    app: &str,
    chain: &str,
) -> HeaderParts {
    HeaderParts {
        state: state.to_string(),
        duration: duration.to_string(),
        words: words.to_string(),
        app: app.to_string(),
        meta: chain.to_string(),
    }
}

pub fn stats_text(duration: &str, words: &str, app: &str) -> String {
    if app.is_empty() {
        format!("{duration} · {words}")
    } else {
        format!("{duration} · {words} · {app}")
    }
}

pub fn panel_frame(
    anchor: LayoutFrame,
    position: OverlayPosition,
    width: f64,
    height: f64,
    screen: LayoutFrame,
) -> LayoutFrame {
    use constants::WINDOW_MARGIN;
    let x = anchor.x + (anchor.w - width) / 2.0;
    let y = match position {
        OverlayPosition::Top => anchor.y + anchor.h - height - WINDOW_MARGIN,
        OverlayPosition::Middle => anchor.y + (anchor.h - height) / 2.0,
        OverlayPosition::Bottom => anchor.y + WINDOW_MARGIN,
    };
    let x = clamp(
        x,
        screen.x + WINDOW_MARGIN,
        screen.x + screen.w - width - WINDOW_MARGIN,
    );
    let y = clamp(
        y,
        screen.y + WINDOW_MARGIN,
        screen.y + screen.h - height - WINDOW_MARGIN,
    );
    LayoutFrame::new(x, y, width, height)
}

pub fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if min > max {
        return min;
    }
    value.max(min).min(max)
}

pub fn frame_nearly_eq(a: LayoutFrame, b: LayoutFrame) -> bool {
    (a.x - b.x).abs() < 0.5
        && (a.y - b.y).abs() < 0.5
        && (a.w - b.w).abs() < 0.5
        && (a.h - b.h).abs() < 0.5
}

pub fn format_duration(ms: u64) -> String {
    let total_secs = ms / 1000;
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    if h > 0 {
        format!("{h}h{m}m{s}s")
    } else if m > 0 {
        format!("{m}m{s}s")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visual_center(frame: LayoutFrame, optical_y: f64) -> f64 {
        frame.y + frame.h / 2.0 + optical_y
    }

    #[test]
    fn text_line_count_is_bounded() {
        assert_eq!(display_text_plan("", 5, 34).1, 1);
        assert_eq!(display_text_plan("短句", 5, 34).1, 1);
        assert_eq!(display_text_plan(&"字".repeat(70), 5, 34).1, 3);
        assert_eq!(display_text_plan(&"字".repeat(300), 5, 34).1, 5);
    }

    #[test]
    fn long_text_keeps_tail() {
        let text = format!("{}{}", "前".repeat(200), "后".repeat(20));
        let (visible, lines) = display_text_plan(&text, 5, 20);
        assert_eq!(lines, 5);
        assert!(visible.starts_with('…'));
        assert!(visible.ends_with(&"后".repeat(20)));
        assert!(!visible.contains(&"前".repeat(120)));
    }

    #[test]
    fn header_parts_keep_state_duration_and_meta_separate() {
        let parts = header_parts("Recording", "3s", "84 words", "Xcode", "filler");
        assert_eq!(parts.state, "Recording");
        assert_eq!(parts.duration, "3s");
        assert_eq!(parts.words, "84 words");
        assert_eq!(parts.app, "Xcode");
        assert_eq!(parts.meta, "filler");
    }

    #[test]
    fn first_row_clusters_stats_and_app_on_left_with_wide_meta() {
        let row = first_row_frames(constants::WIDTH, 1.0, 0.0);
        assert!(row.stats.x - (row.status.x + row.status.w) <= 6.0);
        assert!(row.stats.w >= 210.0);
        assert!(row.stats.x < row.meta.x);
        assert!(row.meta.w >= 180.0);
    }

    #[test]
    fn base_overlay_spacing_is_compact() {
        const {
            assert!(constants::BASE_HEIGHT <= 68.0);
            assert!(constants::H_PAD <= 16.0);
            assert!(constants::BOTTOM_PAD <= 8.0);
        }
    }

    #[test]
    fn first_row_uses_shared_visual_center() {
        let row = first_row_frames(constants::WIDTH, 1.0, 0.0);
        let center = constants::HEADER_CENTER_Y;
        assert!((visual_center(row.icon, constants::ICON_OPTICAL_Y) - center).abs() < 0.1);
        assert!((visual_center(row.status, constants::STATE_OPTICAL_Y) - center).abs() < 0.1);
        assert!((visual_center(row.stats, constants::META_OPTICAL_Y) - center).abs() < 0.1);
        assert!((visual_center(row.meta, constants::META_OPTICAL_Y) - center).abs() < 0.1);
    }

    #[test]
    fn header_body_gap_keeps_rows_breathing() {
        assert_eq!(constants::HEADER_BODY_GAP, 2.0);
    }

    #[test]
    fn text_capacity_follows_width_and_scale() {
        assert_eq!(
            chars_per_line(constants::WIDTH, 1.0),
            constants::CHARS_PER_LINE
        );
        assert!(chars_per_line(constants::WIDTH * 1.2, 1.0) > constants::CHARS_PER_LINE);
        assert!(chars_per_line(constants::WIDTH, 1.2) < constants::CHARS_PER_LINE);
        assert!(body_line_height(1.2) > constants::BODY_LINE_H);
        assert!(base_height(1.5) > constants::BASE_HEIGHT);
        assert!(overlay_frames(constants::WIDTH, 1.5, 1).body.h > constants::BODY_LINE_H);
    }

    #[test]
    fn scaled_overlay_frames_keep_header_above_body() {
        let normal = overlay_frames(constants::WIDTH, 1.0, 1);
        assert_eq!(normal.height, constants::BASE_HEIGHT);

        let large = overlay_frames(constants::WIDTH, 1.5, 1);
        assert!(large.height > normal.height);
        assert!(large.row.status.h > constants::STATE_BOX_H);
        assert!(large.row.stats.h > constants::META_BOX_H);
        assert!(large.body.h > constants::BODY_LINE_H);
        assert!(large.row.status.y >= large.body.y + large.body.h + constants::HEADER_BODY_GAP);
        assert!(large.row.meta.y >= large.body.y + large.body.h + constants::HEADER_BODY_GAP);
    }

    #[test]
    fn stats_text_is_inline_metadata() {
        assert_eq!(stats_text("12s", "128字", "Xcode"), "12s · 128字 · Xcode");
        assert_eq!(stats_text("12s", "128字", ""), "12s · 128字");
    }

    #[test]
    fn header_stats_use_supplied_word_count_text() {
        let header = header_parts("Recording", "4s", "9 words", "Xcode", "chain");

        assert_eq!(
            stats_text(&header.duration, &header.words, &header.app),
            "4s · 9 words · Xcode"
        );
    }

    #[test]
    fn live_text_plan_keeps_segments_and_partial_distinct() {
        let plan = live_text_plan(&["已经定型。".to_string()], "正在识别", 5, 34);
        assert_eq!(plan.segments, "已经定型。");
        assert_eq!(plan.partial, "正在识别");
        assert_eq!(plan.lines, 1);
    }

    #[test]
    fn positions_overlay_inside_anchor_centered() {
        let anchor = LayoutFrame::new(100.0, 100.0, 800.0, 600.0);
        let screen = LayoutFrame::new(0.0, 0.0, 1200.0, 900.0);
        let bottom = panel_frame(anchor, OverlayPosition::Bottom, 540.0, 86.0, screen);
        assert_eq!(bottom.x, 230.0);
        assert_eq!(bottom.y, 116.0);
        let middle = panel_frame(anchor, OverlayPosition::Middle, 540.0, 86.0, screen);
        assert_eq!(middle.x, 230.0);
        assert_eq!(middle.y, 357.0);
        let top = panel_frame(anchor, OverlayPosition::Top, 540.0, 86.0, screen);
        assert_eq!(top.x, 230.0);
        assert_eq!(top.y, 598.0);
    }
}
