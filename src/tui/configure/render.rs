use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::config::field_view::FieldOrigin;
use crate::config::theme::TuiTheme;
use crate::tui::configure::{ConfigureFocus, ConfigureModule, ConfigurePage, Draft};
use crate::tui::settings::SettingsRow;
use crate::tui::ui;

pub(super) fn render_page(
    frame: &mut Frame,
    page: &ConfigurePage,
    area: Rect,
    theme: &TuiTheme,
    footer_status: &str,
) {
    // Clear hit regions — will be repopulated during this frame.
    page.hit.borrow_mut().clear();

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(0)])
        .split(area);

    // Backfill module hit regions (one row per module, starting at cols[0].y + 1).
    for (idx, _module) in all_modules().iter().enumerate() {
        page.hit.borrow_mut().modules.push((
            Rect::new(cols[0].x, cols[0].y + 1 + idx as u16, cols[0].width, 1),
            idx,
        ));
    }

    frame.render_widget(
        Paragraph::new(module_nav_lines(page, theme)).block(
            block_for_focus(page, ConfigureFocus::Modules, theme)
                .title(crate::t!("tui.configure.modules"))
                .borders(Borders::ALL),
        ),
        cols[0],
    );

    let right = cols[1];

    if page.draft_active() {
        render_draft(frame, page, right, theme);
        if let Some(m) = &page.modal {
            render_modal(frame, m, area, theme);
        }
        if let Some(err) = &page.edit_error {
            render_error_popup(frame, err, area, theme);
        }
        return;
    }

    if matches!(
        page.module,
        ConfigureModule::Overview | ConfigureModule::Main
    ) {
        // Split: overview band on top, main fields below.
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(9), Constraint::Min(0)])
            .split(right);

        frame.render_widget(
            Paragraph::new(overview_lines(page, theme, footer_status))
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title(crate::t!("tui.configure.overview"))
                        .borders(Borders::ALL),
                ),
            rows[0],
        );

        render_fields_and_detail(frame, page, rows[1], theme);
    } else {
        // Split: module title/source list on top, fields below.
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(right);

        render_source_strip(frame, page, rows[0], theme);
        render_fields_and_detail(frame, page, rows[1], theme);
    }

    if let Some(m) = &page.modal {
        render_modal(frame, m, area, theme);
    }

    if let Some(picker) = &page.member_picker {
        render_member_picker(frame, picker, area, theme);
    }

    // Error popup is rendered last so it appears on top.
    if let Some(err) = &page.edit_error {
        render_error_popup(frame, err, area, theme);
    }
}

/// The composer `a` flow: a centered selectable list of post component ids.
fn render_member_picker(
    frame: &mut Frame,
    picker: &crate::tui::configure::MemberPicker,
    area: Rect,
    theme: &TuiTheme,
) {
    let rect = centered_modal(area, 50, 14);
    frame.render_widget(Clear, rect);
    let lines: Vec<Line> = if picker.ids.is_empty() {
        vec![Line::styled(
            crate::t!("tui.configure.composer.picker_empty"),
            Style::default().fg(ui::muted(theme)),
        )]
    } else {
        picker
            .ids
            .iter()
            .enumerate()
            .map(|(idx, id)| {
                let selected = idx == picker.selected;
                let marker = if selected { "▶ " } else { "  " };
                let style = if selected {
                    Style::default()
                        .fg(ui::accent(theme))
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(ui::fg(theme))
                };
                Line::from(vec![
                    Span::styled(marker, Style::default().fg(ui::accent(theme))),
                    Span::styled(id.clone(), style),
                ])
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(crate::t!("tui.configure.composer.picker_title"))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ui::accent(theme))),
        ),
        rect,
    );
}

/// 渲染来源 tab 栏，并在同一次遍历里为每个 tab 登记命中区——`source_tabs` 是
/// tab 几何的唯一事实来源，普通页和 draft 页共用，使末尾「+ 新建」tab 和真实来源
/// 一样可点。draft 打开时这条栏依旧存活（可点其他来源退出 draft）。
fn render_source_strip(frame: &mut Frame, page: &ConfigurePage, area: Rect, theme: &TuiTheme) {
    let tabs = source_tabs(page, theme);
    let inner_x = area.x + 1;
    let inner_y = area.y + 1;
    let inner_right = area.x + area.width.saturating_sub(1);
    let mut x = inner_x;
    for (idx, (_, width)) in tabs.iter().enumerate() {
        if x >= inner_right {
            break;
        }
        let hit_w = (*width).min(inner_right - x);
        page.hit
            .borrow_mut()
            .sources
            .push((Rect::new(x, inner_y, hit_w, 1), idx));
        x = x.saturating_add(*width);
    }
    let line = if tabs.is_empty() {
        Line::from(crate::t!("tui.configure.no_entries"))
    } else {
        Line::from(tabs.into_iter().map(|(span, _)| span).collect::<Vec<_>>())
    };
    frame.render_widget(
        Paragraph::new(line).wrap(Wrap { trim: false }).block(
            // 来源栏和字段同属内容区，一起在 Fields 焦点时高亮。
            block_for_focus(page, ConfigureFocus::Fields, theme)
                .title(crate::t!("tui.configure.sources"))
                .borders(Borders::ALL),
        ),
        area,
    );
}

