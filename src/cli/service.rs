use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::ipc::protocol::{Command, Event};

const LABEL: &str = "com.hza2002.shuohua";
const DAEMON_STATUS_TIMEOUT: Duration = Duration::from_secs(1);

pub fn plist_path() -> PathBuf {
    home_dir().join("Library/LaunchAgents/com.hza2002.shuohua.plist")
}

pub fn install() -> Result<()> {
    let state_dir = crate::state::history::state_dir();
    std::fs::create_dir_all(&state_dir)
        .with_context(|| tr_path("cli.service.create_state_dir_failed", &state_dir))?;
    let plist = plist_path();
    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| tr_path("cli.service.create_launch_agents_dir_failed", parent))?;
    }
    let exe =
        std::env::current_exe().context(crate::i18n::tr("cli.service.resolve_exe_failed", &[]))?;
    let body = plist_body(&exe, &state_dir);
    std::fs::write(&plist, body)
        .with_context(|| tr_path("cli.service.write_plist_failed", &plist))?;
    let _ = run_launchctl(
        &["bootout", &gui_domain(), plist.to_str().unwrap_or_default()],
        "cli.service.action_uninstall",
    );
    run_launchctl(
        &[
            "bootstrap",
            &gui_domain(),
            plist.to_str().unwrap_or_default(),
        ],
        "cli.service.action_install",
    )?;
    run_launchctl(
        &["kickstart", "-k", &format!("{}/{}", gui_domain(), LABEL)],
        "cli.service.action_start",
    )?;
    println!(
        "{}",
        crate::i18n::tr(
            "cli.service.installed",
            &[("path", plist.display().to_string())]
        )
    );
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let plist = plist_path();
    let _ = run_launchctl(
        &["bootout", &gui_domain(), plist.to_str().unwrap_or_default()],
        "cli.service.action_uninstall",
    );
    if plist.exists() {
        std::fs::remove_file(&plist)
            .with_context(|| tr_path("cli.service.remove_plist_failed", &plist))?;
    }
    println!(
        "{}",
        crate::i18n::tr(
            "cli.service.uninstalled",
            &[("path", plist.display().to_string())]
        )
    );
    Ok(())
}

pub fn start() -> Result<()> {
    run_launchctl(
        &["kickstart", "-k", &format!("{}/{}", gui_domain(), LABEL)],
        "cli.service.action_start",
    )?;
    println!(
        "{}",
        crate::i18n::tr("cli.service.started", &[("label", LABEL.to_string())])
    );
    Ok(())
}

pub fn stop() -> Result<()> {
    request_daemon_shutdown()?;
    println!(
        "{}",
        crate::i18n::tr("cli.service.stopped", &[("label", LABEL.to_string())])
    );
    Ok(())
}

fn request_daemon_shutdown() -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create stop runtime")?;
    rt.block_on(tokio::time::timeout(DAEMON_STATUS_TIMEOUT, async {
        let mut client =
            crate::ipc::client::IpcClient::connect(crate::ipc::server::default_socket_path())
                .await?;
        client.send(&Command::Shutdown).await?;
        let _ = client.recv().await?;
        Ok::<(), anyhow::Error>(())
    }))
    .context("shutdown IPC timed out")?
}

pub fn restart() -> Result<()> {
    let _ = stop();
    start()
}

pub fn status() -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create status runtime")?;
    match rt.block_on(tokio::time::timeout(DAEMON_STATUS_TIMEOUT, uds_status())) {
        Ok(Ok(Some(line))) => {
            println!("{line}");
            return Ok(());
        }
        Ok(Ok(None)) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => anyhow::bail!(
            "{}",
            crate::i18n::tr(
                "cli.service.status_timeout",
                &[("seconds", DAEMON_STATUS_TIMEOUT.as_secs().to_string())]
            )
        ),
    }
    println!(
        "daemon: {}",
        crate::i18n::tr("cli.service.not_running", &[])
    );
    let plist = plist_path();
    if plist.exists() {
        println!(
            "launchd.plist: {}",
            crate::i18n::tr(
                "cli.service.plist_installed",
                &[("path", plist.display().to_string())]
            )
        );
    } else {
        println!(
            "launchd.plist: {}",
            crate::i18n::tr(
                "cli.service.plist_not_installed",
                &[("path", plist.display().to_string())]
            )
        );
    }
    Ok(())
}

