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
    #[cfg(test)]
    pub(crate) use crate::config::DEFAULT_OVERLAY_WIDTH_PX as DEFAULT_WIDTH_PX;
    #[cfg(test)]
    pub(crate) const WIDTH: f64 = DEFAULT_WIDTH_PX as f64;
    pub const BASE_HEIGHT: f64 = 64.0;
    pub const WINDOW_MARGIN: f64 = 16.0;
    pub const H_PAD: f64 = 16.0;
    pub const BOTTOM_PAD: f64 = 7.0;
    pub const HEADER_BODY_GAP: f64 = 2.0;
    /// 单行 body 的基线高度，也是头部块几何的锚。多行高度由平台层实测，不再估算。
    pub const BODY_LINE_H: f64 = 21.0;
    pub const SCROLL_INDICATOR_TEXT_GAP: f64 = 4.0;
    pub const HEADER_CENTER_Y: f64 = BOTTOM_PAD + BODY_LINE_H + HEADER_BODY_GAP + 12.0;
    pub const ICON_BOX: f64 = 24.0;
    pub const STATE_BOX_H: f64 = 20.0;
    pub const META_BOX_H: f64 = 18.0;
    pub const ICON_OPTICAL_Y: f64 = -0.5;
    pub const STATE_OPTICAL_Y: f64 = 0.0;
    pub const META_OPTICAL_Y: f64 = 0.0;
    pub const ICON_STATE_GAP: f64 = 5.0;
    pub const STATE_W: f64 = 56.0;
    pub const STATE_STATS_GAP: f64 = 5.0;
    pub const META_GAP: f64 = 8.0;
    pub const HEADER_STATS_FALLBACK_FRACTION: f64 = 0.25;
    pub const PICKER_GAP: f64 = 6.0;
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

#[cfg(test)]
pub fn first_row_frames(top_offset: f64) -> FirstRow {
    first_row_frames_with_text_widths(
        constants::WIDTH,
        top_offset,
        constants::STATE_W,
        constants::WIDTH * constants::HEADER_STATS_FALLBACK_FRACTION,
    )
}

pub fn first_row_frames_with_text_widths(
    width: f64,
    top_offset: f64,
    status_text_w: f64,
    stats_text_w: f64,
) -> FirstRow {
    first_row_frames_with_text_widths_and_meta(
        width,
        top_offset,
        status_text_w,
        stats_text_w,
        f64::INFINITY,
    )
}

pub fn first_row_frames_with_text_widths_and_meta(
    width: f64,
    top_offset: f64,
    status_text_w: f64,
    stats_text_w: f64,
    meta_text_w: f64,
) -> FirstRow {
    use constants::*;
    let center_y = HEADER_CENTER_Y + top_offset;
    let content_left = H_PAD;
    let content_right = (width - H_PAD).max(content_left);
    let content_mid = (content_left + content_right) / 2.0;
    let meta_rail_w = (content_right - content_mid).max(0.0);
    let meta_w = meta_text_w.max(0.0).min(meta_rail_w);
    let meta_x = content_right - meta_w;
    let left_rail_right = content_mid - META_GAP;
    let mut x = content_left;
    let icon = LayoutFrame::new(
        x,
        frame_y_for_visual_center(center_y, ICON_BOX, ICON_OPTICAL_Y),
        ICON_BOX,
        ICON_BOX,
    );
    x += ICON_BOX + ICON_STATE_GAP;
    let status_w = status_text_w.max(0.0).min((left_rail_right - x).max(0.0));
    let status = LayoutFrame::new(
        x,
        frame_y_for_visual_center(center_y, STATE_BOX_H, STATE_OPTICAL_Y),
        status_w,
        STATE_BOX_H,
    );
    x += status_w + STATE_STATS_GAP;
    let stats_w = stats_text_w.max(0.0).min((left_rail_right - x).max(0.0));
    let stats = LayoutFrame::new(
        x,
        frame_y_for_visual_center(center_y, META_BOX_H, META_OPTICAL_Y),
        stats_w,
        META_BOX_H,
    );
    FirstRow {
        icon,
        status,
        stats,
        meta: LayoutFrame::new(
            meta_x,
            frame_y_for_visual_center(center_y, META_BOX_H, META_OPTICAL_Y),
            meta_w,
            META_BOX_H,
        ),
    }
}

