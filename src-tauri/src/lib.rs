use shuohua::ipc::protocol::{Command, Event, WireState};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiShellMetadata {
    app_name: &'static str,
    phase: &'static str,
    daemon_connected: bool,
    placeholder_ready: bool,
}

#[tauri::command]
fn gui_shell_metadata() -> GuiShellMetadata {
    GuiShellMetadata {
        app_name: "Shuohua",
        phase: "Phase 9n",
        daemon_connected: false,
        placeholder_ready: true,
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenRequestPlan {
    history_limit: usize,
    requires_daemon_connection: bool,
    transport_opened: bool,
    commands: Vec<GuiFirstScreenCommandSummary>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenCommandSummary {
    kind: &'static str,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiDaemonStatusSnapshot {
    connected: bool,
    transport_opened: bool,
    snapshot_available: bool,
    state_label: &'static str,
    pid: Option<u32>,
    uptime_ms: Option<u64>,
    recording_id: Option<String>,
    request: GuiDaemonStatusRequestSummary,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiDaemonStatusRequestSummary {
    request_kind: &'static str,
    requires_daemon_connection: bool,
}

#[tauri::command]
fn gui_first_screen_request_plan(history_limit: Option<usize>) -> GuiFirstScreenRequestPlan {
    let history_limit = history_limit.unwrap_or(20);
    let commands = shuohua::client_api::first_screen_commands(history_limit)
        .into_iter()
        .map(|command| GuiFirstScreenCommandSummary {
            kind: first_screen_command_kind(&command),
        })
        .collect();

    GuiFirstScreenRequestPlan {
        history_limit,
        requires_daemon_connection: true,
        transport_opened: false,
        commands,
    }
}

#[tauri::command]
fn gui_daemon_status_snapshot() -> GuiDaemonStatusSnapshot {
    empty_daemon_status_snapshot()
}

fn empty_daemon_status_snapshot() -> GuiDaemonStatusSnapshot {
    GuiDaemonStatusSnapshot {
        connected: false,
        transport_opened: false,
        snapshot_available: false,
        state_label: "disconnected",
        pid: None,
        uptime_ms: None,
        recording_id: None,
        request: GuiDaemonStatusRequestSummary {
            request_kind: daemon_status_request_kind(&Command::DaemonStatus),
            requires_daemon_connection: true,
        },
    }
}

#[allow(dead_code)]
fn gui_daemon_status_snapshot_from_event(event: &Event) -> Option<GuiDaemonStatusSnapshot> {
    let Event::DaemonStatus {
        pid,
        uptime_ms,
        state,
        recording_id,
    } = event
    else {
        return None;
    };

    Some(GuiDaemonStatusSnapshot {
        connected: true,
        transport_opened: true,
        snapshot_available: true,
        state_label: wire_state_label(*state),
        pid: Some(*pid),
        uptime_ms: Some(*uptime_ms),
        recording_id: recording_id.clone(),
        request: GuiDaemonStatusRequestSummary {
            request_kind: daemon_status_request_kind(&Command::DaemonStatus),
            requires_daemon_connection: true,
        },
    })
}

fn wire_state_label(state: WireState) -> &'static str {
    match state {
        WireState::Idle => "idle",
        WireState::Recording => "recording",
        WireState::Stopping => "stopping",
        WireState::Error => "error",
    }
}

fn first_screen_command_kind(command: &Command) -> &'static str {
    match command {
        Command::Subscribe => "subscribe",
        Command::DaemonStatus => "daemonStatus",
        Command::GetHistory { .. } => "historyPage",
        Command::GetHistoryStats => "historyStats",
        _ => "other",
    }
}

fn daemon_status_request_kind(command: &Command) -> &'static str {
    match command {
        Command::DaemonStatus => "daemonStatus",
        _ => "other",
    }
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            gui_shell_metadata,
            gui_first_screen_request_plan,
            gui_daemon_status_snapshot
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Shuohua GUI");
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuohua::ipc::protocol::{Event, WireState};

    #[test]
    fn daemon_status_event_maps_to_snapshot_shape_without_ipc() {
        let event = Event::DaemonStatus {
            pid: 42,
            uptime_ms: 12_345,
            state: WireState::Recording,
            recording_id: Some("01STATUS".to_string()),
        };

        let snapshot = gui_daemon_status_snapshot_from_event(&event).unwrap();

        assert!(snapshot.connected);
        assert!(snapshot.transport_opened);
        assert!(snapshot.snapshot_available);
        assert_eq!(snapshot.state_label, "recording");
        assert_eq!(snapshot.pid, Some(42));
        assert_eq!(snapshot.uptime_ms, Some(12_345));
        assert_eq!(snapshot.recording_id.as_deref(), Some("01STATUS"));
        assert_eq!(snapshot.request.request_kind, "daemonStatus");
        assert!(snapshot.request.requires_daemon_connection);
        assert!(gui_daemon_status_snapshot_from_event(&Event::HistoryChanged).is_none());
    }
}
