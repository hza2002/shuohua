use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::ipc::protocol::{Command, Event};

const LABEL: &str = "com.hza2002.shuohua";
const DAEMON_STATUS_TIMEOUT: Duration = Duration::from_secs(1);
const DAEMON_EXIT_TIMEOUT: Duration = Duration::from_secs(20);
const DAEMON_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub fn plist_path() -> PathBuf {
    home_dir().join("Library/LaunchAgents/com.hza2002.shuohua.plist")
}

pub fn install() -> Result<()> {
    let state_dir = crate::paths::StateDirs::discover().root().to_path_buf();
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
    stop_with(request_daemon_shutdown, wait_for_pid_exit, || {
        println!(
            "{}",
            crate::i18n::tr("cli.service.stopped", &[("label", LABEL.to_string())])
        );
    })
}

fn stop_with(
    request_shutdown: impl FnOnce() -> Result<libc::pid_t>,
    wait_for_exit: impl FnOnce(libc::pid_t) -> Result<()>,
    print_stopped: impl FnOnce(),
) -> Result<()> {
    let pid = request_shutdown()?;
    wait_for_exit(pid)?;
    print_stopped();
    Ok(())
}

fn request_daemon_shutdown() -> Result<libc::pid_t> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create stop runtime")?;
    rt.block_on(tokio::time::timeout(DAEMON_STATUS_TIMEOUT, async {
        let mut client =
            crate::ipc::client::IpcClient::connect(crate::ipc::server::default_socket_path())
                .await?;
        client.send(&Command::Shutdown).await?;
        parse_shutdown_reply(client.recv().await?)
    }))
    .context("shutdown IPC timed out")?
}

fn parse_shutdown_reply(reply: Option<Event>) -> Result<libc::pid_t> {
    let pid = match reply {
        Some(Event::DaemonStatus { pid, .. }) => pid,
        Some(event) => anyhow::bail!("expected DaemonStatus shutdown reply, received {event:?}"),
        None => anyhow::bail!("daemon closed IPC before sending DaemonStatus"),
    };
    if pid == 0 || pid > libc::pid_t::MAX as u32 {
        anyhow::bail!("invalid daemon PID in shutdown reply: {pid}");
    }
    Ok(pid as libc::pid_t)
}

fn wait_for_pid_exit(pid: libc::pid_t) -> Result<()> {
    wait_for_pid_exit_with(
        pid,
        DAEMON_EXIT_TIMEOUT,
        DAEMON_EXIT_POLL_INTERVAL,
        process_exists,
        std::thread::sleep,
        Instant::now,
    )
}

fn wait_for_pid_exit_with(
    pid: libc::pid_t,
    timeout: Duration,
    poll_interval: Duration,
    mut process_exists: impl FnMut(libc::pid_t) -> Result<bool>,
    mut sleep: impl FnMut(Duration),
    mut now: impl FnMut() -> Instant,
) -> Result<()> {
    let deadline = now() + timeout;
    loop {
        if !process_exists(pid)? {
            return Ok(());
        }
        if now() >= deadline {
            anyhow::bail!(
                "timed out after {}s waiting for daemon PID {pid} to exit",
                timeout.as_secs()
            );
        }
        sleep(poll_interval);
    }
}

fn process_exists(pid: libc::pid_t) -> Result<bool> {
    debug_assert!(pid > 0);
    let result = unsafe { libc::kill(pid, 0) };
    let errno = if result == -1 {
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    } else {
        0
    };
    process_exists_from_kill_result(result, errno)
}

fn process_exists_from_kill_result(result: libc::c_int, errno: libc::c_int) -> Result<bool> {
    if result == 0 {
        return Ok(true);
    }
    match errno {
        libc::EPERM => Ok(true),
        libc::ESRCH => Ok(false),
        _ => anyhow::bail!(
            "check daemon process failed: {}",
            std::io::Error::from_raw_os_error(errno)
        ),
    }
}

pub fn restart() -> Result<()> {
    restart_with(stop, start)
}

