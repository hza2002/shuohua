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

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![gui_shell_metadata])
        .run(tauri::generate_context!())
        .expect("failed to run Shuohua GUI");
}
