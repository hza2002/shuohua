//! Shared daemon client API for TUI and the future GUI backend.
//!
//! This layer intentionally stays thin: it owns no wire protocol and only
//! exposes existing IPC commands/events through a stable client boundary.

use crate::ipc::protocol::{Command, Event};

pub type DaemonClient = crate::ipc::client::IpcClient;

pub fn subscribe_command() -> Command {
    Command::Subscribe
}

pub fn first_screen_commands(history_limit: usize) -> Vec<Command> {
    vec![
        subscribe_command(),
        Command::DaemonStatus,
        Command::GetHistory {
            limit: history_limit,
            before: None,
            before_id: None,
            query: None,
        },
        Command::GetHistoryStats,
    ]
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FirstScreenEvent<'a> {
    Snapshot(&'a Event),
    DaemonStatus(&'a Event),
    HistoryPage(&'a Event),
    HistoryStats(&'a Event),
    HistoryChanged,
    RecoverableError(&'a Event),
}

pub fn classify_first_screen_event(event: &Event) -> Option<FirstScreenEvent<'_>> {
    match event {
        Event::Snapshot { .. } => Some(FirstScreenEvent::Snapshot(event)),
        Event::DaemonStatus { .. } => Some(FirstScreenEvent::DaemonStatus(event)),
        Event::History { .. } => Some(FirstScreenEvent::HistoryPage(event)),
        Event::HistoryStats { .. } => Some(FirstScreenEvent::HistoryStats(event)),
        Event::HistoryChanged => Some(FirstScreenEvent::HistoryChanged),
        Event::Error { .. } => Some(FirstScreenEvent::RecoverableError(event)),
        _ => None,
    }
}

pub const DEFAULT_RECONNECT_DELAYS_MS: &[u64] = &[250, 500, 1_000, 2_000, 5_000];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting { attempt: u32, next_delay_ms: u64 },
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonConnectionProblemKind {
    ConnectFailed,
    EventStreamClosed,
    ReadFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonConnectionProblem {
    pub kind: DaemonConnectionProblemKind,
    pub message: String,
    pub recoverable: bool,
}

pub fn next_reconnect_delay_ms(attempt_index: usize) -> u64 {
    DEFAULT_RECONNECT_DELAYS_MS
        .get(attempt_index)
        .copied()
        .unwrap_or_else(|| {
            *DEFAULT_RECONNECT_DELAYS_MS
                .last()
                .expect("reconnect delays must not be empty")
        })
}

pub fn reconnecting_state(attempt_index: usize) -> DaemonConnectionState {
    DaemonConnectionState::Reconnecting {
        attempt: u32::try_from(attempt_index.saturating_add(1)).unwrap_or(u32::MAX),
        next_delay_ms: next_reconnect_delay_ms(attempt_index),
    }
}

pub fn daemon_connect_failed_problem(message: impl Into<String>) -> DaemonConnectionProblem {
    DaemonConnectionProblem {
        kind: DaemonConnectionProblemKind::ConnectFailed,
        message: message.into(),
        recoverable: true,
    }
}

pub fn daemon_event_stream_closed_problem() -> DaemonConnectionProblem {
    DaemonConnectionProblem {
        kind: DaemonConnectionProblemKind::EventStreamClosed,
        message: "daemon event stream closed".to_string(),
        recoverable: true,
    }
}

pub fn daemon_read_failed_problem(message: impl Into<String>) -> DaemonConnectionProblem {
    DaemonConnectionProblem {
        kind: DaemonConnectionProblemKind::ReadFailed,
        message: message.into(),
        recoverable: true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GuiBackendEvent<'a> {
    Daemon(FirstScreenEvent<'a>),
    ConnectionState(&'a DaemonConnectionState),
    ConnectionProblem(&'a DaemonConnectionProblem),
}

pub fn gui_backend_event_from_daemon_event(event: &Event) -> Option<GuiBackendEvent<'_>> {
    classify_first_screen_event(event).map(GuiBackendEvent::Daemon)
}

pub fn gui_backend_event_from_connection_state(
    state: &DaemonConnectionState,
) -> GuiBackendEvent<'_> {
    GuiBackendEvent::ConnectionState(state)
}

pub fn gui_backend_event_from_connection_problem(
    problem: &DaemonConnectionProblem,
) -> GuiBackendEvent<'_> {
    GuiBackendEvent::ConnectionProblem(problem)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FirstScreenReadiness {
    pub daemon_status: bool,
    pub history_page: bool,
    pub history_stats: bool,
}

impl FirstScreenReadiness {
    pub fn record_event(&mut self, event: FirstScreenEvent<'_>) {
        match event {
            FirstScreenEvent::DaemonStatus(_) => self.daemon_status = true,
            FirstScreenEvent::HistoryPage(_) => self.history_page = true,
            FirstScreenEvent::HistoryStats(_) => self.history_stats = true,
            FirstScreenEvent::Snapshot(_)
            | FirstScreenEvent::HistoryChanged
            | FirstScreenEvent::RecoverableError(_) => {}
        }
    }

    pub fn is_ready(&self) -> bool {
        self.daemon_status && self.history_page && self.history_stats
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstScreenTimingMarks {
    pub gui_started_ms: u64,
    pub daemon_connected_ms: Option<u64>,
    pub first_daemon_event_ms: Option<u64>,
    pub first_screen_ready_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstScreenTiming {
    pub connect_ms: Option<u64>,
    pub first_daemon_event_ms: Option<u64>,
    pub first_screen_ready_ms: Option<u64>,
}

impl FirstScreenTiming {
    pub fn from_marks(marks: FirstScreenTimingMarks) -> Self {
        Self {
            connect_ms: elapsed_since_start(marks.gui_started_ms, marks.daemon_connected_ms),
            first_daemon_event_ms: elapsed_since_start(
                marks.gui_started_ms,
                marks.first_daemon_event_ms,
            ),
            first_screen_ready_ms: elapsed_since_start(
                marks.gui_started_ms,
                marks.first_screen_ready_ms,
            ),
        }
    }
}

fn elapsed_since_start(start_ms: u64, mark_ms: Option<u64>) -> Option<u64> {
    mark_ms.map(|mark_ms| mark_ms.saturating_sub(start_ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{AggregateStats, HistoryStatsSnapshot, HistoryStatsStatus};
    use crate::ipc::protocol::{Event, WireState, PROTO_VERSION};

    #[test]
    fn shared_client_boundary_uses_existing_ipc_protocol() {
        assert_eq!(PROTO_VERSION, 2);
        assert_eq!(subscribe_command(), Command::Subscribe);
    }

    #[test]
    fn first_screen_commands_map_to_existing_ipc_protocol() {
        assert_eq!(
            first_screen_commands(20),
            vec![
                Command::Subscribe,
                Command::DaemonStatus,
                Command::GetHistory {
                    limit: 20,
                    before: None,
                    before_id: None,
                    query: None,
                },
                Command::GetHistoryStats,
            ]
        );
    }

    #[test]
    fn first_screen_event_classifier_preserves_existing_events() {
        let daemon = Event::DaemonStatus {
            pid: 42,
            uptime_ms: 1000,
            state: WireState::Idle,
            recording_id: None,
        };
        assert_eq!(
            classify_first_screen_event(&daemon),
            Some(FirstScreenEvent::DaemonStatus(&daemon))
        );

        let history = Event::History {
            records: Vec::new(),
            matched: Some(0),
            stats: None,
        };
        assert_eq!(
            classify_first_screen_event(&history),
            Some(FirstScreenEvent::HistoryPage(&history))
        );

        let error = Event::Error {
            recording_id: None,
            kind: "daemon_offline".to_string(),
            msg: "not running".to_string(),
        };
        assert_eq!(
            classify_first_screen_event(&error),
            Some(FirstScreenEvent::RecoverableError(&error))
        );

        assert_eq!(
            classify_first_screen_event(&Event::HistoryChanged),
            Some(FirstScreenEvent::HistoryChanged)
        );
    }

    #[test]
    fn daemon_connection_state_models_bounded_reconnect_without_protocol_changes() {
        assert_eq!(PROTO_VERSION, 2);
        assert_eq!(
            DEFAULT_RECONNECT_DELAYS_MS,
            &[250, 500, 1_000, 2_000, 5_000]
        );
        assert_eq!(next_reconnect_delay_ms(0), DEFAULT_RECONNECT_DELAYS_MS[0]);
        assert_eq!(
            next_reconnect_delay_ms(99),
            *DEFAULT_RECONNECT_DELAYS_MS.last().unwrap()
        );
        assert_eq!(
            reconnecting_state(2),
            DaemonConnectionState::Reconnecting {
                attempt: 3,
                next_delay_ms: 1_000,
            }
        );
        assert_eq!(
            reconnecting_state(usize::MAX),
            DaemonConnectionState::Reconnecting {
                attempt: u32::MAX,
                next_delay_ms: 5_000,
            }
        );

        let offline = daemon_connect_failed_problem("connect IPC /tmp/shuohua.sock");
        assert_eq!(offline.kind, DaemonConnectionProblemKind::ConnectFailed);
        assert!(offline.recoverable);

        let closed = daemon_event_stream_closed_problem();
        assert_eq!(closed.kind, DaemonConnectionProblemKind::EventStreamClosed);
        assert!(closed.recoverable);
    }

    #[test]
    fn gui_backend_event_bridge_wraps_existing_client_api_shapes() {
        assert_eq!(PROTO_VERSION, 2);

        let daemon = Event::DaemonStatus {
            pid: 42,
            uptime_ms: 1000,
            state: WireState::Idle,
            recording_id: None,
        };
        assert_eq!(
            gui_backend_event_from_daemon_event(&daemon),
            Some(GuiBackendEvent::Daemon(FirstScreenEvent::DaemonStatus(
                &daemon
            )))
        );

        let ignored = Event::ConfigReloaded {
            path: "/tmp/config.toml".to_string(),
        };
        assert_eq!(gui_backend_event_from_daemon_event(&ignored), None);

        let state = DaemonConnectionState::Connected;
        assert_eq!(
            gui_backend_event_from_connection_state(&state),
            GuiBackendEvent::ConnectionState(&state)
        );

        let problem = daemon_read_failed_problem("read IPC event");
        assert_eq!(
            gui_backend_event_from_connection_problem(&problem),
            GuiBackendEvent::ConnectionProblem(&problem)
        );
    }

    #[test]
    fn first_screen_timing_models_readiness_without_runtime_or_protocol_changes() {
        assert_eq!(PROTO_VERSION, 2);

        let mut readiness = FirstScreenReadiness::default();
        let status = Event::DaemonStatus {
            pid: 42,
            uptime_ms: 1000,
            state: WireState::Idle,
            recording_id: None,
        };
        readiness.record_event(FirstScreenEvent::DaemonStatus(&status));
        assert!(!readiness.is_ready());

        let history = Event::History {
            records: Vec::new(),
            matched: Some(0),
            stats: None,
        };
        readiness.record_event(FirstScreenEvent::HistoryPage(&history));
        assert!(!readiness.is_ready());

        let stats = Event::HistoryStats {
            snapshot: HistoryStatsSnapshot {
                status: HistoryStatsStatus::Ready,
                total: AggregateStats::default(),
                current_month: AggregateStats::default(),
                today: AggregateStats::default(),
                error: None,
            },
        };
        readiness.record_event(FirstScreenEvent::HistoryStats(&stats));
        assert!(readiness.is_ready());

        let timing = FirstScreenTiming::from_marks(FirstScreenTimingMarks {
            gui_started_ms: 100,
            daemon_connected_ms: Some(250),
            first_daemon_event_ms: Some(275),
            first_screen_ready_ms: Some(475),
        });
        assert_eq!(timing.connect_ms, Some(150));
        assert_eq!(timing.first_daemon_event_ms, Some(175));
        assert_eq!(timing.first_screen_ready_ms, Some(375));

        let saturated = FirstScreenTiming::from_marks(FirstScreenTimingMarks {
            gui_started_ms: 500,
            daemon_connected_ms: Some(250),
            first_daemon_event_ms: Some(275),
            first_screen_ready_ms: Some(475),
        });
        assert_eq!(saturated.connect_ms, Some(0));
        assert_eq!(saturated.first_daemon_event_ms, Some(0));
        assert_eq!(saturated.first_screen_ready_ms, Some(0));
    }
}