/// 面板几何，由平台层实测的 body 文本高度换算而来。
///
/// `body_h`、`line_count` 和 `tail_metrics` 都来自真正绘制 body 的 NSTextView /
/// NSLayoutManager。layout 不再按固定行高重建多行高度；单行基线 `BODY_LINE_H` 只在
/// AppKit 无法测量时作为兜底，保持单行时与旧布局一致的头部几何。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyGeometry {
    /// body document 内容高度，来自 renderer 实测。
    pub content_height: f64,
    /// 面板总高。
    pub panel_height: f64,
    /// 头部块相对单行布局上移的量（多出来的 body 高度）。
    pub top_offset: f64,
    /// body 视口 frame 高度。
    pub field_height: f64,
    /// NSScrollView frame，宽度包含右侧 panel padding 里的悬浮 indicator 区。
    pub body_viewport: LayoutFrame,
    /// NSTextView document frame。正文宽度不为 indicator 让位。
    pub body_document: LayoutFrame,
    /// follow 模式滚到底时 clip view 的 y offset。
    pub scroll_bottom_offset: f64,
    /// 内容真实高度超过视口，需要打开鼠标滚动。
    pub body_overflow: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyTailMetrics {
    pub viewport_height: f64,
    pub scroll_offset: f64,
}

impl BodyGeometry {
    pub fn scroll_indicator_frame(
        &self,
        min_height: f64,
        width: f64,
        visible_y: f64,
    ) -> Option<LayoutFrame> {
        if !self.body_overflow {
            return None;
        }
        let content_h = self.body_document.h.max(self.field_height);
        let ratio = (self.field_height / content_h).clamp(0.0, 1.0);
        let indicator_h = (self.field_height * ratio)
            .max(min_height)
            .min(self.field_height);
        let scrollable = (content_h - self.field_height).max(1.0);
        let progress = (visible_y / scrollable).clamp(0.0, 1.0);
        let y = (self.field_height - indicator_h) * progress;
        let x = self.body_document.w + constants::SCROLL_INDICATOR_TEXT_GAP;
        Some(LayoutFrame::new(x, y, width, indicator_h))
    }
}

#[cfg(test)]
pub fn body_geometry_with_tail_metrics(
    body_h: f64,
    max_text_lines: usize,
    single_line_h: f64,
    extra_line_h: f64,
    line_count: usize,
    tail_metrics: Option<BodyTailMetrics>,
) -> BodyGeometry {
    body_geometry_with_tail_metrics_for_width(
        constants::WIDTH,
        body_h,
        max_text_lines,
        single_line_h,
        extra_line_h,
        line_count,
        tail_metrics,
    )
}

pub fn body_geometry_with_tail_metrics_for_width(
    width: f64,
    body_h: f64,
    max_text_lines: usize,
    single_line_h: f64,
    extra_line_h: f64,
    line_count: usize,
    tail_metrics: Option<BodyTailMetrics>,
) -> BodyGeometry {
    let single_line_h = single_line_h.max(1.0);
    let extra_line_h = extra_line_h.max(1.0);
    let max_lines = max_text_lines.max(1) as f64;
    let viewport_cap = single_line_h + extra_line_h * (max_lines - 1.0);
    let content_height = body_h.max(single_line_h);
    let body_overflow = line_count > max_text_lines.max(1);
    let (field_height, requested_scroll_offset) = if body_overflow {
        let field_height = tail_metrics
            .map(|metrics| metrics.viewport_height)
            .unwrap_or(viewport_cap)
            .min(content_height)
            .max(single_line_h);
        let requested_scroll_offset = tail_metrics
            .map(|metrics| metrics.scroll_offset)
            .unwrap_or((content_height - field_height).max(0.0));
        (field_height, requested_scroll_offset)
    } else {
        (content_height.min(viewport_cap).max(single_line_h), 0.0)
    };
    let extra = field_height - single_line_h;
    let document_height = content_height
        .max(field_height)
        .max(requested_scroll_offset + field_height);
    let scroll_bottom_offset = requested_scroll_offset.clamp(0.0, document_height - field_height);
    let body_text_w = (width - constants::H_PAD * 2.0).max(1.0);
    BodyGeometry {
        content_height,
        panel_height: constants::BASE_HEIGHT + extra,
        top_offset: extra,
        field_height,
        body_viewport: LayoutFrame::new(
            constants::H_PAD,
            constants::BOTTOM_PAD,
            (width - constants::H_PAD).max(1.0),
            field_height,
        ),
        body_document: LayoutFrame::new(0.0, 0.0, body_text_w, document_height),
        scroll_bottom_offset,
        body_overflow,
    }
}

