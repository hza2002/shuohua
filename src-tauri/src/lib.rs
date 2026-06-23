use shuohua::ipc::protocol::Command;

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
    let request = Command::DaemonStatus;

    GuiDaemonStatusSnapshot {
        connected: false,
        transport_opened: false,
        snapshot_available: false,
        state_label: "disconnected",
        request: GuiDaemonStatusRequestSummary {
            request_kind: daemon_status_request_kind(&request),
            requires_daemon_connection: true,
        },
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
