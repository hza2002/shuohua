pub mod history;

use std::sync::{Arc, Mutex};

use history::{HistoryRecord, PipelineStepHistory};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AudioMeter {
    pub rms: f32,
    pub peak: f32,
    pub clipped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vad_probability: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vad_speech: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub provider: String,
    pub chain: String,
    pub vad: String,
    pub hotwords: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    Active,
    Idle,
    Stopping,
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
    AudioMeter {
        recording_id: String,
        meter: AudioMeter,
    },
    SessionMeta {
        recording_id: String,
        meta: SessionMeta,
    },
    SessionPhase {
        recording_id: String,
        phase: SessionPhase,
    },
    Error {
        recording_id: Option<String>,
        kind: String,
        msg: String,
    },
    HistoryAppended {
        record: Box<HistoryRecord>,
    },
}

#[derive(Clone)]
pub struct StateStore {
    inner: Arc<Mutex<StateInner>>,
}

struct StateInner {
    snapshot: StateSnapshot,
    tx: broadcast::Sender<StateEvent>,
    #[cfg(test)]
    before_subscribe: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl StateStore {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(128);
        Self {
            inner: Arc::new(Mutex::new(StateInner {
                snapshot: StateSnapshot::default(),
                tx,
                #[cfg(test)]
                before_subscribe: None,
            })),
        }
    }

    pub fn subscribe_with_snapshot(&self) -> (StateSnapshot, broadcast::Receiver<StateEvent>) {
        let inner = self.inner.lock().expect("state lock poisoned");
        #[cfg(test)]
        if let Some(hook) = &inner.before_subscribe {
            hook();
        }
        let rx = inner.tx.subscribe();
        let snapshot = inner.snapshot.clone();
        (snapshot, rx)
    }

    pub fn snapshot(&self) -> StateSnapshot {
        self.inner
            .lock()
            .expect("state lock poisoned")
            .snapshot
            .clone()
    }

    pub fn set_recording(&self, recording_id: String, started_at: OffsetDateTime) {
        self.set_state(DaemonState::Recording, Some(recording_id), Some(started_at));
    }

    pub fn set_stopping(&self, recording_id: String) {
        self.set_state_preserving_started_at(DaemonState::Stopping, Some(recording_id));
    }

    pub fn set_idle(&self) {
        self.set_state(DaemonState::Idle, None, None);
    }

    pub fn set_error(&self, recording_id: Option<String>) {
        self.set_state_preserving_started_at(DaemonState::Error, recording_id);
    }

    pub fn app(&self, bundle_id: Option<String>, app_name: Option<String>) {
        let mut inner = self.inner.lock().expect("state lock poisoned");
        inner.snapshot.app_bundle_id = bundle_id.clone();
        inner.snapshot.app_name = app_name.clone();
        let _ = inner.tx.send(StateEvent::AppChanged {
            bundle_id,
            app_name,
        });
    }

    pub fn stats(&self, recording_id: String, dur_ms: u64, words: u32) {
        let mut inner = self.inner.lock().expect("state lock poisoned");
        if !current_recording_matches(&inner.snapshot, &recording_id) {
            tracing::debug!(
                recording_id,
                current = inner.snapshot.recording_id.as_deref(),
                "drop stale state stats"
            );
            return;
        }
        inner.snapshot.dur_ms = dur_ms;
        inner.snapshot.words = words;
        let _ = inner.tx.send(StateEvent::StatsChanged { dur_ms, words });
    }

    pub fn partial(&self, recording_id: String, text: String) {
        let mut inner = self.inner.lock().expect("state lock poisoned");
        if !current_recording_matches(&inner.snapshot, &recording_id) {
            tracing::debug!(
                recording_id,
                current = inner.snapshot.recording_id.as_deref(),
                "drop stale state partial"
            );
            return;
        }
        inner.snapshot.partial = text.clone();
        let _ = inner.tx.send(StateEvent::Partial { recording_id, text });
    }

