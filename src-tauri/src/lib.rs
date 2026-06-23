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

fn first_screen_command_kind(command: &Command) -> &'static str {
    match command {
        Command::Subscribe => "subscribe",
        Command::DaemonStatus => "daemonStatus",
        Command::GetHistory { .. } => "historyPage",
        Command::GetHistoryStats => "historyStats",
        _ => "other",
    }
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            gui_shell_metadata,
            gui_first_screen_request_plan
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Shuohua GUI");
}
