use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::ipc::protocol::{Command, Event};

/// A single keybinding shown in the footer. Pages return these from
/// `key_hints()` right next to their `on_key` handler, so the footer is derived
/// from one source of truth and can never drift from what the keys actually do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyHint {
    pub keys: &'static str,
    pub label_key: &'static str,
}

impl KeyHint {
    pub const fn new(keys: &'static str, label_key: &'static str) -> Self {
        Self { keys, label_key }
    }
}

/// A mouse interaction reduced to what the pages act on: a click or a wheel
/// scroll. "Click triggers whatever is under the cursor."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Down,
    ScrollUp,
    ScrollDown,
}

#[derive(Debug, Default)]
pub struct KeyOutcome {
    pub status: Option<String>,
    pub command: Option<Command>,
}

impl KeyOutcome {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn status(msg: impl Into<String>) -> Self {
        Self {
            status: Some(msg.into()),
            command: None,
        }
    }

    pub fn command_and_status(cmd: Command, msg: impl Into<String>) -> Self {
        Self {
            status: Some(msg.into()),
            command: Some(cmd),
        }
    }
}

pub trait Page {
    fn apply_event(&mut self, event: &Event, active: bool);
    fn on_key(&mut self, key: KeyEvent) -> KeyOutcome;
    fn on_enter(&mut self) {}
    /// Context-aware footer hints for the current page state. Keep in sync with
    /// `on_key`: every key handled there should have a hint here.
    fn key_hints(&self) -> Vec<KeyHint> {
        Vec::new()
    }
    fn render(&self, frame: &mut Frame, area: Rect, theme: &TuiTheme, footer_status: &str);
}