/// 新建 draft：沿用普通模块的三段结构——来源 tab 栏 / 字段列表 / detail 面板，
/// 而不是独占一个大窗口。
fn render_draft(frame: &mut Frame, page: &ConfigurePage, area: Rect, theme: &TuiTheme) {
    let Some(draft) = page.draft.as_ref() else {
        return;
    };
    let (rows, selected, title, test_status) = match draft {
        Draft::Llm(form) => (
            form.rows(),
            form.selected,
            crate::t!("tui.configure.llm_create.title"),
            Some(&form.test_status),
        ),
        Draft::Asr(form) => (
            form.rows(),
            form.selected,
            crate::t!("tui.configure.asr_create.title"),
            None,
        ),
        Draft::Profile(form) => (
            form.rows(),
            form.selected,
            crate::t!("tui.configure.profile_create.title"),
            None,
        ),
    };

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    // 顶部：来源 tab 栏（末尾「+ 新建 LLM」高亮），和普通页共用同一渲染+命中登记。
    render_source_strip(frame, page, outer[0], theme);

    // 中部字段 + 底部 detail。
    let detail_h = detail_pane_height(outer[1].height);
    let (fields_area, detail_area) = if detail_h == 0 {
        (outer[1], None)
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(detail_h)])
            .split(outer[1]);
        (chunks[0], Some(chunks[1]))
    };

    let refs: Vec<&SettingsRow> = rows.iter().collect();
    let lines = draft_field_lines(&refs, selected, page, theme);
    let inner_h = fields_area.height.saturating_sub(2) as usize;
    let visible = ui::visible_range_for_selection(selected, lines.len(), inner_h);
    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(visible.start)
        .take(visible.end.saturating_sub(visible.start))
        .collect();
    frame.render_widget(
        // No wrap: each field is exactly one row（多行值在此只显示单行摘要）。
        Paragraph::new(visible_lines).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ui::accent(theme))),
        ),
        fields_area,
    );
    set_draft_inline_edit_cursor(frame, page, fields_area, &visible);

    // 每个可见 draft 字段行登记一个命中区，点它即选中对应的绝对行。
    let fx = fields_area.x + 1;
    let fy = fields_area.y + 1;
    let fw = fields_area.width.saturating_sub(2);
    for (local_idx, absolute_idx) in visible.clone().enumerate() {
        page.hit
            .borrow_mut()
            .fields
            .push((Rect::new(fx, fy + local_idx as u16, fw, 1), absolute_idx));
    }

    if let Some(detail_area) = detail_area {
        let inner_w = detail_area.width.saturating_sub(2) as usize;
        let selected = rows.get(selected);
        frame.render_widget(
            Paragraph::new(draft_detail_lines(selected, test_status, inner_w, theme))
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title(crate::t!("tui.configure.detail"))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(ui::muted(theme))),
                ),
            detail_area,
        );
    }
}

/// 连通性测试状态行（在 detail 顶部）。
fn draft_test_status_line(
    status: &crate::tui::configure::DraftTestStatus,
    theme: &TuiTheme,
) -> Option<Line<'static>> {
    use crate::tui::configure::DraftTestStatus;
    match status {
        DraftTestStatus::Idle => None,
        DraftTestStatus::Testing => Some(Line::styled(
            crate::t!("tui.configure.llm_create.test_testing"),
            Style::default().fg(ui::muted(theme)),
        )),
        DraftTestStatus::Ok => Some(Line::styled(
            crate::t!("tui.configure.llm_create.test_ok"),
            Style::default()
                .fg(ui::success(theme))
                .add_modifier(Modifier::BOLD),
        )),
        DraftTestStatus::Failed(message) => Some(Line::styled(
            crate::i18n::tr(
                "tui.configure.llm_create.test_failed",
                &[("error", message.clone())],
            ),
            Style::default().fg(ui::error(theme)),
        )),
    }
}

fn draft_field_lines(
    rows: &[&SettingsRow],
    selected: usize,
    page: &ConfigurePage,
    theme: &TuiTheme,
) -> Vec<Line<'static>> {
    rows.iter()
        .enumerate()
        .map(|(idx, row)| {
            let is_editing = page
                .editing
                .as_ref()
                .map(|e| e.target.draft_key() == Some(row.field_path.as_str()))
                .unwrap_or(false);
            field_line_with_edit(row, None, is_editing, idx == selected, page, theme)
        })
        .collect()
}

/// draft detail 面板：选中字段的说明 + 当前值。
fn draft_detail_lines(
    row: Option<&SettingsRow>,
    test_status: Option<&crate::tui::configure::DraftTestStatus>,
    width: usize,
    theme: &TuiTheme,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    if let Some(test_status) = test_status {
        if let Some(line) = draft_test_status_line(test_status, theme) {
            out.push(line);
            out.push(Line::from(""));
        }
    }
    let Some(row) = row else {
        if out.is_empty() {
            out.push(Line::styled(
                crate::t!("tui.configure.detail_empty"),
                Style::default().fg(ui::muted(theme)),
            ));
        }
        return out;
    };
    push_detail_field(
        &mut out,
        "key",
        &row.display_key,
        width,
        ui::fg(theme),
        theme,
    );
    push_detail_field(
        &mut out,
        "value",
        &detail_cell(&row.value),
        width,
        origin_style(row.origin, theme).fg.unwrap_or(ui::fg(theme)),
        theme,
    );
    if let Some(desc_key) = row.description_key {
        out.push(Line::from(""));
        out.push(Line::styled(
            crate::i18n::tr(desc_key, &[]),
            Style::default().fg(ui::muted(theme)),
        ));
    }
    out
}

