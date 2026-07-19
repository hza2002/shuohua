use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs, Wrap};
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::tui::page::{KeyHint, Page as _};
use crate::tui::ui;
use crate::tui::{App, Page};

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let (hint_text, status_text) = footer_parts(app);
    let footer_rows = footer_rows_needed(&hint_text, &status_text, area.width);

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(footer_rows),
        ])
        .split(area);

    let tabs = Tabs::new([
        crate::t!("tui.tab_status"),
        crate::t!("tui.tab_history"),
        crate::t!("tui.tab_configure"),
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

    let page_theme: &TuiTheme = &app.theme;
    match app.page {
        Page::Status => app
            .status_page
            .render(frame, root[1], page_theme, &app.status),
        Page::History => app.history.render(frame, root[1], page_theme, &app.status),
        Page::Configure => app
            .configure
            .render(frame, root[1], page_theme, &app.status),
    }

    let mut spans = vec![Span::styled(
        hint_text,
        Style::default().fg(ui::muted(&app.theme)),
    )];
    if !status_text.is_empty() {
        spans.push(Span::styled(
            format!("   {status_text}"),
            Style::default().fg(ui::fg(&app.theme)),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).wrap(Wrap { trim: false }),
        root[2],
    );
}

/// Map a click in the top tab bar to its page. Mirrors ratatui `Tabs` layout:
/// each tab is `pad_left(1) + title + pad_right(1)`, with a 1-cell divider
/// between tabs; the text sits on the top row of the 3-row header band.
pub fn tab_at(column: u16, row: u16) -> Option<Page> {
    if row > 1 {
        return None;
    }
    let tabs = [
        (crate::t!("tui.tab_status"), Page::Status),
        (crate::t!("tui.tab_history"), Page::History),
        (crate::t!("tui.tab_configure"), Page::Configure),
    ];
    let mut start = 0u16;
    let last = tabs.len() - 1;
    for (i, (label, page)) in tabs.iter().enumerate() {
        let divider = if i < last { 1 } else { 0 };
        let seg = 1 + ui::display_width(label) as u16 + 1 + divider;
        if column >= start && column < start + seg {
            return Some(*page);
        }
        start += seg;
    }
    None
}

/// Global keybindings available on every page. Single source for the footer;
/// `keybindings::action_for` handles the matching keys.
fn global_hints() -> Vec<KeyHint> {
    // `/` search is not here: it only works on the History page, so that page
    // contributes its own search hint (footer hints mirror the active keys).
    vec![
        KeyHint::new("Tab", "tui.hint.pages"),
        KeyHint::new("1/2/3", "tui.hint.jump"),
        KeyHint::new("q", "tui.hint.quit"),
    ]
}

fn page_hints(app: &App) -> Vec<KeyHint> {
    match app.page {
        Page::Status => app.status_page.key_hints(),
        Page::History => app.history.key_hints(),
        Page::Configure => app.configure.key_hints(),
    }
}

/// Returns `(hint_text, status_text)` for the footer.
fn footer_parts(app: &App) -> (String, String) {
    let mut hints = global_hints();
    hints.extend(page_hints(app));
    let hint_text = hints
        .iter()
        .map(|hint| format!("{} {}", hint.keys, crate::i18n::tr(hint.label_key, &[])))
        .collect::<Vec<_>>()
        .join("  ");

    (hint_text, app.status.clone())
}

/// Footer grows to a second line when hints + status overflow one row, so no
/// registered key is ever hidden. Capped at 2 rows.
fn footer_rows_needed(hint_text: &str, status_text: &str, width: u16) -> u16 {
    if width == 0 {
        return 1;
    }
    let mut total = ui::display_width(hint_text);
    if !status_text.is_empty() {
        total += 3 + ui::display_width(status_text);
    }
    total.div_ceil(width as usize).clamp(1, 2) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_footer_shows_all_record_actions() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.page = Page::History;
        let (hints, _) = footer_parts(&app);
        // Actions that used to be missing / drifting must now appear, with keys.
        assert!(hints.contains("m more"), "load-more hint missing: {hints}");
        assert!(
            hints.contains("x del history"),
            "delete hint missing: {hints}"
        );
        assert!(
            hints.contains("s analytics"),
            "analytics hint missing: {hints}"
        );
    }

    #[test]
    fn settings_footer_shows_validate_reload_and_open() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.page = Page::Configure;
        let (hints, _) = footer_parts(&app);
        assert!(
            hints.contains("v validate"),
            "validate hint missing: {hints}"
        );
        assert!(hints.contains("R reload"), "reload hint missing: {hints}");
        assert!(hints.contains("e open"), "open-file hint missing: {hints}");
    }

    #[test]
    fn settings_footer_status_is_only_transient_status() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.page = Page::Configure;
        app.status = "ready".to_string();
        app.configure.module = crate::tui::configure::ConfigureModule::Main;
        app.configure.focus = crate::tui::configure::ConfigureFocus::Fields;
        app.configure.selected = 0;
        app.configure.rows = vec![crate::tui::settings::SettingsRow {
            group: "main".to_string(),
            field_path: "ui.language".to_string(),
            display_key: "ui.language".to_string(),
            value: "zh-CN".to_string(),
            default_value: "auto".to_string(),
            origin: crate::config::field_view::FieldOrigin::Set,
            control: crate::config::field_view::ControlKind::Text,
            editable: true,
            secret: false,
            can_unset: true,
            source: "config.toml".to_string(),
            description_key: None,
        }];

        let (_, status) = footer_parts(&app);

        assert_eq!(status, "ready");
    }

    #[test]
    fn search_hint_appears_only_on_history_page() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.page = Page::History;
        let (hints, _) = footer_parts(&app);
        assert!(
            hints.contains("/ search"),
            "History must hint search: {hints}"
        );

        for page in [Page::Status, Page::Configure] {
            app.page = page;
            let (hints, _) = footer_parts(&app);
            assert!(
                !hints.contains("/ search"),
                "{page:?} must not hint search (it does nothing there): {hints}"
            );
        }
    }

    #[test]
    fn analytics_footer_hides_search_and_internal_anchor_wording() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.page = Page::History;
        app.history.view = crate::tui::history::HistoryView::Analytics;

        let (hints, _) = footer_parts(&app);

        assert!(!hints.contains("/ search"), "{hints}");
        assert!(!hints.contains("anchor"), "{hints}");
        assert!(hints.contains("[ ] range"), "{hints}");
    }

    #[test]
    fn no_footer_hint_key_is_left_unresolved() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        for page in [Page::Status, Page::History, Page::Configure] {
            app.page = page;
            let (hints, _) = footer_parts(&app);
            assert!(
                !hints.contains("tui.hint."),
                "unresolved hint key on {page:?}: {hints}"
            );
        }
    }

    #[test]
    fn footer_grows_to_two_rows_when_content_overflows() {
        assert_eq!(footer_rows_needed("abcdefghij", "", 100), 1);
        assert_eq!(footer_rows_needed("abcdefghij", "", 5), 2);
    }

    #[test]
    fn tab_bar_click_maps_column_to_page() {
        crate::i18n::init("en-US");
        // Labels: "1 Status"(8) "2 History"(9) "3 Configure"(11), padded 1 each
        // side, 1-cell divider between: [0,11) status, [11,23) history, [23,36) settings.
        assert_eq!(tab_at(5, 0), Some(Page::Status));
        assert_eq!(tab_at(15, 0), Some(Page::History));
        assert_eq!(tab_at(30, 0), Some(Page::Configure));
        // Past the last tab, and below the tab band, hit nothing.
        assert_eq!(tab_at(100, 0), None);
        assert_eq!(tab_at(5, 5), None);
    }
}
