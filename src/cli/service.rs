use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use anyhow::{Context, Result};

use crate::ipc::protocol::{Command, Event};

const LABEL: &str = "com.hza2002.shuohua";

pub fn plist_path() -> PathBuf {
    home_dir().join("Library/LaunchAgents/com.hza2002.shuohua.plist")
}

pub fn install() -> Result<()> {
    let state_dir = crate::state::history::state_dir();
    std::fs::create_dir_all(&state_dir)
        .with_context(|| format!("create state dir {}", state_dir.display()))?;
    let plist = plist_path();
    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create launch agents dir {}", parent.display()))?;
    }
    let exe = std::env::current_exe().context("resolve current shuo path")?;
    let body = plist_body(&exe, &state_dir);
    std::fs::write(&plist, body).with_context(|| format!("write {}", plist.display()))?;
    let _ = run_launchctl(&["bootout", &gui_domain(), plist.to_str().unwrap_or_default()]);
    run_launchctl(&[
        "bootstrap",
        &gui_domain(),
        plist.to_str().unwrap_or_default(),
    ])?;
    run_launchctl(&["kickstart", "-k", &format!("{}/{}", gui_domain(), LABEL)])?;
    println!("installed {}", plist.display());
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let plist = plist_path();
    let _ = run_launchctl(&["bootout", &gui_domain(), plist.to_str().unwrap_or_default()]);
    if plist.exists() {
        std::fs::remove_file(&plist).with_context(|| format!("remove {}", plist.display()))?;
    }
    println!("uninstalled {}", plist.display());
    Ok(())
}

pub fn start() -> Result<()> {
    run_launchctl(&["kickstart", "-k", &format!("{}/{}", gui_domain(), LABEL)])?;
    println!("started {LABEL}");
    Ok(())
}

pub fn stop() -> Result<()> {
    run_launchctl(&["kill", "TERM", &format!("{}/{}", gui_domain(), LABEL)])?;
    println!("stopped {LABEL}");
    Ok(())
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
    if let Some(line) = rt.block_on(uds_status())? {
        println!("{line}");
        return Ok(());
    }
    println!("daemon: not running");
    let plist = plist_path();
    if plist.exists() {
        println!("launchd.plist: installed {}", plist.display());
    } else {
        println!("launchd.plist: not installed {}", plist.display());
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
    while let Some(event) = client.recv().await? {
        match event {
            Event::DaemonStatus {
                pid,
                uptime_ms,
                state,
                recording_id,
            } => {
                return Ok(Some(format!(
                    "daemon: running pid={pid} uptime={} state={state:?} recording={}",
                    format_duration(uptime_ms),
                    recording_id.as_deref().unwrap_or("-")
                )));
            }
            Event::Error { kind, msg, .. } => anyhow::bail!("{kind}: {msg}"),
            _ => {}
        }
    }
    Ok(None)
}

fn run_launchctl(args: &[&str]) -> Result<()> {
    let output = ProcessCommand::new("/bin/launchctl")
        .args(args)
        .output()
        .with_context(|| format!("run launchctl {}", args.join(" ")))?;
    if !output.status.success() {
        anyhow::bail!(
            "launchctl {} failed with status {}\nstdout: {}\nstderr: {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
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
  <key>EnvironmentVariables</key>
  <dict>
    <key>RUST_LOG</key>
    <string>info</string>
  </dict>
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
