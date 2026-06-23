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

#[cfg(test)]
mod tests {
    use super::*;
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
}