fn centered_modal(area: Rect, w: u16, h: u16) -> Rect {
    let w = area.width.min(w);
    let h = area.height.min(h);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

fn render_modal(
    frame: &mut Frame,
    m: &crate::tui::configure::modal::ModalEditor,
    area: Rect,
    theme: &TuiTheme,
) {
    use crate::tui::configure::modal::ModalKind;
    if m.kind == ModalKind::KeyCapture {
        render_hotkey_modal(frame, m, area, theme);
        return;
    }
    let rect = centered_modal(area, 70, 16);
    let clear_rect = Rect::new(
        rect.x.saturating_sub(1),
        rect.y.saturating_sub(1),
        rect.width.saturating_add(2).min(area.width),
        rect.height.saturating_add(2).min(area.height),
    );
    frame.render_widget(Clear, clear_rect);
    let (title, hint) = match m.kind {
        ModalKind::Multiline => (
            crate::t!("tui.configure.modal.title_multiline"),
            crate::t!("tui.configure.modal.hint_multiline"),
        ),
        ModalKind::Array => (
            crate::t!("tui.configure.modal.title_array"),
            crate::t!("tui.configure.modal.hint_array"),
        ),
        ModalKind::Secret => (
            crate::t!("tui.configure.modal.title_secret"),
            crate::t!("tui.configure.modal.hint_secret"),
        ),
        ModalKind::KeyCapture => (
            crate::t!("tui.configure.modal.title_keycapture"),
            crate::t!("tui.configure.modal.hint_keycapture"),
        ),
    };
    // Secret buffer is shown in plaintext by design (see hint); the on-disk secret is never prefilled.
    let block = Block::default()
        .title(format!("{title} · {}", m.field_path))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ui::accent(theme)));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    // Content area scrolls to keep the cursor visible; the hint stays pinned to
    // the last inner rows. A hint may carry multiple lines (`\n`) and long ones
    // (e.g. the hotkey grammar/examples) — word-wrap them to the modal width and
    // size the hint area to the wrapped row count so nothing is truncated.
    let inner_w = (inner.width as usize).max(1);
    let hint_rows = ui::wrap_to_width(&hint, inner_w);
    // Keep at least one content row: cap the hint at inner height − 1.
    let hint_h = (hint_rows.len() as u16).clamp(1, inner.height.saturating_sub(1).max(1));
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(hint_h)])
        .split(inner);

    // Locate the cursor in the wrapped layout (char-boundary-safe). Greedy wrap
    // is prefix-deterministic, so wrapping the text before the cursor yields the
    // cursor's exact row (line count) and column (last line width).
    let mut cur = m.cursor.min(m.buffer.len());
    while cur > 0 && !m.buffer.is_char_boundary(cur) {
        cur -= 1;
    }
    let prefix = ui::wrap_to_width(&m.buffer[..cur], inner_w);
    let cursor_row = prefix.len().saturating_sub(1);
    let cursor_col = prefix.last().map(|s| ui::display_width(s)).unwrap_or(0);

    let content = ui::wrap_to_width(&m.buffer, inner_w);
    let content_h = parts[0].height.max(1) as usize;
    let scroll = cursor_row.saturating_sub(content_h.saturating_sub(1));
    frame.render_widget(
        Paragraph::new(content.into_iter().map(Line::from).collect::<Vec<_>>())
            .scroll((scroll as u16, 0)),
        parts[0],
    );
    frame.render_widget(
        Paragraph::new(
            hint_rows
                .into_iter()
                .map(|l| Line::styled(l, Style::default().fg(ui::muted(theme))))
                .collect::<Vec<_>>(),
        ),
        parts[1],
    );

    // Draw the real terminal cursor at the edit position — no text is shifted.
    if cursor_row >= scroll && cursor_row < scroll + content_h {
        let x = (parts[0].x + cursor_col as u16).min(parts[0].x + parts[0].width.saturating_sub(1));
        let y = parts[0].y + (cursor_row - scroll) as u16;
        frame.set_cursor_position(Position::new(x, y));
    }
}

/// Hotkey editing gets a dedicated layout: a single input line on top, then a
/// static syntax reference card in normal text (not a dim wrapped paragraph),
/// so the supported modifiers/keys/rules and examples read as a clean table.
fn render_hotkey_modal(
    frame: &mut Frame,
    m: &crate::tui::configure::modal::ModalEditor,
    area: Rect,
    theme: &TuiTheme,
) {
    let rect = centered_modal(area, 76, 18);
    let clear_rect = Rect::new(
        rect.x.saturating_sub(1),
        rect.y.saturating_sub(1),
        rect.width.saturating_add(2).min(area.width),
        rect.height.saturating_add(2).min(area.height),
    );
    frame.render_widget(Clear, clear_rect);

    let title = crate::t!("tui.configure.modal.title_keycapture");
    let block = Block::default()
        .title(format!("{title} · {}", m.field_path))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ui::accent(theme)));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    // input row · blank · reference (fills the rest).
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);
    let inner_w = (rows[0].width as usize).max(1);

    frame.render_widget(Paragraph::new(Line::from(m.buffer.as_str())), rows[0]);

    let reference = crate::t!("tui.configure.modal.hint_keycapture");
    frame.render_widget(
        Paragraph::new(
            ui::wrap_to_width(&reference, inner_w)
                .into_iter()
                .map(|l| Line::styled(l, Style::default().fg(ui::muted(theme))))
                .collect::<Vec<_>>(),
        ),
        rows[2],
    );

    // Single-line input: cursor column is the display width up to the cursor.
    let mut cur = m.cursor.min(m.buffer.len());
    while cur > 0 && !m.buffer.is_char_boundary(cur) {
        cur -= 1;
    }
    let col = ui::display_width(&m.buffer[..cur]) as u16;
    let x = (rows[0].x + col).min(rows[0].x + rows[0].width.saturating_sub(1));
    frame.set_cursor_position(Position::new(x, rows[0].y));
}

/// Render the fields area (with hint line when editing).
fn render_field_area(frame: &mut Frame, page: &ConfigurePage, area: Rect, theme: &TuiTheme) {
    let (field_area, hint_area) = if page.editing.is_some() && area.height > 2 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let lines = field_area_lines(page, theme);
    let total = lines.len();
    let inner_h = field_area.height.saturating_sub(2) as usize;
    // Keep the selected field visible by centering it, like the History list.
    let scroll = centered_scroll(selected_field_line(page, total), total, inner_h);

    // Backfill field hit regions (shifted by the scroll offset, visible rows only).
    {
        let module_label = page.module.inventory_module().label();
        let field_inner_x = field_area.x + 1;
        let field_inner_y = field_area.y + 1;
        let field_inner_w = field_area.width.saturating_sub(2);
        let line_offsets: Vec<u16> = match page.module {
            ConfigureModule::Overview | ConfigureModule::Main => {
                let keys: Vec<&str> = page
                    .rows
                    .iter()
                    .filter(|r| r.group == module_label)
                    .map(|r| r.display_key.as_str())
                    .collect();
                main_field_line_offsets(&keys)
            }
            ConfigureModule::Profile if page.composer.is_some() => {
                (0..page.composer.as_ref().map(|c| c.rows().len()).unwrap_or(0) as u16).collect()
            }
            _ => (0..selected_source_rows(page).len() as u16).collect(),
        };
        for (i, offset) in line_offsets.into_iter().enumerate() {
            // Skip fields scrolled out of view so clicks don't map to hidden rows.
            if offset < scroll || (offset - scroll) as usize >= inner_h {
                continue;
            }
            page.hit.borrow_mut().fields.push((
                Rect::new(
                    field_inner_x,
                    field_inner_y + offset - scroll,
                    field_inner_w,
                    1,
                ),
                i,
            ));
        }
    }

    // Render only the visible slice top-aligned (like the History list) instead
    // of Paragraph.scroll: scrolling wide (CJK) content through a scroll offset
    // leaves stale glyphs in the second cell of double-width chars, because
    // ratatui's frame diff doesn't reflush those spacer cells when a row shifts.
    let visible: Vec<Line> = lines
        .into_iter()
        .skip(scroll as usize)
        .take(inner_h.max(1))
        .collect();
    frame.render_widget(
        // No wrap: each field is exactly one row so mouse hit regions stay 1:1.
        Paragraph::new(visible).block(
            block_for_focus(page, ConfigureFocus::Fields, theme)
                .title(focused_title(
                    page,
                    ConfigureFocus::Fields,
                    field_area_title(page),
                ))
                .borders(Borders::ALL),
        ),
        field_area,
    );

    if let (Some(rect), Some(edit)) = (hint_area, &page.editing) {
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                edit_hint(edit),
                Style::default().fg(ui::muted(theme)),
            )])),
            rect,
        );
    }

    set_inline_edit_cursor(frame, page, field_area, theme);
}