    pub fn segment(&self, recording_id: String, text: String) {
        let mut inner = self.inner.lock().expect("state lock poisoned");
        if !current_recording_matches(&inner.snapshot, &recording_id) {
            tracing::debug!(
                recording_id,
                current = inner.snapshot.recording_id.as_deref(),
                "drop stale state segment"
            );
            return;
        }
        inner.snapshot.segments.push(text.clone());
        inner.snapshot.partial.clear();
        inner.snapshot.words =
            crate::text_stats::compute(&inner.snapshot.segments.join("")).words as u32;
        let _ = inner.tx.send(StateEvent::Segment { recording_id, text });
    }

    pub fn pipeline_step(&self, recording_id: String, step: PipelineStepHistory) {
        let inner = self.inner.lock().expect("state lock poisoned");
        let _ = inner
            .tx
            .send(StateEvent::PipelineStep { recording_id, step });
    }

    pub fn audio_meter(&self, recording_id: String, meter: AudioMeter) {
        let inner = self.inner.lock().expect("state lock poisoned");
        let _ = inner.tx.send(StateEvent::AudioMeter {
            recording_id,
            meter,
        });
    }

    pub fn session_meta(&self, recording_id: String, meta: SessionMeta) {
        let inner = self.inner.lock().expect("state lock poisoned");
        let _ = inner
            .tx
            .send(StateEvent::SessionMeta { recording_id, meta });
    }

    pub fn session_phase(&self, recording_id: String, phase: SessionPhase) {
        let inner = self.inner.lock().expect("state lock poisoned");
        let _ = inner.tx.send(StateEvent::SessionPhase {
            recording_id,
            phase,
        });
    }

    pub fn error(
        &self,
        recording_id: Option<String>,
        kind: impl Into<String>,
        msg: impl Into<String>,
    ) {
        let inner = self.inner.lock().expect("state lock poisoned");
        let _ = inner.tx.send(StateEvent::Error {
            recording_id,
            kind: kind.into(),
            msg: msg.into(),
        });
    }

    pub fn history_appended(&self, record: HistoryRecord) {
        let inner = self.inner.lock().expect("state lock poisoned");
        let _ = inner.tx.send(StateEvent::HistoryAppended {
            record: Box::new(record),
        });
    }

    #[cfg(test)]
    fn set_before_subscribe_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        self.inner
            .lock()
            .expect("state lock poisoned")
            .before_subscribe = Some(hook);
    }

    fn set_state(
        &self,
        state: DaemonState,
        recording_id: Option<String>,
        started_at: Option<OffsetDateTime>,
    ) {
        let mut inner = self.inner.lock().expect("state lock poisoned");
        inner.snapshot.state = state;
        inner.snapshot.recording_id = recording_id.clone();
        inner.snapshot.started_at = started_at;
        if matches!(state, DaemonState::Idle) {
            inner.snapshot.partial.clear();
            inner.snapshot.segments.clear();
            inner.snapshot.app_bundle_id = None;
            inner.snapshot.app_name = None;
            inner.snapshot.dur_ms = 0;
            inner.snapshot.words = 0;
        }
        let _ = inner.tx.send(StateEvent::StateChanged {
            state,
            recording_id,
            started_at,
        });
    }

    fn set_state_preserving_started_at(&self, state: DaemonState, recording_id: Option<String>) {
        let mut inner = self.inner.lock().expect("state lock poisoned");
        let started_at = inner.snapshot.started_at;
        inner.snapshot.state = state;
        inner.snapshot.recording_id = recording_id.clone();
        inner.snapshot.started_at = started_at;
        let _ = inner.tx.send(StateEvent::StateChanged {
            state,
            recording_id,
            started_at,
        });
    }
}

fn current_recording_matches(snapshot: &StateSnapshot, recording_id: &str) -> bool {
    snapshot.recording_id.as_deref() == Some(recording_id)
}