fn restart_with(
    stop: impl FnOnce() -> Result<()>,
    start: impl FnOnce() -> Result<()>,
) -> Result<()> {
    stop()?;
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
    use std::cell::{Cell, RefCell};
    use std::time::{Duration, Instant};

    use anyhow::anyhow;

    use crate::ipc::protocol::{Event, WireState};

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

    #[test]
    fn shutdown_reply_returns_daemon_pid() {
        let pid = super::parse_shutdown_reply(Some(Event::DaemonStatus {
            pid: 42,
            uptime_ms: 1,
            state: WireState::Idle,
            recording_id: None,
        }))
        .unwrap();

        assert_eq!(pid, 42);
    }

    #[test]
    fn shutdown_reply_rejects_other_event_and_eof() {
        let other = super::parse_shutdown_reply(Some(Event::ConfigReloaded {
            path: "config.toml".to_string(),
        }))
        .unwrap_err();
        let eof = super::parse_shutdown_reply(None).unwrap_err();

        assert!(other.to_string().contains("DaemonStatus"), "{other:#}");
        assert!(eof.to_string().contains("closed"), "{eof:#}");
    }

    #[test]
    fn shutdown_reply_rejects_invalid_pid() {
        for pid in [0, i32::MAX as u32 + 1] {
            let error = super::parse_shutdown_reply(Some(Event::DaemonStatus {
                pid,
                uptime_ms: 1,
                state: WireState::Idle,
                recording_id: None,
            }))
            .unwrap_err();

            assert!(
                error.to_string().contains("invalid daemon PID"),
                "{error:#}"
            );
        }
    }

    #[test]
    fn pid_probe_treats_eperm_as_running_and_esrch_as_exited() {
        assert!(super::process_exists_from_kill_result(-1, libc::EPERM).unwrap());
        assert!(!super::process_exists_from_kill_result(-1, libc::ESRCH).unwrap());
        assert!(super::process_exists_from_kill_result(0, 0).unwrap());
        assert!(super::process_exists_from_kill_result(-1, libc::EIO).is_err());
    }

    #[test]
    fn wait_for_pid_exit_stops_after_process_disappears() {
        let probes = RefCell::new(vec![true, true, false].into_iter());
        let sleeps = Cell::new(0);
        let now = Instant::now();

        super::wait_for_pid_exit_with(
            42,
            Duration::from_secs(20),
            Duration::from_millis(100),
            |_| Ok(probes.borrow_mut().next().unwrap()),
            |_| sleeps.set(sleeps.get() + 1),
            || now,
        )
        .unwrap();

        assert_eq!(sleeps.get(), 2);
    }

    #[test]
    fn wait_for_pid_exit_accepts_exit_observed_at_deadline() {
        let start = Instant::now();
        let times = RefCell::new(vec![start, start, start + Duration::from_secs(20)].into_iter());
        let probes = RefCell::new(vec![true, false].into_iter());
        let sleeps = Cell::new(0);

        super::wait_for_pid_exit_with(
            42,
            Duration::from_secs(20),
            Duration::from_millis(100),
            |_| Ok(probes.borrow_mut().next().unwrap()),
            |_| sleeps.set(sleeps.get() + 1),
            || times.borrow_mut().next().unwrap(),
        )
        .unwrap();

        assert_eq!(sleeps.get(), 1);
    }

    #[test]
    fn wait_for_pid_exit_times_out_without_killing() {
        let start = Instant::now();
        let times = RefCell::new(vec![start, start, start + Duration::from_secs(21)].into_iter());
        let probes = Cell::new(0);

        let error = super::wait_for_pid_exit_with(
            42,
            Duration::from_secs(20),
            Duration::from_millis(100),
            |_| {
                probes.set(probes.get() + 1);
                Ok(true)
            },
            |_| {},
            || times.borrow_mut().next().unwrap(),
        )
        .unwrap_err();

        assert!(error.to_string().contains("timed out"), "{error:#}");
        assert_eq!(probes.get(), 2);
    }

    #[test]
    fn stop_prints_only_after_pid_exit_is_confirmed() {
        let calls = RefCell::new(Vec::new());

        super::stop_with(
            || {
                calls.borrow_mut().push("request");
                Ok(42)
            },
            |pid| {
                assert_eq!(pid, 42);
                calls.borrow_mut().push("wait");
                Ok(())
            },
            || calls.borrow_mut().push("print"),
        )
        .unwrap();

        assert_eq!(*calls.borrow(), ["request", "wait", "print"]);
    }

    #[test]
    fn restart_propagates_stop_error_without_starting() {
        let starts = Cell::new(0);

        let error = super::restart_with(
            || Err(anyhow!("stop failed")),
            || {
                starts.set(starts.get() + 1);
                Ok(())
            },
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "stop failed");
        assert_eq!(starts.get(), 0);
    }

    #[test]
    fn restart_starts_only_after_stop_completes() {
        let calls = RefCell::new(Vec::new());

        super::restart_with(
            || {
                calls.borrow_mut().push("stop");
                Ok(())
            },
            || {
                calls.borrow_mut().push("start");
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(*calls.borrow(), ["stop", "start"]);
    }

    #[test]
    fn stop_timeout_exceeds_daemon_graceful_shutdown_timeout() {
        assert!(super::DAEMON_EXIT_TIMEOUT > Duration::from_secs(15));
    }
}