/// Fields list on top, a full-detail pane for the selected field below (when
/// the area is tall enough). The detail pane shows the untruncated key, value,
/// default and description, wrapped, and scrolls for long content.
fn render_fields_and_detail(frame: &mut Frame, page: &ConfigurePage, area: Rect, theme: &TuiTheme) {
    let detail_h = detail_pane_height(area.height);
    if detail_h == 0 {
        page.hit.borrow_mut().detail = None;
        render_field_area(frame, page, area, theme);
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(detail_h)])
        .split(area);
    render_field_area(frame, page, chunks[0], theme);
    render_detail(frame, page, chunks[1], theme);
}

/// Height reserved for the detail pane, or 0 to hide it on short terminals so
/// the field list keeps enough room.
fn detail_pane_height(total: u16) -> u16 {
    if total >= 16 {
        8
    } else if total >= 11 {
        5
    } else {
        0
    }
}

fn render_detail(frame: &mut Frame, page: &ConfigurePage, area: Rect, theme: &TuiTheme) {
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner_h = area.height.saturating_sub(2);
    let lines = detail_lines(page, theme, inner_w);
    let max_scroll = (lines.len() as u16).saturating_sub(inner_h);
    page.detail_max_scroll.set(max_scroll);
    let scroll = page.detail_scroll.min(max_scroll);
    page.hit.borrow_mut().detail = Some(area);
    // Render the visible slice top-aligned rather than Paragraph.scroll — same
    // wide-char diff issue as the field list (see render_field_area).
    let visible: Vec<Line> = lines
        .into_iter()
        .skip(scroll as usize)
        .take((inner_h as usize).max(1))
        .collect();
    frame.render_widget(
        // Lines are pre-wrapped, so no `Wrap`: the slice maps 1:1 to rows.
        Paragraph::new(visible).block(
            Block::default()
                .title(crate::t!("tui.configure.detail"))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ui::muted(theme))),
        ),
        area,
    );
}

/// Full detail for the selected field, pre-wrapped to `width`.
pub(super) fn detail_lines(
    page: &ConfigurePage,
    theme: &TuiTheme,
    width: usize,
) -> Vec<Line<'static>> {
    // In Profile the field list is the composer's rows; show the selected one.
    let composer_row = if page.module == ConfigureModule::Profile {
        page.composer
            .as_ref()
            .and_then(|c| c.rows().get(c.selected))
            .map(|crow| &crow.row)
    } else {
        None
    };
    let Some(row) = composer_row.or_else(|| page.selected_settings_row()) else {
        return vec![Line::styled(
            crate::t!("tui.configure.detail_empty"),
            Style::default().fg(ui::muted(theme)),
        )];
    };
    let mut out = Vec::new();
    push_detail_field(
        &mut out,
        "key",
        &row.field_path,
        width,
        ui::fg(theme),
        theme,
    );
    push_detail_field(
        &mut out,
        "value",
        &detail_cell(&row.value),
        width,
        origin_style(row.origin, theme).fg.unwrap_or(ui::fg(theme)),
        theme,
    );
    push_detail_field(
        &mut out,
        "default",
        &detail_cell(&row.default_value),
        width,
        ui::muted(theme),
        theme,
    );
    if let Some(desc_key) = row.description_key {
        let desc = crate::i18n::tr(desc_key, &[]);
        if !desc.trim().is_empty() {
            out.push(Line::from(""));
            for seg in ui::wrap_to_width(&desc, width.max(1)) {
                out.push(Line::styled(seg, Style::default().fg(ui::muted(theme))));
            }
        }
    }
    out
}

/// Push a `label: value` field, wrapping the value and aligning continuation
/// lines under the value column.
fn push_detail_field(
    out: &mut Vec<Line<'static>>,
    label: &str,
    value: &str,
    width: usize,
    color: Color,
    theme: &TuiTheme,
) {
    let prefix = format!("{label}: ");
    let prefix_w = ui::display_width(&prefix);
    let body_w = width.saturating_sub(prefix_w).max(1);
    let segs = ui::wrap_to_width(value, body_w);
    for (i, seg) in segs.into_iter().enumerate() {
        if i == 0 {
            out.push(Line::from(vec![
                Span::styled(prefix.clone(), Style::default().fg(ui::muted(theme))),
                Span::styled(seg, Style::default().fg(color)),
            ]));
        } else {
            out.push(Line::from(vec![
                Span::raw(" ".repeat(prefix_w)),
                Span::styled(seg, Style::default().fg(color)),
            ]));
        }
    }
}

/// Rendered line index of the selected field within `field_area_lines`.
/// Main/Overview interleave section headers, so the field index isn't the line
/// index; per-source modules are 1:1.
fn selected_field_line(page: &ConfigurePage, total_lines: usize) -> usize {
    if page.module == ConfigureModule::Profile {
        if let Some(composer) = page.composer.as_ref() {
            return composer.selected.min(total_lines.saturating_sub(1));
        }
    }
    match page.module {
        ConfigureModule::Overview | ConfigureModule::Main => {
            let label = ConfigureModule::Main.inventory_module().label();
            let keys: Vec<&str> = page
                .rows
                .iter()
                .filter(|r| r.group == label)
                .map(|r| r.display_key.as_str())
                .collect();
            main_field_line_offsets(&keys)
                .get(page.selected)
                .map(|offset| *offset as usize)
                .unwrap_or(0)
        }
        _ => page.selected.min(total_lines.saturating_sub(1)),
    }
}

