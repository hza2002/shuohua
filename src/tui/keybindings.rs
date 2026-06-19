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

pub fn action_for(key: KeyEvent, searching: bool) -> Action {
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
        KeyCode::Char('3') => Action::SetPage(Page::Settings),
        KeyCode::Char('/') => Action::StartSearch,
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
        assert_eq!(action_for(press(KeyCode::Char('q')), false), Action::Quit);
        assert_eq!(action_for(press(KeyCode::Tab), false), Action::NextPage);
        assert_eq!(action_for(press(KeyCode::BackTab), false), Action::PrevPage);
        assert_eq!(
            action_for(press(KeyCode::Char('1')), false),
            Action::SetPage(Page::Status)
        );
        assert_eq!(
            action_for(press(KeyCode::Char('/')), false),
            Action::StartSearch
        );
    }

    #[test]
    fn page_keys_forward_when_not_searching() {
        let key = press(KeyCode::Char('o'));
        assert_eq!(action_for(key, false), Action::Forward(key));
        let key = press(KeyCode::Char('j'));
        assert_eq!(action_for(key, false), Action::Forward(key));
    }

    #[test]
    fn all_keys_forward_when_searching() {
        let key = press(KeyCode::Char('q'));
        assert_eq!(action_for(key, true), Action::Forward(key));
        let key = press(KeyCode::Tab);
        assert_eq!(action_for(key, true), Action::Forward(key));
        let key = press(KeyCode::Char('/'));
        assert_eq!(action_for(key, true), Action::Forward(key));
    }
}
