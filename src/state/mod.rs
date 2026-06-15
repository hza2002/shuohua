pub mod history;

use std::sync::{Arc, RwLock};

use history::{HistoryRecord, PipelineStepHistory};
use time::OffsetDateTime;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    Idle,
    Recording,
    Stopping,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSnapshot {
    pub state: DaemonState,
    pub recording_id: Option<String>,
    pub started_at: Option<OffsetDateTime>,
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub dur_ms: u64,
    pub words: u32,
    pub segments: Vec<String>,
    pub partial: String,
}

impl Default for StateSnapshot {
    fn default() -> Self {
        Self {
            state: DaemonState::Idle,
            recording_id: None,
            started_at: None,
            app_bundle_id: None,
            app_name: None,
            dur_ms: 0,
            words: 0,
            segments: Vec::new(),
            partial: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum StateEvent {
    StateChanged {
        state: DaemonState,
        recording_id: Option<String>,
        started_at: Option<OffsetDateTime>,
    },
    AppChanged {
        bundle_id: Option<String>,
        app_name: Option<String>,
    },
    StatsChanged {
        dur_ms: u64,
        words: u32,
    },
    Partial {
        recording_id: String,
        text: String,
    },
    Segment {
        recording_id: String,
        text: String,
    },
    PipelineStep {
        recording_id: String,
        step: PipelineStepHistory,
    },
    HistoryAppended {
        record: Box<HistoryRecord>,
    },
}

#[derive(Debug, Clone)]
pub struct StateStore {
    snapshot: Arc<RwLock<StateSnapshot>>,
    tx: broadcast::Sender<StateEvent>,
}

impl StateStore {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(128);
        Self {
            snapshot: Arc::new(RwLock::new(StateSnapshot::default())),
            tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<StateEvent> {
        self.tx.subscribe()
    }

    pub fn subscribe_with_snapshot(&self) -> (StateSnapshot, broadcast::Receiver<StateEvent>) {
        let snapshot = self
            .snapshot
            .read()
            .expect("state snapshot lock poisoned")
            .clone();
        let rx = self.tx.subscribe();
        (snapshot, rx)
    }

    pub fn snapshot(&self) -> StateSnapshot {
        self.snapshot
            .read()
            .expect("state snapshot lock poisoned")
            .clone()
    }

    pub fn set_recording(&self, recording_id: String, started_at: OffsetDateTime) {
        self.set_state(DaemonState::Recording, Some(recording_id), Some(started_at));
    }

    pub fn set_stopping(&self, recording_id: String) {
        let started_at = self.snapshot().started_at;
        self.set_state(DaemonState::Stopping, Some(recording_id), started_at);
    }

    pub fn set_idle(&self) {
        self.set_state(DaemonState::Idle, None, None);
    }

    pub fn set_error(&self, recording_id: Option<String>) {
        let started_at = self.snapshot().started_at;
        self.set_state(DaemonState::Error, recording_id, started_at);
    }

    pub fn app(&self, bundle_id: Option<String>, app_name: Option<String>) {
        {
            let mut snapshot = self.snapshot.write().expect("state snapshot lock poisoned");
            snapshot.app_bundle_id = bundle_id.clone();
            snapshot.app_name = app_name.clone();
        }
        let _ = self.tx.send(StateEvent::AppChanged {
            bundle_id,
            app_name,
        });
    }

    pub fn stats(&self, dur_ms: u64, words: u32) {
        {
            let mut snapshot = self.snapshot.write().expect("state snapshot lock poisoned");
            snapshot.dur_ms = dur_ms;
            snapshot.words = words;
        }
        let _ = self.tx.send(StateEvent::StatsChanged { dur_ms, words });
    }

    pub fn partial(&self, recording_id: String, text: String) {
        self.snapshot
            .write()
            .expect("state snapshot lock poisoned")
            .partial = text.clone();
        let _ = self.tx.send(StateEvent::Partial { recording_id, text });
    }

    pub fn segment(&self, recording_id: String, text: String) {
        {
            let mut snapshot = self.snapshot.write().expect("state snapshot lock poisoned");
            snapshot.segments.push(text.clone());
            snapshot.partial.clear();
            snapshot.words = crate::text_stats::compute(&snapshot.segments.join("")).words as u32;
        }
        let _ = self.tx.send(StateEvent::Segment { recording_id, text });
    }

    pub fn pipeline_step(&self, recording_id: String, step: PipelineStepHistory) {
        let _ = self
            .tx
            .send(StateEvent::PipelineStep { recording_id, step });
    }

    pub fn history_appended(&self, record: HistoryRecord) {
        let _ = self.tx.send(StateEvent::HistoryAppended {
            record: Box::new(record),
        });
    }

    fn set_state(
        &self,
        state: DaemonState,
        recording_id: Option<String>,
        started_at: Option<OffsetDateTime>,
    ) {
        {
            let mut snapshot = self.snapshot.write().expect("state snapshot lock poisoned");
            snapshot.state = state;
            snapshot.recording_id = recording_id.clone();
            snapshot.started_at = started_at;
            if matches!(state, DaemonState::Idle) {
                snapshot.partial.clear();
                snapshot.segments.clear();
                snapshot.app_bundle_id = None;
                snapshot.app_name = None;
                snapshot.dur_ms = 0;
                snapshot.words = 0;
            }
        }
        let _ = self.tx.send(StateEvent::StateChanged {
            state,
            recording_id,
            started_at,
        });
    }
}

impl Default for StateStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn store_updates_snapshot_and_broadcasts_events() {
        let store = StateStore::new();
        let mut rx = store.subscribe();

        store.set_recording("01HXYZ".to_string(), datetime!(2026-06-13 12:00:00 UTC));
        store.app(
            Some("com.apple.dt.Xcode".to_string()),
            Some("Xcode".to_string()),
        );
        store.stats(3000, 1);
        store.segment("01HXYZ".to_string(), "he".to_string());
        store.partial("01HXYZ".to_string(), "hello".to_string());

        let snapshot = store.snapshot();
        assert_eq!(snapshot.state, DaemonState::Recording);
        assert_eq!(snapshot.recording_id.as_deref(), Some("01HXYZ"));
        assert_eq!(snapshot.app_name.as_deref(), Some("Xcode"));
        assert_eq!(snapshot.dur_ms, 3000);
        assert_eq!(snapshot.segments, vec!["he"]);
        assert_eq!(snapshot.partial, "hello");

        match rx.try_recv().unwrap() {
            StateEvent::StateChanged {
                state,
                recording_id,
                ..
            } => {
                assert_eq!(state, DaemonState::Recording);
                assert_eq!(recording_id.as_deref(), Some("01HXYZ"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
        match rx.try_recv().unwrap() {
            StateEvent::AppChanged {
                bundle_id,
                app_name,
            } => {
                assert_eq!(bundle_id.as_deref(), Some("com.apple.dt.Xcode"));
                assert_eq!(app_name.as_deref(), Some("Xcode"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
        match rx.try_recv().unwrap() {
            StateEvent::StatsChanged { dur_ms, words } => {
                assert_eq!(dur_ms, 3000);
                assert_eq!(words, 1);
            }
            other => panic!("unexpected event: {other:?}"),
        }
        match rx.try_recv().unwrap() {
            StateEvent::Segment { recording_id, text } => {
                assert_eq!(recording_id, "01HXYZ");
                assert_eq!(text, "he");
            }
            other => panic!("unexpected event: {other:?}"),
        }
        match rx.try_recv().unwrap() {
            StateEvent::Partial { recording_id, text } => {
                assert_eq!(recording_id, "01HXYZ");
                assert_eq!(text, "hello");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn subscribe_with_snapshot_returns_current_state_and_future_events() {
        let store = StateStore::new();
        store.set_recording("01HXYZ".to_string(), datetime!(2026-06-13 12:00:00 UTC));
        store.partial("01HXYZ".to_string(), "hello".to_string());

        let (snapshot, mut rx) = store.subscribe_with_snapshot();
        assert_eq!(snapshot.recording_id.as_deref(), Some("01HXYZ"));
        assert_eq!(snapshot.partial, "hello");

        store.segment("01HXYZ".to_string(), "hello".to_string());
        assert!(matches!(rx.try_recv().unwrap(), StateEvent::Segment { .. }));
    }
}