/// Scroll offset (in lines) that centers `selected_line` in a `view_h`-row
/// viewport, clamped so the list never scrolls past its end. 0 when it all fits.
pub(super) fn centered_scroll(selected_line: usize, total: usize, view_h: usize) -> u16 {
    ui::visible_range_for_selection(selected_line, total, view_h).start as u16
}

/// Lines for the fields area (either main grouped or per-source detail).
fn field_area_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
    match page.module {
        ConfigureModule::Overview | ConfigureModule::Main => main_grouped_lines(page, theme),
        ConfigureModule::Profile if page.composer.is_some() => composer_field_lines(page, theme),
        _ => {
            // 支持新建的末尾「+ 新建」槽位：提示回车进入新建，而非「未选中」。
            if page.module.supports_new()
                && page.selected_source_idx == page.sources_for_current_module().len()
            {
                let key = match page.module {
                    ConfigureModule::AsrProvider => "tui.configure.asr_create.enter_to_create",
                    _ => "tui.configure.llm_create.enter_to_create",
                };
                return vec![Line::styled(
                    crate::i18n::tr(key, &[]),
                    Style::default().fg(ui::muted(theme)),
                )];
            }
            let rows = selected_source_rows(page);
            if rows.is_empty() {
                return vec![Line::from(crate::t!("tui.configure.no_config_selected"))];
            }
            field_lines_with_edit(rows, page, theme)
        }
    }
}

pub(super) fn module_nav_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
    all_modules()
        .into_iter()
        .map(|module| {
            let selected = module == page.module
                || (module == ConfigureModule::Main && page.module == ConfigureModule::Overview);
            let marker = if selected {
                if page.focus == ConfigureFocus::Modules {
                    "> "
                } else {
                    "* "
                }
            } else {
                "  "
            };
            let style = if selected {
                Style::default()
                    .fg(ui::accent(theme))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ui::segment(theme))
            };
            let label = module.inventory_module().label();
            let (errors, missing) = page
                .overview_counts
                .iter()
                .find(|(l, _, _)| l == label)
                .map(|(_, e, m)| (*e, *m))
                .unwrap_or((0, 0));
            let mut spans = vec![
                Span::styled(marker, style),
                Span::styled(module.title(), style),
            ];
            if let Some(mark) = module_problem_marker(errors, missing) {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(mark, Style::default().fg(ui::error(theme))));
            }
            Line::from(spans)
        })
        .collect()
}

/// Fields-panel title: module name, plus the selected source name for
/// per-source modules so it is clear which file is being edited.
fn field_area_title(page: &ConfigurePage) -> String {
    let base = page.module.title();
    match page.module {
        ConfigureModule::Overview | ConfigureModule::Main => base,
        _ => match page.selected_config_source() {
            Some(source) => format!("{base} · {}", source_name(&source)),
            None => base,
        },
    }
}

fn focused_title(page: &ConfigurePage, focus: ConfigureFocus, title: String) -> String {
    if page.focus == focus {
        format!("> {title}")
    } else {
        title
    }
}

fn block_for_focus(
    page: &ConfigurePage,
    focus: ConfigureFocus,
    theme: &TuiTheme,
) -> Block<'static> {
    if page.focus == focus {
        Block::default().border_style(Style::default().fg(ui::accent(theme)))
    } else {
        Block::default()
            .border_style(Style::default().fg(ui::muted(theme)))
            .title_style(Style::default().fg(ui::muted(theme)))
    }
}

fn all_modules() -> Vec<ConfigureModule> {
    vec![
        ConfigureModule::Main,
        ConfigureModule::Profile,
        ConfigureModule::AsrProvider,
        ConfigureModule::PostProcessor,
    ]
}

pub(super) fn main_grouped_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
    let rows = page
        .rows
        .iter()
        .filter(|row| row.group == ConfigureModule::Main.inventory_module().label())
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return vec![Line::from(crate::t!("tui.configure.no_entries"))];
    }
    let mut lines = Vec::new();
    let mut current_section = String::new();
    // Enumerate counts only field rows (section headers are extra lines, not row entries).
    for (field_row_index, row) in rows.iter().enumerate() {
        let (section, item_key) = split_main_display_key(&row.display_key);
        if section != current_section {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            current_section = section.clone();
            lines.push(Line::styled(
                section,
                Style::default().fg(ui::accent(theme)),
            ));
        }
        let is_editing = page.editing.as_ref().is_some_and(|e| {
            e.field_path == row.field_path
                && e.target
                    .file_path()
                    .is_some_and(|p| p.to_string_lossy() == row.source)
        });
        let selected = field_row_index == page.selected;
        lines.push(field_line_with_edit(
            row,
            Some(&item_key),
            is_editing,
            selected,
            page,
            theme,
        ));
    }
    lines
}

/// Rendered line index (relative to the panel's first inner row) of each field
/// in the grouped Main view. Mirrors `main_grouped_lines`: a section header
/// precedes the first field of each section, with a blank separator before
/// every section after the first. Used to place mouse hit regions so clicks
/// land on the right field.
pub(super) fn main_field_line_offsets(display_keys: &[&str]) -> Vec<u16> {
    let mut offsets = Vec::with_capacity(display_keys.len());
    let mut current_section = String::new();
    let mut line: u16 = 0;
    let mut first = true;
    for key in display_keys {
        let (section, _) = split_main_display_key(key);
        if first || section != current_section {
            if !first {
                line += 1; // blank separator before a new section
            }
            line += 1; // section header
            current_section = section;
            first = false;
        }
        offsets.push(line);
        line += 1; // the field row itself
    }
    offsets
}

pub(super) fn split_main_display_key(key: &str) -> (String, String) {
    key.split_once('.')
        .map(|(section, rest)| (section.to_string(), rest.to_string()))
        .unwrap_or_else(|| ("root".to_string(), key.to_string()))
}

