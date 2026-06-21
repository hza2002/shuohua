use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::tui::page::Page as _;
use crate::tui::ui;
use crate::tui::{App, Page};

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

    let page_theme: &TuiTheme = &app.theme;
    match app.page {
        Page::Status => app
            .status_page
            .render(frame, root[1], page_theme, &app.status),
        Page::History => app.history.render(frame, root[1], page_theme, &app.status),
        Page::Settings => app
            .configure
            .render(frame, root[1], page_theme, &app.status),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_only_shows_history_actions_on_history_page() {
        crate::i18n::init("en-US");
        let mut app = App::new();
        app.page = Page::Status;
        assert!(!footer_text(&app).contains("delete history"));

        app.page = Page::History;
        let footer = footer_text(&app);
        assert!(footer.contains("delete history") || footer.contains("删历史"));
        assert!(footer.contains("analytics") || footer.contains("分析"));
    }
}
