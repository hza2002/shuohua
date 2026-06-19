#![allow(dead_code)]

use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::ipc::protocol::{Command, Event};

#[derive(Debug)]
pub enum KeyOutcome {
    None,
    SetStatus(String),
    StartSearch,
    SendCommand(Command),
    Quit,
}

pub trait Page {
    fn apply_event(&mut self, event: &Event, active: bool);
    fn on_key(&mut self, key: KeyEvent) -> KeyOutcome;
    fn on_enter(&mut self) {}
    fn on_leave(&mut self) {}
    fn render(&self, frame: &mut Frame, area: Rect, theme: &TuiTheme, footer_status: &str);
}