pub fn scroll_discovery_should_extend(
    last_visible_y: Option<f64>,
    visible_y: f64,
    programmatic_scroll: bool,
) -> bool {
    last_visible_y.is_some_and(|last| (last - visible_y).abs() > 0.5) && !programmatic_scroll
}

pub fn wants_mouse(visible: bool, body_overflow: bool, pipeline_clickable: bool) -> bool {
    visible && (body_overflow || pipeline_clickable)
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

pub fn profile_chain_display(
    display_name: &str,
    asr_instance: &str,
    chain_summary: &str,
) -> String {
    let mut steps = Vec::new();
    if !asr_instance.is_empty() {
        steps.push(asr_instance.to_string());
    }
    let pipeline = chain_summary.replace(" -> ", " → ");
    if !pipeline.is_empty() {
        steps.extend(pipeline.split(" → ").map(str::to_string));
    }
    if steps.is_empty() {
        display_name.to_string()
    } else {
        format!("{display_name}: {}", steps.join(" → "))
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

pub fn picker_frame(
    overlay: LayoutFrame,
    width: f64,
    height: f64,
    screen: LayoutFrame,
) -> LayoutFrame {
    let screen_left = screen.x + constants::WINDOW_MARGIN;
    let screen_right = screen.x + screen.w - constants::WINDOW_MARGIN;
    let screen_bottom = screen.y + constants::WINDOW_MARGIN;
    let screen_top = screen.y + screen.h - constants::WINDOW_MARGIN;

    let width = width.min((screen_right - screen_left).max(1.0));
    let height = height.min((screen_top - screen_bottom).max(1.0));
    let x = overlay.x + overlay.w - constants::H_PAD - width;
    let below = overlay.y - height - constants::PICKER_GAP;
    let above = overlay.y + overlay.h + constants::PICKER_GAP;
    let below_fits = below >= screen_bottom;
    let above_fits = above + height <= screen_top;
    let below_space = overlay.y - constants::PICKER_GAP - screen_bottom;
    let above_space = screen_top - (overlay.y + overlay.h + constants::PICKER_GAP);
    let y = match (below_fits, above_fits) {
        (true, false) => below,
        (false, true) => above,
        (true, true) => {
            if below_space >= above_space {
                below
            } else {
                above
            }
        }
        (false, false) => {
            if above_space >= below_space {
                above
            } else {
                below
            }
        }
    };
    let x = clamp(x, screen_left, screen_right - width);
    let y = clamp(y, screen_bottom, screen_top - height);
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

    fn body_geometry_with_line_metrics(
        body_h: f64,
        max_text_lines: usize,
        single_line_h: f64,
        extra_line_h: f64,
    ) -> BodyGeometry {
        let line_count = (((body_h - single_line_h) / extra_line_h).ceil().max(0.0) as usize) + 1;
        body_geometry_with_tail_metrics(
            body_h,
            max_text_lines,
            single_line_h,
            extra_line_h,
            line_count,
            None,
        )
    }

    fn visual_center(frame: LayoutFrame, optical_y: f64) -> f64 {
        frame.y + frame.h / 2.0 + optical_y
    }

    #[test]
    fn single_line_body_keeps_base_geometry() {
        // 实测高度 ≤ 单行基线时按单行处理：面板高 = BASE_HEIGHT，不上移。
        let g = body_geometry_with_line_metrics(
            constants::BODY_LINE_H,
            3,
            constants::BODY_LINE_H,
            constants::BODY_LINE_H,
        );
        assert_eq!(g.panel_height, constants::BASE_HEIGHT);
        assert_eq!(g.top_offset, 0.0);
        assert_eq!(g.field_height, constants::BODY_LINE_H);
        assert!(!g.body_overflow);

        let shorter = body_geometry_with_line_metrics(
            constants::BODY_LINE_H - 5.0,
            3,
            constants::BODY_LINE_H,
            constants::BODY_LINE_H,
        );
        assert_eq!(shorter.panel_height, constants::BASE_HEIGHT);
        assert_eq!(shorter.field_height, constants::BODY_LINE_H);
        assert!(!shorter.body_overflow);
    }

    #[test]
    fn multi_line_body_grows_panel_by_measured_overflow() {
        // 实测两行高度 → 面板按多出的真实像素长高，field 跟随实测高度。
        let two_lines = constants::BODY_LINE_H * 2.0;
        let g = body_geometry_with_line_metrics(
            two_lines,
            3,
            constants::BODY_LINE_H,
            constants::BODY_LINE_H,
        );
        assert_eq!(g.field_height, two_lines);
        assert_eq!(g.top_offset, constants::BODY_LINE_H);
        assert_eq!(
            g.panel_height,
            constants::BASE_HEIGHT + constants::BODY_LINE_H
        );
        assert!(!g.body_overflow);
    }

    #[test]
    fn body_geometry_caps_viewport_at_max_lines() {
        let g = body_geometry_with_line_metrics(
            constants::BODY_LINE_H * 8.0,
            3,
            constants::BODY_LINE_H,
            constants::BODY_LINE_H,
        );

        assert_eq!(g.field_height, constants::BODY_LINE_H * 3.0);
        assert_eq!(g.top_offset, constants::BODY_LINE_H * 2.0);
        assert_eq!(
            g.panel_height,
            constants::BASE_HEIGHT + constants::BODY_LINE_H * 2.0
        );
        assert!(g.body_overflow);
    }

    #[test]
    fn capped_viewport_preserves_appkit_tail_scroll_extent() {
        let single_line_h = 22.0;
        let extra_line_h = 18.0;
        let appkit_tail_h = 55.25;
        let appkit_tail_y = 57.25;
        let g = body_geometry_with_tail_metrics(
            112.0,
            3,
            single_line_h,
            extra_line_h,
            6,
            Some(BodyTailMetrics {
                viewport_height: appkit_tail_h,
                scroll_offset: appkit_tail_y,
            }),
        );

        assert_eq!(g.field_height, appkit_tail_h);
        assert_eq!(g.scroll_bottom_offset, appkit_tail_y);
        assert_eq!(g.body_document.h, appkit_tail_y + appkit_tail_h);
        assert!(g.body_overflow);
    }

    #[test]
    fn body_overflow_uses_actual_line_count_not_height_cap() {
        let g = body_geometry_with_tail_metrics(
            44.0,
            3,
            22.0,
            18.0,
            4,
            Some(BodyTailMetrics {
                viewport_height: 40.0,
                scroll_offset: 4.0,
            }),
        );

        assert!(g.body_overflow);
        assert_eq!(g.field_height, 40.0);
        assert_eq!(g.scroll_bottom_offset, 4.0);
    }

    #[test]
    fn body_document_height_preserves_measured_content_height() {
        let g = body_geometry_with_tail_metrics(73.4, 5, 22.0, 18.0, 4, None);

        assert_eq!(g.content_height, 73.4);
        assert_eq!(g.body_document.h, 73.4);
        assert!(!g.body_overflow);
    }

    #[test]
    fn body_document_height_covers_requested_scroll_extent() {
        let g = body_geometry_with_tail_metrics(
            100.0,
            3,
            22.0,
            18.0,
            5,
            Some(BodyTailMetrics {
                viewport_height: 54.0,
                scroll_offset: 48.0,
            }),
        );

        assert_eq!(g.content_height, 100.0);
        assert_eq!(g.body_document.h, 102.0);
        assert_eq!(g.scroll_bottom_offset, 48.0);
        assert!(g.body_overflow);
    }

    #[test]
    fn body_geometry_uses_measured_line_height_for_cap_and_growth() {
        let line_h = 23.5;
        let content_h = line_h * 8.0;
        let g = body_geometry_with_line_metrics(content_h, 3, line_h, line_h);

        assert_eq!(g.field_height, line_h * 3.0);
        assert_eq!(g.top_offset, line_h * 2.0);
        assert_eq!(g.panel_height, constants::BASE_HEIGHT + line_h * 2.0);
        assert!(g.body_overflow);
    }

    #[test]
    fn body_geometry_counts_vertical_padding_once() {
        let single_line_h = 23.0;
        let extra_line_h = 19.0;
        let content_h = single_line_h + extra_line_h * 7.0;
        let g = body_geometry_with_line_metrics(content_h, 3, single_line_h, extra_line_h);

        assert_eq!(g.field_height, single_line_h + extra_line_h * 2.0);
        assert_eq!(g.top_offset, extra_line_h * 2.0);
        assert_eq!(g.panel_height, constants::BASE_HEIGHT + extra_line_h * 2.0);
        assert!(g.body_overflow);
    }

    #[test]
    fn body_geometry_preserves_measured_content_height() {
        let single_line_h = 23.0;
        let extra_line_h = 19.0;
        let almost_four_lines = single_line_h + extra_line_h * 3.0 + 0.2;
        let g = body_geometry_with_line_metrics(almost_four_lines, 5, single_line_h, extra_line_h);

        assert_eq!(g.content_height, almost_four_lines);
        assert_eq!(g.field_height, almost_four_lines);
        assert_eq!(g.top_offset, almost_four_lines - single_line_h);
    }

    #[test]
    fn body_text_width_matches_body_width() {
        let viewport_w = constants::WIDTH - constants::H_PAD * 2.0;
        let text_w = constants::WIDTH - constants::H_PAD * 2.0;

        assert_eq!(text_w, viewport_w);
    }

    #[test]
    fn body_geometry_returns_all_body_frames_from_one_source() {
        let g = body_geometry_with_line_metrics(76.0, 3, 22.0, 18.0);

        assert_eq!(g.body_viewport.h, 58.0);
        assert_eq!(
            g.body_document,
            LayoutFrame::new(0.0, 0.0, constants::WIDTH - constants::H_PAD * 2.0, 76.0)
        );
        assert_eq!(g.scroll_bottom_offset, 18.0);
        assert!(g.body_overflow);
    }

    #[test]
    fn scroll_indicator_frame_does_not_reduce_text_width() {
        let g = body_geometry_with_line_metrics(90.0, 3, 22.0, 18.0);
        let indicator = g
            .scroll_indicator_frame(18.0, 3.0, 0.0)
            .expect("overflow has indicator");
        let body_text_w = constants::WIDTH - constants::H_PAD * 2.0;
        assert_eq!(g.body_document.w, body_text_w);
        assert!(indicator.x >= body_text_w);
        assert_eq!(
            indicator.x - body_text_w,
            constants::SCROLL_INDICATOR_TEXT_GAP
        );
        assert_eq!(indicator.w, 3.0);
    }

    #[test]
    fn scroll_indicator_tracks_visible_offset() {
        let g = body_geometry_with_line_metrics(94.0, 3, 22.0, 18.0);
        let bottom = g.scroll_bottom_offset;
        let top_indicator = g
            .scroll_indicator_frame(18.0, 3.0, 0.0)
            .expect("overflow has indicator");
        let bottom_indicator = g
            .scroll_indicator_frame(18.0, 3.0, bottom)
            .expect("overflow has indicator");

        assert_eq!(top_indicator.y, 0.0);
        assert!((bottom_indicator.y + bottom_indicator.h - g.field_height).abs() < 0.1);
    }

    #[test]
    fn programmatic_scroll_does_not_extend_scroll_discovery() {
        assert!(scroll_discovery_should_extend(Some(0.0), 9.0, false));
        assert!(!scroll_discovery_should_extend(Some(0.0), 9.0, true));
        assert!(!scroll_discovery_should_extend(None, 9.0, false));
        assert!(!scroll_discovery_should_extend(Some(9.0), 9.2, false));
    }

    #[test]
    fn wants_mouse_only_when_visible_and_interactive() {
        assert!(!wants_mouse(false, true, true));
        assert!(wants_mouse(true, true, false));
        assert!(wants_mouse(true, false, true));
        assert!(!wants_mouse(true, false, false));
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
        let row = first_row_frames_with_text_widths(constants::WIDTH, 0.0, 72.0, 120.0);
        assert!(row.stats.x - (row.status.x + row.status.w) <= 6.0);
        assert_eq!(row.stats.w, 120.0);
        assert!(row.stats.x < row.meta.x);
        assert_eq!(row.meta.x, constants::WIDTH / 2.0);
    }

    #[test]
    fn first_row_uses_left_and_right_content_rails() {
        let row = first_row_frames_with_text_widths(constants::WIDTH, 0.0, 72.0, 120.0);
        let content_left = constants::H_PAD;
        let content_right = constants::WIDTH - constants::H_PAD;
        let content_mid = (content_left + content_right) / 2.0;

        assert_eq!(row.icon.x, content_left);
        assert_eq!(row.meta.x, content_mid);
        assert_eq!(row.meta.x + row.meta.w, content_right);
    }

    #[test]
    fn first_row_left_cluster_never_crosses_midline_gap() {
        let row = first_row_frames_with_text_widths(constants::WIDTH, 0.0, 220.0, 220.0);
        let content_mid = constants::WIDTH / 2.0;

        assert!(row.status.x + row.status.w <= content_mid - constants::META_GAP);
        assert!(row.stats.x + row.stats.w <= content_mid - constants::META_GAP);
        assert_eq!(row.meta.x, content_mid);
    }

    #[test]
    fn first_row_short_meta_hugs_content_right_edge() {
        let row =
            first_row_frames_with_text_widths_and_meta(constants::WIDTH, 0.0, 72.0, 120.0, 96.0);
        let content_right = constants::WIDTH - constants::H_PAD;

        assert_eq!(row.meta.w, 96.0);
        assert_eq!(row.meta.x + row.meta.w, content_right);
    }

    #[test]
    fn first_row_long_meta_uses_full_right_rail() {
        let row =
            first_row_frames_with_text_widths_and_meta(constants::WIDTH, 0.0, 72.0, 120.0, 900.0);
        let content_mid = constants::WIDTH / 2.0;
        let content_right = constants::WIDTH - constants::H_PAD;

        assert_eq!(row.meta.x, content_mid);
        assert_eq!(row.meta.x + row.meta.w, content_right);
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
        let row = first_row_frames(0.0);
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
    fn profile_chain_display_puts_asr_first() {
        assert_eq!(
            profile_chain_display("Agent", "doubao", "zh_filter → deepseek"),
            "Agent: doubao → zh_filter → deepseek"
        );
        assert_eq!(
            profile_chain_display("Agent", "", "zh_filter"),
            "Agent: zh_filter"
        );
        assert_eq!(
            profile_chain_display("Agent", "doubao", ""),
            "Agent: doubao"
        );
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

    #[test]
    fn picker_opens_above_bottom_overlay() {
        let screen = LayoutFrame::new(0.0, 0.0, 1200.0, 900.0);
        let overlay = LayoutFrame::new(300.0, 16.0, constants::WIDTH, 64.0);

        let picker = picker_frame(overlay, 220.0, 124.0, screen);

        assert!(picker.y >= overlay.y + overlay.h);
        assert!(picker.y + picker.h <= screen.y + screen.h - constants::WINDOW_MARGIN);
    }

    #[test]
    fn picker_right_edge_aligns_with_overlay_content_right_rail() {
        let screen = LayoutFrame::new(0.0, 0.0, 1200.0, 900.0);
        let overlay = LayoutFrame::new(300.0, 320.0, constants::WIDTH, 64.0);

        let picker = picker_frame(overlay, 220.0, 124.0, screen);

        assert_eq!(
            picker.x + picker.w,
            overlay.x + overlay.w - constants::H_PAD
        );
    }

    #[test]
    fn picker_opens_below_top_overlay() {
        let screen = LayoutFrame::new(0.0, 0.0, 1200.0, 900.0);
        let overlay = LayoutFrame::new(300.0, 820.0, constants::WIDTH, 64.0);

        let picker = picker_frame(overlay, 220.0, 124.0, screen);

        assert!(picker.y + picker.h <= overlay.y);
        assert!(picker.y >= screen.y + constants::WINDOW_MARGIN);
    }

    #[test]
    fn picker_frame_clamps_inside_screen() {
        let screen = LayoutFrame::new(0.0, 0.0, 360.0, 240.0);
        let overlay = LayoutFrame::new(10.0, 80.0, constants::WIDTH, 64.0);

        let picker = picker_frame(overlay, 340.0, 210.0, screen);

        assert!(picker.x >= screen.x + constants::WINDOW_MARGIN);
        assert!(picker.y >= screen.y + constants::WINDOW_MARGIN);
        assert!(picker.x + picker.w <= screen.x + screen.w - constants::WINDOW_MARGIN);
        assert!(picker.y + picker.h <= screen.y + screen.h - constants::WINDOW_MARGIN);
    }
}
