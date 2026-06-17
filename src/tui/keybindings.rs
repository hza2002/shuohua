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
    MoveTop,
    MoveBottom,
    NextDetail,
    PrevDetail,
    StartSearch,
    CancelSearch,
    ClearSearch,
    SearchChar(char),
    Backspace,
    CopySelected,
    CopySelectedRaw,
    OpenAudio,
    RevealAudio,
    DeleteAudio,
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
        KeyCode::Char('g') => Action::MoveTop,
        KeyCode::Char('G') => Action::MoveBottom,
        KeyCode::Char('l') | KeyCode::Right => Action::NextDetail,
        KeyCode::Char('h') | KeyCode::Left => Action::PrevDetail,
        KeyCode::Char('/') => Action::StartSearch,
        KeyCode::Esc => Action::ClearSearch,
        KeyCode::Enter | KeyCode::Char('y') => Action::CopySelected,
        KeyCode::Char('Y') => Action::CopySelectedRaw,
        KeyCode::Char('o') => Action::OpenAudio,
        KeyCode::Char('r') => Action::RevealAudio,
        KeyCode::Char('d') => Action::DeleteAudio,
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn audio_shortcuts_are_open_reveal_and_delete_only() {
        assert_eq!(
            action_for(press(KeyCode::Char('o')), false),
            Action::OpenAudio
        );
        assert_eq!(
            action_for(press(KeyCode::Char('r')), false),
            Action::RevealAudio
        );
        assert_eq!(
            action_for(press(KeyCode::Char('d')), false),
            Action::DeleteAudio
        );
        assert_eq!(action_for(press(KeyCode::Char('p')), false), Action::None);
    }
}
