pub mod history;

use std::sync::{Arc, RwLock};

use history::{HistoryRecord, PipelineStepHistory};
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
    pub partial: String,
}

impl Default for StateSnapshot {
    fn default() -> Self {
        Self {
            state: DaemonState::Idle,
            recording_id: None,
            partial: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum StateEvent {
    StateChanged {
        state: DaemonState,
        recording_id: Option<String>,
    },
    Partial {
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

    pub fn snapshot(&self) -> StateSnapshot {
        self.snapshot
            .read()
            .expect("state snapshot lock poisoned")
            .clone()
    }

    pub fn set_recording(&self, recording_id: String) {
        self.set_state(DaemonState::Recording, Some(recording_id));
    }

    pub fn set_stopping(&self, recording_id: String) {
        self.set_state(DaemonState::Stopping, Some(recording_id));
    }

    pub fn set_idle(&self) {
        self.set_state(DaemonState::Idle, None);
    }

    pub fn set_error(&self, recording_id: Option<String>) {
        self.set_state(DaemonState::Error, recording_id);
    }

    pub fn partial(&self, recording_id: String, text: String) {
        self.snapshot
            .write()
            .expect("state snapshot lock poisoned")
            .partial = text.clone();
        let _ = self.tx.send(StateEvent::Partial { recording_id, text });
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

    fn set_state(&self, state: DaemonState, recording_id: Option<String>) {
        {
            let mut snapshot = self.snapshot.write().expect("state snapshot lock poisoned");
            snapshot.state = state;
            snapshot.recording_id = recording_id.clone();
            if matches!(state, DaemonState::Idle) {
                snapshot.partial.clear();
            }
        }
        let _ = self.tx.send(StateEvent::StateChanged {
            state,
            recording_id,
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

    #[test]
    fn store_updates_snapshot_and_broadcasts_events() {
        let store = StateStore::new();
        let mut rx = store.subscribe();

        store.set_recording("01HXYZ".to_string());
        store.partial("01HXYZ".to_string(), "hello".to_string());

        let snapshot = store.snapshot();
        assert_eq!(snapshot.state, DaemonState::Recording);
        assert_eq!(snapshot.recording_id.as_deref(), Some("01HXYZ"));
        assert_eq!(snapshot.partial, "hello");

        match rx.try_recv().unwrap() {
            StateEvent::StateChanged {
                state,
                recording_id,
            } => {
                assert_eq!(state, DaemonState::Recording);
                assert_eq!(recording_id.as_deref(), Some("01HXYZ"));
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
}
