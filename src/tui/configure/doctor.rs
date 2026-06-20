use std::process::Command as ProcessCommand;

use crate::tui::configure::DoctorState;

pub(super) fn run_doctor() -> DoctorState {
    let output = ProcessCommand::new(std::env::current_exe().unwrap_or_else(|_| "shuo".into()))
        .arg("doctor")
        .output();
    match output {
        Ok(output) => {
            let mut text = String::new();
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            if !output.stderr.is_empty() {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            DoctorState {
                ran_once: true,
                status: Some(if output.status.success() {
                    "ok".to_string()
                } else {
                    format!("exit {}", output.status)
                }),
                output: text,
            }
        }
        Err(e) => DoctorState {
            ran_once: true,
            status: Some("error".to_string()),
            output: crate::i18n::tr("tui.configure.doctor_failed", &[("error", e.to_string())]),
        },
    }
}
