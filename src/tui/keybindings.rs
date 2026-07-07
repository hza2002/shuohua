use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::tui::Page;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    NextPage,
    PrevPage,
    SetPage(Page),
    StartSearch,
    Forward(KeyEvent),
    None,
}

pub fn action_for(key: KeyEvent, searching: bool, page: Page) -> Action {
    if key.kind != KeyEventKind::Press {
        return Action::None;
    }
    if searching {
        return Action::Forward(key);
    }
    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Tab => Action::NextPage,
        KeyCode::BackTab => Action::PrevPage,
        KeyCode::Char('1') => Action::SetPage(Page::Status),
        KeyCode::Char('2') => Action::SetPage(Page::History),
        KeyCode::Char('3') => Action::SetPage(Page::Configure),
        // `/` only starts search on the History page; elsewhere it is a page key.
        KeyCode::Char('/') if page == Page::History => Action::StartSearch,
        _ => Action::Forward(key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn global_keys_match_when_not_searching() {
        let p = Page::History;
        assert_eq!(
            action_for(press(KeyCode::Char('q')), false, p),
            Action::Quit
        );
        assert_eq!(action_for(press(KeyCode::Tab), false, p), Action::NextPage);
        assert_eq!(
            action_for(press(KeyCode::BackTab), false, p),
            Action::PrevPage
        );
        assert_eq!(
            action_for(press(KeyCode::Char('1')), false, p),
            Action::SetPage(Page::Status)
        );
    }

    #[test]
    fn slash_starts_search_only_on_history_page() {
        assert_eq!(
            action_for(press(KeyCode::Char('/')), false, Page::History),
            Action::StartSearch
        );
        let key = press(KeyCode::Char('/'));
        assert_eq!(
            action_for(key, false, Page::Status),
            Action::Forward(key),
            "`/` on Status must not jump to History search"
        );
        assert_eq!(
            action_for(key, false, Page::Configure),
            Action::Forward(key)
        );
    }

    #[test]
    fn page_keys_forward_when_not_searching() {
        let p = Page::History;
        let key = press(KeyCode::Char('o'));
        assert_eq!(action_for(key, false, p), Action::Forward(key));
        let key = press(KeyCode::Char('j'));
        assert_eq!(action_for(key, false, p), Action::Forward(key));
    }

    #[test]
    fn all_keys_forward_when_searching() {
        let p = Page::History;
        let key = press(KeyCode::Char('q'));
        assert_eq!(action_for(key, true, p), Action::Forward(key));
        let key = press(KeyCode::Tab);
        assert_eq!(action_for(key, true, p), Action::Forward(key));
        let key = press(KeyCode::Char('/'));
        assert_eq!(action_for(key, true, p), Action::Forward(key));
    }
}
