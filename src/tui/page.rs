use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::ipc::protocol::{Command, Event};

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
    fn render(&self, frame: &mut Frame, area: Rect, theme: &TuiTheme, footer_status: &str);
}