impl Default for StateStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::Duration;
    use time::macros::datetime;

    #[test]
    fn store_updates_snapshot_and_broadcasts_events() {
        let store = StateStore::new();
        let (_, mut rx) = store.subscribe_with_snapshot();

        store.set_recording("01HXYZ".to_string(), datetime!(2026-06-13 12:00:00 UTC));
        store.app(
            Some("com.apple.dt.Xcode".to_string()),
            Some("Xcode".to_string()),
        );
        store.stats("01HXYZ".to_string(), 3000, 1);
        store.segment("01HXYZ".to_string(), "he".to_string());
        store.partial("01HXYZ".to_string(), "hello".to_string());
        store.audio_meter(
            "01HXYZ".to_string(),
            AudioMeter {
                rms: 0.25,
                peak: 0.75,
                clipped: false,
                vad_probability: Some(0.8),
                vad_speech: Some(true),
            },
        );

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
        match rx.try_recv().unwrap() {
            StateEvent::AudioMeter {
                recording_id,
                meter,
            } => {
                assert_eq!(recording_id, "01HXYZ");
                assert_eq!(meter.vad_speech, Some(true));
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

    #[test]
    fn subscribe_with_snapshot_does_not_miss_concurrent_updates() {
        let store = StateStore::new();
        store.set_recording("01HXYZ".to_string(), datetime!(2026-06-13 12:00:00 UTC));

        let (hook_entered_tx, hook_entered_rx) = mpsc::channel();
        let (release_hook_tx, release_hook_rx) = mpsc::channel();
        let release_hook_rx = Arc::new(Mutex::new(release_hook_rx));
        store.set_before_subscribe_hook(Arc::new(move || {
            hook_entered_tx.send(()).unwrap();
            release_hook_rx.lock().unwrap().recv().unwrap();
        }));

        let subscriber_store = store.clone();
        let subscriber = std::thread::spawn(move || subscriber_store.subscribe_with_snapshot());

        hook_entered_rx.recv().unwrap();
        let writer_store = store.clone();
        let writer = std::thread::spawn(move || {
            writer_store.partial("01HXYZ".to_string(), "concurrent".to_string());
        });
        std::thread::sleep(Duration::from_millis(20));
        release_hook_tx.send(()).unwrap();

        let (snapshot, mut rx) = subscriber.join().unwrap();
        writer.join().unwrap();

        if snapshot.partial != "concurrent" {
            assert!(matches!(
                rx.try_recv().unwrap(),
                StateEvent::Partial { text, .. } if text == "concurrent"
            ));
        }
    }

    #[test]
    fn error_broadcast_does_not_change_daemon_state() {
        let store = StateStore::new();
        let (_, mut rx) = store.subscribe_with_snapshot();

        store.error(Some("01HXYZ".to_string()), "history_append", "disk full");

        assert_eq!(store.snapshot().state, DaemonState::Idle);
        match rx.try_recv().unwrap() {
            StateEvent::Error {
                recording_id,
                kind,
                msg,
            } => {
                assert_eq!(recording_id.as_deref(), Some("01HXYZ"));
                assert_eq!(kind, "history_append");
                assert_eq!(msg, "disk full");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn stale_recording_updates_do_not_mutate_snapshot_or_broadcast() {
        let store = StateStore::new();
        let (_, mut rx) = store.subscribe_with_snapshot();
        store.set_recording("new".to_string(), datetime!(2026-06-13 12:00:00 UTC));
        let _ = rx.try_recv().unwrap();

        store.partial("old".to_string(), "stale partial".to_string());
        store.segment("old".to_string(), "stale segment".to_string());
        store.stats("old".to_string(), 9000, 42);

        let snapshot = store.snapshot();
        assert_eq!(snapshot.recording_id.as_deref(), Some("new"));
        assert_eq!(snapshot.partial, "");
        assert!(snapshot.segments.is_empty());
        assert_eq!(snapshot.dur_ms, 0);
        assert_eq!(snapshot.words, 0);
        assert!(rx.try_recv().is_err());
    }
}