/// The source tab strip as a single ordered list of (styled span, display
/// width). It is the one source of truth for the strip: real sources plus the
/// trailing "+ New LLM" tab on PostProcessor — the +New tab is just the last
/// entry, styled and measured exactly like a real source, so rendering and
/// mouse hit-testing can never drift apart.
pub(super) fn source_tabs(page: &ConfigurePage, theme: &TuiTheme) -> Vec<(Span<'static>, u16)> {
    let sources = page.sources_for_current_module();
    let mut names: Vec<String> = sources.iter().map(|s| source_name(s)).collect();
    if page.module.supports_new() {
        let key = match page.module {
            ConfigureModule::AsrProvider => "tui.configure.asr_create.new_entry",
            ConfigureModule::Profile => "tui.configure.profile_create.new_entry",
            _ => "tui.configure.llm_create.new_entry",
        };
        names.push(crate::i18n::tr(key, &[]));
    }
    names
        .into_iter()
        .enumerate()
        .map(|(idx, name)| {
            let (text, style) = if idx == page.selected_source_idx {
                (
                    format!("[ {name} ]  "),
                    Style::default()
                        .fg(ui::accent(theme))
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                (
                    format!("  {name}    "),
                    Style::default().fg(ui::muted(theme)),
                )
            };
            let width = ui::display_width(&text) as u16;
            (Span::styled(text, style), width)
        })
        .collect()
}

/// Test-only: the rendered source strip as `Line`s, for asserting strip text.
/// 生产渲染走 `render_source_strip`（渲染+命中登记一次遍历），这里只复用 `source_tabs`。
#[cfg(test)]
pub(super) fn source_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
    let tabs = source_tabs(page, theme);
    if tabs.is_empty() {
        return vec![Line::from(crate::t!("tui.configure.no_entries"))];
    }
    vec![Line::from(
        tabs.into_iter().map(|(span, _)| span).collect::<Vec<_>>(),
    )]
}

/// Render field rows for source detail (respects edit state).
pub(super) fn field_lines_with_edit(
    rows: Vec<&SettingsRow>,
    page: &ConfigurePage,
    theme: &TuiTheme,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return vec![Line::from(crate::t!("tui.configure.no_entries"))];
    }
    rows.iter()
        .enumerate()
        .map(|(idx, row)| {
            let is_editing = page.editing.as_ref().is_some_and(|e| {
                e.field_path == row.field_path
                    && e.target
                        .file_path()
                        .is_some_and(|p| p.to_string_lossy() == row.source)
            });
            let selected =
                page.focus == crate::tui::configure::ConfigureFocus::Fields && idx == page.selected;
            field_line_with_edit(row, None, is_editing, selected, page, theme)
        })
        .collect()
}

/// Field lines for the Profile composer. Scalar/override rows reuse the normal
/// field-line renderer; `SectionHeader`/`ChainMember` rows render as label-only
/// lines (no value column) in their origin color.
fn composer_field_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
    use crate::tui::configure::profile_composer::ComposerRowKind;
    let Some(composer) = page.composer.as_ref() else {
        return vec![Line::from(crate::t!("tui.configure.no_config_selected"))];
    };
    let selected_row = composer.selected;
    composer
        .rows()
        .iter()
        .enumerate()
        .map(|(idx, crow)| {
            let selected = idx == selected_row;
            match crow.kind {
                ComposerRowKind::SectionHeader | ComposerRowKind::ChainMember { .. } => {
                    composer_label_line(&crow.row, selected, theme)
                }
                _ => {
                    let is_editing = page
                        .editing
                        .as_ref()
                        .map(|e| e.target.is_composer() && e.field_path == crow.row.field_path)
                        .unwrap_or(false);
                    field_line_with_edit(&crow.row, None, is_editing, selected, page, theme)
                }
            }
        })
        .collect()
}

/// A label-only composer line (section header or chain member): marker + key in
/// the row's origin color, no value/description columns.
fn composer_label_line(row: &SettingsRow, selected: bool, theme: &TuiTheme) -> Line<'static> {
    let marker = if selected { "▶ " } else { "  " };
    let row_bold = if selected {
        Modifier::BOLD
    } else {
        Modifier::empty()
    };
    Line::from(vec![
        Span::styled(
            marker,
            Style::default()
                .fg(ui::accent(theme))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            row.display_key.clone(),
            origin_style(row.origin, theme).add_modifier(row_bold),
        ),
    ])
}

/// Three-column field row: field_path | value | description.
/// Selected row: bold `"▶ "` marker + all spans BOLD.
/// Set values: highlight (green); default values: muted (grey).
fn field_line_with_edit(
    row: &SettingsRow,
    key_override: Option<&str>,
    is_editing: bool,
    selected: bool,
    page: &ConfigurePage,
    theme: &TuiTheme,
) -> Line<'static> {
    let display_key = key_override.unwrap_or(&row.display_key);
    let (value_cell, value_style) = if is_editing {
        if let Some(edit) = &page.editing {
            (
                edit_value_cell(edit),
                Style::default().fg(ui::accent(theme)),
            )
        } else {
            (
                compact_value(&row.value),
                Style::default().fg(ui::segment(theme)),
            )
        }
    } else {
        (display_cell(&row.value), origin_style(row.origin, theme))
    };

    let marker = if selected { "▶ " } else { "  " };
    let row_bold = if selected {
        Modifier::BOLD
    } else {
        Modifier::empty()
    };
    let key_style = Style::default().fg(ui::fg(theme)).add_modifier(row_bold);

    Line::from(vec![
        Span::styled(
            marker,
            Style::default()
                .fg(ui::accent(theme))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(pad_display(display_key, 22), key_style),
        Span::styled(" ", Style::default().add_modifier(row_bold)),
        Span::styled(
            pad_display(&value_cell, 28),
            value_style.add_modifier(row_bold),
        ),
        Span::styled("  ", Style::default().add_modifier(row_bold)),
        Span::styled(
            row.description_key
                .map(|key| crate::i18n::tr(key, &[]))
                .unwrap_or_default(),
            Style::default().fg(ui::muted(theme)).add_modifier(row_bold),
        ),
    ])
}

fn compact_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 左对齐补齐到 `width` 显示单元格：先按显示宽度截断（超出加「…」），再补空格。
/// 全程用 `ui::display_width`（CJK=2、`•`/`—`/`…`=1），保证截断与补齐都不错位。
fn pad_display(value: &str, width: usize) -> String {
    let value = truncate_to_width(value, width);
    let padding = width.saturating_sub(ui::display_width(&value));
    format!("{value}{}", " ".repeat(padding))
}

