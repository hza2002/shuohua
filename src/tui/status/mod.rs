use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::config::theme::TuiTheme;
use crate::ipc::protocol::{Event, WireState};
use crate::state::{AudioMeter, SessionMeta, SessionPhase};
use crate::tui::page::{KeyOutcome, Page};
use crate::tui::status::render::render_status;

mod render;

#[cfg(test)]
mod tests;

pub const MAX_METER_HISTORY: usize = 1024;

#[derive(Debug)]
pub struct StatusPage {
    pub state: WireState,
    pub recording_id: Option<String>,
    pub started_at: Option<time::OffsetDateTime>,
    pub app: Option<String>,
    pub app_name: Option<String>,
    pub dur_ms: u64,
    pub words: u32,
    pub segments: Vec<String>,
    pub partial: String,
    pub pipeline: Vec<String>,
    pub session_meta: Option<SessionMeta>,
    pub session_phase: Option<SessionPhase>,
    pub meters: Vec<AudioMeter>,
    pub meter_width: usize,
}

impl StatusPage {
    pub fn new() -> Self {
        Self {
            state: WireState::Idle,
            recording_id: None,
            started_at: None,
            app: None,
            app_name: None,
            dur_ms: 0,
            words: 0,
            segments: Vec::new(),
            partial: String::new(),
            pipeline: Vec::new(),
            session_meta: None,
            session_phase: None,
            meters: Vec::new(),
            meter_width: 160,
        }
    }

    pub fn current_elapsed_ms(&self) -> u64 {
        if matches!(self.state, WireState::Recording | WireState::Stopping) {
            if let Some(started_at) = self.started_at {
                if let Ok(duration) = (time::OffsetDateTime::now_utc() - started_at).try_into() {
                    let duration: std::time::Duration = duration;
                    return duration.as_millis() as u64;
                }
            }
        }
        self.dur_ms
    }

    pub fn meter_capacity_for_terminal_width(width: u16) -> usize {
        (width.saturating_sub(11).max(16) as usize).min(MAX_METER_HISTORY)
    }

    fn trim_meters_to_capacity(&mut self) {
        if self.meters.len() > MAX_METER_HISTORY {
            self.meters.drain(..self.meters.len() - MAX_METER_HISTORY);
        }
    }
}

impl Page for StatusPage {
    fn apply_event(&mut self, event: &Event, active: bool) {
        match event {
            Event::Snapshot {
                state,
                recording,
                started_at,
                app,
                app_name,
                dur_ms,
                words,
                segments,
                partial,
                ..
            } => {
                self.state = *state;
                self.recording_id = recording.clone();
                self.started_at = parse_time(started_at.as_deref());
                self.app = app.clone();
                self.app_name = app_name.clone();
                self.dur_ms = *dur_ms;
                self.words = *words;
                self.segments = segments.clone();
                self.partial = partial.clone();
            }
            Event::StateChanged {
                state,
                recording_id,
                started_at,
            } => {
                self.state = *state;
                self.recording_id = recording_id.clone();
                self.started_at = parse_time(started_at.as_deref());
                if *state == WireState::Idle {
                    self.segments.clear();
                    self.partial.clear();
                    self.pipeline.clear();
                    self.session_meta = None;
                    self.session_phase = None;
                    self.meters.clear();
                    self.app = None;
                    self.app_name = None;
                    self.dur_ms = 0;
                    self.words = 0;
                }
            }
            Event::AppChanged { app, app_name } => {
                self.app = app.clone();
                self.app_name = app_name.clone();
            }
            Event::StatsChanged { dur_ms, words } => {
                self.dur_ms = *dur_ms;
                self.words = *words;
            }
            Event::Partial { recording_id, text } if self.matches_recording(recording_id) => {
                self.partial = text.clone()
            }
            Event::Segment { recording_id, text } if self.matches_recording(recording_id) => {
                self.segments.push(text.clone());
                self.partial.clear();
            }
            Event::PipelineStep {
                name,
                status,
                duration_ms,
                text,
                error,
                recording_id,
            } if self.matches_recording(recording_id) => {
                let detail = text.clone().or_else(|| error.clone()).unwrap_or_default();
                self.pipeline
                    .push(format!("{name} {status} {duration_ms:.1}ms  {detail}"));
            }
            Event::AudioMeter {
                recording_id,
                meter,
            } if self.matches_recording(recording_id) => {
                if active {
                    self.meters.push(*meter);
                    self.trim_meters_to_capacity();
                }
            }
            Event::SessionMeta { recording_id, meta } if self.matches_recording(recording_id) => {
                self.session_meta = Some(meta.clone());
            }
            Event::SessionPhase {
                recording_id,
                phase,
            } if self.matches_recording(recording_id) => {
                self.session_phase = Some(*phase);
            }
            _ => {}
        }
    }

    fn on_key(&mut self, _key: KeyEvent) -> KeyOutcome {
        KeyOutcome::none()
    }

    fn on_enter(&mut self) {
        self.meters.clear();
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &TuiTheme, _footer_status: &str) {
        render_status(frame, self, area, theme);
    }
}

impl StatusPage {
    fn matches_recording(&self, recording_id: &str) -> bool {
        self.recording_id.as_deref() == Some(recording_id)
    }
}

fn parse_time(value: Option<&str>) -> Option<time::OffsetDateTime> {
    value.and_then(|value| {
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
    })
}