async fn uds_status() -> Result<Option<String>> {
    let mut client =
        match crate::ipc::client::IpcClient::connect(crate::ipc::server::default_socket_path())
            .await
        {
            Ok(client) => client,
            Err(_) => return Ok(None),
        };
    client.send(&Command::DaemonStatus).await?;
    client
        .recv_until(|event| match event {
            Event::DaemonStatus {
                pid,
                uptime_ms,
                state,
                recording_id,
            } => Ok(ControlFlow::Break(format!(
                "daemon: running pid={pid} uptime={} state={state:?} recording={}",
                format_duration(uptime_ms),
                recording_id.as_deref().unwrap_or("-")
            ))),
            Event::Error { kind, msg, .. } => anyhow::bail!("{kind}: {msg}"),
            _ => Ok(ControlFlow::Continue(())),
        })
        .await
}

fn run_launchctl(args: &[&str], action_key: &str) -> Result<()> {
    let output = ProcessCommand::new("/bin/launchctl")
        .args(args)
        .output()
        .with_context(|| launchctl_spawn_context(args, action_key))?;
    if !output.status.success() {
        anyhow::bail!(
            "{}",
            launchctl_failure_message(
                args,
                action_key,
                &output.status.to_string(),
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            )
        );
    }
    Ok(())
}

fn launchctl_spawn_context(args: &[&str], action_key: &str) -> String {
    format!(
        "{}\n{}",
        crate::i18n::tr(action_key, &[]),
        crate::i18n::tr("cli.service.launchctl_command", &[("args", args.join(" "))])
    )
}

fn tr_path(key: &str, path: &std::path::Path) -> String {
    crate::i18n::tr(key, &[("path", path.display().to_string())])
}

fn launchctl_failure_message(
    args: &[&str],
    action_key: &str,
    status: &str,
    stdout: &str,
    stderr: &str,
) -> String {
    format!(
        "{}\n{}",
        crate::i18n::tr(action_key, &[]),
        crate::i18n::tr(
            "cli.service.launchctl_failed",
            &[
                ("args", args.join(" ")),
                ("status", status.to_string()),
                ("stdout", stdout.to_string()),
                ("stderr", stderr.to_string())
            ]
        )
    )
}

fn plist_body(exe: &std::path::Path, state_dir: &std::path::Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>--daemon</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ThrottleInterval</key>
  <integer>10</integer>
  <key>ProcessType</key>
  <string>Interactive</string>
  <key>StandardOutPath</key>
  <string>{}/launchd.stdout.log</string>
  <key>StandardErrorPath</key>
  <string>{}/launchd.stderr.log</string>
</dict>
</plist>
"#,
        xml_escape(&exe.display().to_string()),
        xml_escape(&state_dir.display().to_string()),
        xml_escape(&state_dir.display().to_string())
    )
}

fn gui_domain() -> String {
    let uid = unsafe { libc::getuid() };
    format!("gui/{uid}")
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn format_duration(ms: u64) -> String {
    let secs = ms / 1000;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h{m}m{s}s")
    } else if m > 0 {
        format!("{m}m{s}s")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[test]
    fn status_timeout_matches_cli_contract() {
        assert_eq!(super::DAEMON_STATUS_TIMEOUT, Duration::from_secs(1));
    }

    #[test]
    fn launchctl_failure_keeps_raw_output_with_action_hint() {
        crate::i18n::init("en-US");

        let msg = super::launchctl_failure_message(
            &["kickstart", "-k", "gui/501/com.hza2002.shuohua"],
            "cli.service.action_start",
            "exit status: 113",
            "out",
            "service not found",
        );

        assert!(msg.contains("starting launchd service"), "{msg}");
        assert!(msg.contains("run `shuo install` first"), "{msg}");
        assert!(msg.contains("service not found"), "{msg}");
    }
}