/// Truncate to at most `max_width` display cells, appending `…` when cut so the
/// result still fits (the ellipsis is one cell). CJK-aware so a cell never
/// overflows its column and pushes trailing content out of alignment.
fn truncate_to_width(value: &str, max_width: usize) -> String {
    if ui::display_width(value) <= max_width {
        return value.to_string();
    }
    let budget = max_width.saturating_sub(1); // reserve one cell for the ellipsis
    let mut out = String::new();
    let mut w = 0;
    for ch in value.chars() {
        let cw = ui::char_width(ch);
        if w + cw > budget {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// Value shown in a field cell. Empty values render as an em dash so the row
/// reads as "unset" rather than looking blank or broken.
fn display_cell(value: &str) -> String {
    let compact = compact_value(value);
    if compact.is_empty() {
        "—".to_string()
    } else {
        compact
    }
}

/// Value shown in the detail pane: empty -> em dash, otherwise verbatim so
/// meaningful whitespace and newlines (e.g. a multi-line prompt) survive —
/// unlike the compact one-line list cell.
fn detail_cell(value: &str) -> String {
    if value.trim().is_empty() {
        "—".to_string()
    } else {
        value.to_string()
    }
}

/// A module in the left nav shows a red problem marker only when it has
/// validation errors or missing required fields; otherwise nothing (the raw
/// field count was noise).
fn module_problem_marker(errors: usize, missing: usize) -> Option<&'static str> {
    if errors + missing > 0 {
        Some("●")
    } else {
        None
    }
}

fn origin_style(origin: FieldOrigin, theme: &TuiTheme) -> Style {
    match origin {
        FieldOrigin::Set => Style::default().fg(ui::info(theme)),
        FieldOrigin::Error => Style::default().fg(ui::error(theme)),
        FieldOrigin::Default | FieldOrigin::RequiredUnset => Style::default().fg(ui::fg(theme)),
    }
}

fn edit_value_cell(edit: &crate::tui::configure::EditState) -> String {
    use crate::config::field_view::ControlKind;
    match &edit.control {
        ControlKind::Toggle => format!("[{}]", edit.buffer),
        ControlKind::Select(_) => format!("◀ {} ▶", edit.buffer),
        ControlKind::Number { .. } | ControlKind::Text => compact_value(&edit.buffer),
        ControlKind::MultilineText | ControlKind::Array | ControlKind::KeyCapture => {
            edit.buffer.clone()
        }
        ControlKind::ReadOnly => edit.buffer.clone(),
    }
}

fn set_inline_edit_cursor(
    frame: &mut Frame,
    page: &ConfigurePage,
    field_area: Rect,
    theme: &TuiTheme,
) {
    let Some(edit) = page.editing.as_ref() else {
        return;
    };
    if !matches!(
        edit.control,
        crate::config::field_view::ControlKind::Text
            | crate::config::field_view::ControlKind::Number { .. }
    ) {
        return;
    }

    // Composer inline edits: the selected line is `composer.selected`.
    if edit.target.is_composer() {
        if let Some(composer) = page.composer.as_ref() {
            let total = field_area_lines(page, theme).len();
            let inner_h = field_area.height.saturating_sub(2) as usize;
            let selected_line = composer.selected;
            let scroll = centered_scroll(selected_field_line(page, total), total, inner_h) as usize;
            if selected_line < scroll || selected_line >= scroll + inner_h {
                return;
            }
            place_inline_cursor(frame, edit, field_area, selected_line - scroll);
        }
        return;
    }

    let rows = match page.module {
        ConfigureModule::Overview | ConfigureModule::Main => page
            .rows
            .iter()
            .filter(|r| r.group == ConfigureModule::Main.inventory_module().label())
            .collect::<Vec<_>>(),
        _ => selected_source_rows(page),
    };
    let Some(row_idx) = rows.iter().position(|row| {
        row.field_path == edit.field_path
            && edit
                .target
                .file_path()
                .is_none_or(|p| p.to_string_lossy() == row.source)
    }) else {
        return;
    };

    let total = field_area_lines(page, theme).len();
    let inner_h = field_area.height.saturating_sub(2) as usize;
    let selected_line = match page.module {
        ConfigureModule::Overview | ConfigureModule::Main => {
            let keys: Vec<&str> = rows.iter().map(|r| r.display_key.as_str()).collect();
            main_field_line_offsets(&keys)
                .get(row_idx)
                .copied()
                .unwrap_or(0) as usize
        }
        _ => row_idx,
    };
    let scroll = centered_scroll(selected_field_line(page, total), total, inner_h) as usize;
    if selected_line < scroll || selected_line >= scroll + inner_h {
        return;
    }
    place_inline_cursor(frame, edit, field_area, selected_line - scroll);
}

/// Place the terminal cursor on an inline Text/Number edit: field column offset
/// (marker + 22-wide key + gap) plus the value prefix width, on row `row_offset`.
fn place_inline_cursor(
    frame: &mut Frame,
    edit: &crate::tui::configure::EditState,
    field_area: Rect,
    row_offset: usize,
) {
    let mut cur = edit.cursor.min(edit.buffer.len());
    while cur > 0 && !edit.buffer.is_char_boundary(cur) {
        cur -= 1;
    }
    let prefix_width = ui::display_width(&compact_value(&edit.buffer[..cur]));
    let x = field_area
        .x
        .saturating_add(1)
        .saturating_add(2)
        .saturating_add(22)
        .saturating_add(1)
        .saturating_add(prefix_width.min(27) as u16)
        .min(field_area.x + field_area.width.saturating_sub(2));
    let y = field_area.y + 1 + row_offset as u16;
    frame.set_cursor_position(Position::new(x, y));
}

fn set_draft_inline_edit_cursor(
    frame: &mut Frame,
    page: &ConfigurePage,
    fields_area: Rect,
    visible: &std::ops::Range<usize>,
) {
    let Some(edit) = page.editing.as_ref() else {
        return;
    };
    if edit.target.draft_key().is_none()
        || !matches!(
            edit.control,
            crate::config::field_view::ControlKind::Text
                | crate::config::field_view::ControlKind::Number { .. }
        )
    {
        return;
    }
    let Some(row_idx) = page
        .draft_rows()
        .iter()
        .position(|row| edit.target.draft_key() == Some(row.field_path.as_str()))
    else {
        return;
    };
    if !visible.contains(&row_idx) {
        return;
    }
    let mut cur = edit.cursor.min(edit.buffer.len());
    while cur > 0 && !edit.buffer.is_char_boundary(cur) {
        cur -= 1;
    }
    let prefix_width = ui::display_width(&compact_value(&edit.buffer[..cur]));
    let x = fields_area
        .x
        .saturating_add(1)
        .saturating_add(2)
        .saturating_add(22)
        .saturating_add(1)
        .saturating_add(prefix_width.min(27) as u16)
        .min(fields_area.x + fields_area.width.saturating_sub(2));
    let y = fields_area.y + 1 + (row_idx - visible.start) as u16;
    frame.set_cursor_position(Position::new(x, y));
}

fn edit_hint(edit: &crate::tui::configure::EditState) -> String {
    use crate::config::field_view::ControlKind;
    match &edit.control {
        ControlKind::Toggle => crate::t!("tui.configure.edit.hint_toggle"),
        ControlKind::Select(_) => crate::t!("tui.configure.edit.hint_select"),
        ControlKind::Number { min, max, .. } => crate::i18n::tr(
            "tui.configure.edit.hint_number",
            &[("min", fmt_opt(*min)), ("max", fmt_opt(*max))],
        ),
        ControlKind::Text => crate::t!("tui.configure.edit.hint_text"),
        ControlKind::MultilineText | ControlKind::Array | ControlKind::KeyCapture => String::new(),
        ControlKind::ReadOnly => String::new(),
    }
}

fn fmt_opt(v: Option<f64>) -> String {
    v.map(|v| v.to_string()).unwrap_or_else(|| "-".into())
}

fn render_error_popup(
    frame: &mut Frame,
    err: &crate::tui::configure::EditError,
    area: Rect,
    theme: &TuiTheme,
) {
    let rect = centered_modal(area, 60, 6);
    frame.render_widget(Clear, rect);
    let lines = vec![
        Line::from(format!("{} = {}", err.field_path, err.value)),
        Line::from(err.message.clone()),
        Line::from(""),
        Line::from(crate::t!("tui.configure.edit.dismiss")),
    ];
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(crate::t!("tui.configure.edit.invalid_title"))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ui::error(theme))),
        ),
        rect,
    );
}

