use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::tui::Page;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    NextPage,
    PrevPage,
    SetPage(Page),
    MoveDown,
    MoveUp,
    StartSearch,
    CancelSearch,
    ClearSearch,
    SearchChar(char),
    Backspace,
    CopySelected,
    None,
}

pub fn action_for(key: KeyEvent, searching: bool) -> Action {
    if key.kind != KeyEventKind::Press {
        return Action::None;
    }
    if searching {
        return match key.code {
            KeyCode::Esc => Action::CancelSearch,
            KeyCode::Enter => Action::CancelSearch,
            KeyCode::Backspace => Action::Backspace,
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Action::ClearSearch
            }
            KeyCode::Char(ch) => Action::SearchChar(ch),
            _ => Action::None,
        };
    }
    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Tab => Action::NextPage,
        KeyCode::BackTab => Action::PrevPage,
        KeyCode::Char('1') => Action::SetPage(Page::Status),
        KeyCode::Char('2') => Action::SetPage(Page::History),
        KeyCode::Char('3') => Action::SetPage(Page::Settings),
        KeyCode::Char('j') | KeyCode::Down => Action::MoveDown,
        KeyCode::Char('k') | KeyCode::Up => Action::MoveUp,
        KeyCode::Char('/') => Action::StartSearch,
        KeyCode::Esc => Action::ClearSearch,
        KeyCode::Enter => Action::CopySelected,
        _ => Action::None,
    }
}