pub(super) fn selected_source_rows(page: &ConfigurePage) -> Vec<&SettingsRow> {
    let Some(source) = page.selected_config_source() else {
        return Vec::new();
    };
    let module = page.module.inventory_module();
    page.rows
        .iter()
        .filter(|row| row.group == module.label() && std::path::Path::new(&row.source) == source)
        .collect()
}

pub(super) fn overview_lines(
    page: &ConfigurePage,
    theme: &TuiTheme,
    footer_status: &str,
) -> Vec<Line<'static>> {
    let config_path = crate::config::default_path().display().to_string();
    let mut lines = vec![
        kv_line(theme, "config root", config_path, ui::warning(theme)),
        Line::from(""),
        Line::from(vec![
            label_span("module", theme),
            Span::raw("        "),
            label_span("items", theme),
            Span::raw("  "),
            label_span("errors", theme),
            Span::raw("  "),
            label_span("missing", theme),
        ]),
    ];
    for module in all_modules() {
        let label = module.inventory_module().label();
        let rows_count = page.rows.iter().filter(|row| row.group == label).count();
        let (errors, missing) = page
            .overview_counts
            .iter()
            .find(|(l, _, _)| l == label)
            .map(|(_, e, m)| (*e, *m))
            .unwrap_or((0, 0));
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<13}", module.title()),
                Style::default().fg(ui::accent(theme)),
            ),
            Span::styled(
                format!("{:>5}", rows_count),
                Style::default().fg(ui::fg(theme)),
            ),
            Span::styled(
                format!("{:>8}", errors),
                Style::default().fg(status_count_color(errors, theme)),
            ),
            Span::styled(
                format!("{:>9}", missing),
                Style::default().fg(status_count_color(missing, theme)),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.extend(status_lines(page, theme, footer_status));
    lines
}

fn status_lines(page: &ConfigurePage, theme: &TuiTheme, footer_status: &str) -> Vec<Line<'static>> {
    vec![
        kv_line(
            theme,
            "doctor",
            doctor_status_value(page),
            ui::success(theme),
        ),
        kv_line(
            theme,
            "reload/status",
            footer_status.to_string(),
            ui::fg(theme),
        ),
    ]
}

fn doctor_status_value(page: &ConfigurePage) -> String {
    match &page.doctor.status {
        Some(status) => status.clone(),
        None => crate::t!("tui.configure.doctor_not_run"),
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

// ---- shared UI helpers ----

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

fn label_span(text: impl Into<String>, theme: &TuiTheme) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(ui::muted(theme)))
}

fn value_span(text: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(color))
}

// ---- tests ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_value_renders_as_dash() {
        assert_eq!(display_cell(""), "—");
        assert_eq!(display_cell("   "), "—");
    }

    #[test]
    fn non_empty_value_is_compacted_not_dashed() {
        assert_eq!(display_cell("hello"), "hello");
        assert_eq!(display_cell("a  b\tc"), "a b c");
    }

    #[test]
    fn module_marker_only_when_problems_exist() {
        assert_eq!(module_problem_marker(0, 0), None);
        assert_eq!(module_problem_marker(1, 0), Some("●"));
        assert_eq!(module_problem_marker(0, 2), Some("●"));
        assert_eq!(module_problem_marker(3, 4), Some("●"));
    }

    #[test]
    fn detail_pane_hidden_on_short_terminals() {
        assert_eq!(detail_pane_height(8), 0);
        assert_eq!(detail_pane_height(10), 0);
        assert_eq!(detail_pane_height(11), 5);
        assert_eq!(detail_pane_height(15), 5);
        assert_eq!(detail_pane_height(16), 8);
        assert_eq!(detail_pane_height(40), 8);
    }
}
