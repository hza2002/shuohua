use std::path::PathBuf;

use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchdStatus {
    Installed(PathBuf),
    NotInstalled(PathBuf),
    #[cfg(not(target_os = "macos"))]
    Unsupported,
}

#[cfg(target_os = "macos")]
mod imp {
    use std::future::Future;
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

    pub fn launchd_status() -> super::LaunchdStatus {
        let path = plist_path();
        if path.exists() {
            super::LaunchdStatus::Installed(path)
        } else {
            super::LaunchdStatus::NotInstalled(path)
        }
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
        let exe = std::env::current_exe()
            .context(crate::i18n::tr("cli.service.resolve_exe_failed", &[]))?;
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

    pub async fn start() -> Result<()> {
        start_launchd()
    }

    fn start_launchd() -> Result<()> {
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

    pub async fn stop() -> Result<()> {
        stop_with(request_daemon_shutdown, wait_for_pid_exit, || {
            println!(
                "{}",
                crate::i18n::tr("cli.service.stopped", &[("label", LABEL.to_string())])
            );
        })
        .await
    }

    async fn stop_with<F>(
        request_shutdown: impl FnOnce() -> F,
        wait_for_exit: impl FnOnce(crate::platform::lifecycle::Pid) -> Result<()>,
        print_stopped: impl FnOnce(),
    ) -> Result<()>
    where
        F: Future<Output = Result<crate::platform::lifecycle::Pid>>,
    {
        let pid = request_shutdown().await?;
        wait_for_exit(pid)?;
        print_stopped();
        Ok(())
    }

    async fn request_daemon_shutdown() -> Result<crate::platform::lifecycle::Pid> {
        tokio::time::timeout(DAEMON_STATUS_TIMEOUT, async {
            let mut client = crate::ipc::client::IpcClient::connect_default().await?;
            client.send(&Command::Shutdown).await?;
            parse_shutdown_reply(client.recv().await?)
        })
        .await
        .context("shutdown IPC timed out")?
    }

    fn parse_shutdown_reply(reply: Option<Event>) -> Result<crate::platform::lifecycle::Pid> {
        let pid = match reply {
            Some(Event::DaemonStatus { pid, .. }) => pid,
            Some(event) => {
                anyhow::bail!("expected DaemonStatus shutdown reply, received {event:?}")
            }
            None => anyhow::bail!("daemon closed IPC before sending DaemonStatus"),
        };
        if pid == 0 || pid > crate::platform::lifecycle::Pid::MAX as u32 {
            anyhow::bail!("invalid daemon PID in shutdown reply: {pid}");
        }
        Ok(pid as crate::platform::lifecycle::Pid)
    }

    fn wait_for_pid_exit(pid: crate::platform::lifecycle::Pid) -> Result<()> {
        wait_for_pid_exit_with(
            pid,
            DAEMON_EXIT_TIMEOUT,
            DAEMON_EXIT_POLL_INTERVAL,
            crate::platform::lifecycle::process_exists,
            std::thread::sleep,
            Instant::now,
        )
    }

    fn wait_for_pid_exit_with(
        pid: crate::platform::lifecycle::Pid,
        timeout: Duration,
        poll_interval: Duration,
        mut process_exists: impl FnMut(crate::platform::lifecycle::Pid) -> Result<bool>,
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

    pub async fn restart() -> Result<()> {
        restart_with(stop, start).await
    }

    async fn restart_with<F>(
        stop: impl FnOnce() -> F,
        start: impl FnOnce() -> Result<()>,
    ) -> Result<()>
    where
        F: Future<Output = Result<()>>,
    {
        stop().await?;
        start()
    }

    pub async fn status() -> Result<()> {
        match tokio::time::timeout(DAEMON_STATUS_TIMEOUT, uds_status()).await {
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
        let mut client = match crate::ipc::client::IpcClient::connect_default().await {
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
            assert!(msg.contains("run `shuo service install` first"), "{msg}");
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
            let times =
                RefCell::new(vec![start, start, start + Duration::from_secs(20)].into_iter());
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
            let times =
                RefCell::new(vec![start, start, start + Duration::from_secs(21)].into_iter());
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

        #[tokio::test(flavor = "current_thread")]
        async fn stop_prints_only_after_pid_exit_is_confirmed() {
            let calls = RefCell::new(Vec::new());

            super::stop_with(
                || async {
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
            .await
            .unwrap();

            assert_eq!(*calls.borrow(), ["request", "wait", "print"]);
        }

        #[tokio::test(flavor = "current_thread")]
        async fn restart_propagates_stop_error_without_starting() {
            let starts = Cell::new(0);

            let error = super::restart_with(
                || async { Err(anyhow!("stop failed")) },
                || {
                    starts.set(starts.get() + 1);
                    Ok(())
                },
            )
            .await
            .unwrap_err();

            assert_eq!(error.to_string(), "stop failed");
            assert_eq!(starts.get(), 0);
        }

        #[tokio::test(flavor = "current_thread")]
        async fn restart_starts_only_after_stop_completes() {
            let calls = RefCell::new(Vec::new());

            super::restart_with(
                || async {
                    calls.borrow_mut().push("stop");
                    Ok(())
                },
                || {
                    calls.borrow_mut().push("start");
                    Ok(())
                },
            )
            .await
            .unwrap();

            assert_eq!(*calls.borrow(), ["stop", "start"]);
        }

        #[test]
        fn stop_timeout_exceeds_daemon_graceful_shutdown_timeout() {
            assert!(super::DAEMON_EXIT_TIMEOUT > Duration::from_secs(15));
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::path::PathBuf;

    use anyhow::Result;

    const UNIT_NAME: &str = "shuohua.service";

    pub fn launchd_status() -> super::LaunchdStatus {
        super::LaunchdStatus::Unsupported
    }

    pub fn install() -> Result<()> {
        unsupported()
    }

    pub fn uninstall() -> Result<()> {
        unsupported()
    }

    pub async fn start() -> Result<()> {
        unsupported()
    }

    pub async fn stop() -> Result<()> {
        unsupported()
    }

    pub async fn restart() -> Result<()> {
        unsupported()
    }

    pub async fn status() -> Result<()> {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("shuo"));
        let unit_body = unit_body(&exe);
        let exec_line = unit_body
            .lines()
            .find(|line| line.starts_with("ExecStart="))
            .unwrap_or("ExecStart=shuo --daemon");
        println!(
            "daemon: {}",
            crate::i18n::tr("cli.service.not_running", &[])
        );
        println!(
            "systemd.user: dry-run unit={} path={} {} install_start=unsupported",
            unit_name(),
            unit_path().display(),
            exec_line
        );
        Ok(())
    }

    fn unit_name() -> &'static str {
        UNIT_NAME
    }

    fn unit_path() -> PathBuf {
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_else(|| PathBuf::from(".config"));
        config_home.join("systemd/user").join(UNIT_NAME)
    }

    fn unit_body(exe: &std::path::Path) -> String {
        format!(
            r#"[Unit]
Description=Shuohua voice input daemon

[Service]
ExecStart={} --daemon
Restart=on-failure
RestartSec=2s

[Install]
WantedBy=default.target
"#,
            systemd_escape_arg(&exe.display().to_string())
        )
    }

    fn systemd_escape_arg(value: &str) -> String {
        value.replace('\\', "\\\\").replace(' ', "\\x20")
    }

    fn unsupported<T>() -> Result<T> {
        // systemctl --user is intentionally not called in the dry-run skeleton.
        anyhow::bail!("systemd user service management is not implemented yet")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn unit_path_uses_xdg_config_home() {
            std::env::set_var("XDG_CONFIG_HOME", "/tmp/shuohua-config");
            assert_eq!(
                unit_path(),
                PathBuf::from("/tmp/shuohua-config/systemd/user/shuohua.service")
            );
            std::env::remove_var("XDG_CONFIG_HOME");
        }

        #[test]
        fn unit_body_runs_current_binary_as_daemon_with_restart_policy() {
            let body = unit_body(std::path::Path::new("/usr/local/bin/shuo"));
            assert!(body.contains("ExecStart=/usr/local/bin/shuo --daemon"));
            assert!(body.contains("Restart=on-failure"));
            assert!(body.contains("RestartSec=2s"));
            assert!(body.contains("WantedBy=default.target"));
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ops::ControlFlow;
    use std::path::PathBuf;
    use std::process::{Command as ProcessCommand, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result};

    use crate::ipc::protocol::{Command, Event};

    const DAEMON_STATUS_TIMEOUT: Duration = Duration::from_secs(1);
    const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(2);
    const DAEMON_EXIT_TIMEOUT: Duration = Duration::from_secs(20);
    const DAEMON_START_POLL_INTERVAL: Duration = Duration::from_millis(50);
    const DAEMON_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(100);
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_PATH_NOT_FOUND: i32 = 3;
    const WINDOWS_SERVICE_LABEL: &str = "windows.user";

    pub fn launchd_status() -> super::LaunchdStatus {
        super::LaunchdStatus::Unsupported
    }

    pub fn install() -> Result<()> {
        unsupported()
    }

    pub fn uninstall() -> Result<()> {
        unsupported()
    }

    pub async fn start() -> Result<()> {
        start_with(
            current_daemon_status,
            spawn_daemon_process,
            wait_for_daemon_ready,
            |line| {
                println!("{line}");
                println!(
                    "{}",
                    crate::i18n::tr(
                        "cli.service.already_running",
                        &[("label", WINDOWS_SERVICE_LABEL.to_string())]
                    )
                );
            },
            |_line| {
                println!(
                    "{}",
                    crate::i18n::tr(
                        "cli.service.started",
                        &[("label", WINDOWS_SERVICE_LABEL.to_string())]
                    )
                );
            },
        )
        .await
    }

    pub async fn stop() -> Result<()> {
        stop_with(request_daemon_shutdown, wait_for_pid_exit, || {
            println!(
                "{}",
                crate::i18n::tr(
                    "cli.service.stopped",
                    &[("label", WINDOWS_SERVICE_LABEL.to_string())]
                )
            );
        })
        .await
    }

    pub async fn restart() -> Result<()> {
        restart_with(current_daemon_status, stop, start).await
    }

    pub async fn status() -> Result<()> {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("shuo"));
        match current_daemon_status().await {
            Ok(Some(line)) => println!("{line}"),
            Ok(None) => println!(
                "daemon: {}",
                crate::i18n::tr("cli.service.not_running", &[])
            ),
            Err(error) => return Err(error),
        }
        println!(
            "windows.user: dry-run strategy={} command={} start=explicit_process startup_registration=unsupported",
            service_strategy(),
            daemon_command(&exe)
        );
        Ok(())
    }

    async fn start_with<FStatus, FStatusFuture, FSpawn, FWait, FWaitFuture>(
        current_status: FStatus,
        spawn_daemon: FSpawn,
        wait_for_ready: FWait,
        print_already_running: impl FnOnce(&str),
        print_started: impl FnOnce(&str),
    ) -> Result<()>
    where
        FStatus: FnOnce() -> FStatusFuture,
        FStatusFuture: std::future::Future<Output = Result<Option<String>>>,
        FSpawn: FnOnce() -> Result<()>,
        FWait: FnOnce() -> FWaitFuture,
        FWaitFuture: std::future::Future<Output = Result<String>>,
    {
        if let Some(line) = current_status().await? {
            print_already_running(&line);
            return Ok(());
        }

        spawn_daemon()?;
        let line = wait_for_ready().await?;
        print_started(&line);
        Ok(())
    }

    async fn restart_with<FStatus, FStatusFuture, FStop, FStopFuture, FStart, FStartFuture>(
        current_status: FStatus,
        stop: FStop,
        start: FStart,
    ) -> Result<()>
    where
        FStatus: FnOnce() -> FStatusFuture,
        FStatusFuture: std::future::Future<Output = Result<Option<String>>>,
        FStop: FnOnce() -> FStopFuture,
        FStopFuture: std::future::Future<Output = Result<()>>,
        FStart: FnOnce() -> FStartFuture,
        FStartFuture: std::future::Future<Output = Result<()>>,
    {
        if current_status().await?.is_some() {
            stop().await?;
        }
        start().await
    }

    async fn stop_with<F>(
        request_shutdown: impl FnOnce() -> F,
        wait_for_exit: impl FnOnce(crate::platform::lifecycle::Pid) -> Result<()>,
        print_stopped: impl FnOnce(),
    ) -> Result<()>
    where
        F: std::future::Future<Output = Result<crate::platform::lifecycle::Pid>>,
    {
        let pid = request_shutdown().await?;
        wait_for_exit(pid)?;
        print_stopped();
        Ok(())
    }

    async fn request_daemon_shutdown() -> Result<crate::platform::lifecycle::Pid> {
        tokio::time::timeout(DAEMON_STATUS_TIMEOUT, async {
            let mut client = match crate::ipc::client::IpcClient::connect_default().await {
                Ok(client) => client,
                Err(error) if is_daemon_absent_connect_error(&error) => {
                    anyhow::bail!("daemon is not running")
                }
                Err(error) => return Err(error.context("connect Windows daemon for shutdown")),
            };
            client.send(&Command::Shutdown).await?;
            parse_shutdown_reply(client.recv().await?)
        })
        .await
        .context("shutdown IPC timed out")?
    }

    async fn current_daemon_status() -> Result<Option<String>> {
        match tokio::time::timeout(DAEMON_STATUS_TIMEOUT, ipc_status()).await {
            Ok(result) => result,
            Err(_) => anyhow::bail!(
                "{}",
                crate::i18n::tr(
                    "cli.service.status_timeout",
                    &[("seconds", DAEMON_STATUS_TIMEOUT.as_secs().to_string())]
                )
            ),
        }
    }

    fn spawn_daemon_process() -> Result<()> {
        let stderr = service_log("service.start.stderr.log")?;
        let stdout = service_log("service.start.stdout.log")?;
        let child = ProcessCommand::new(std::env::current_exe().context("resolve current exe")?)
            .arg("--daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .context("spawn shuo --daemon")?;
        drop(child);
        Ok(())
    }

    fn service_log(name: &str) -> Result<std::fs::File> {
        let dir = crate::paths::StateDirs::discover().root().to_path_buf();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create state dir {}", dir.display()))?;
        let path = dir.join(name);
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open {}", path.display()))
    }

    async fn wait_for_daemon_ready() -> Result<String> {
        let deadline = Instant::now() + DAEMON_START_TIMEOUT;
        loop {
            match current_daemon_status().await {
                Ok(Some(line)) => return Ok(line),
                Ok(None) => {}
                Err(error) => return Err(error).context("wait for Windows daemon IPC ready"),
            }

            if Instant::now() >= deadline {
                anyhow::bail!(
                    "daemon did not accept IPC connections within {:?}",
                    DAEMON_START_TIMEOUT
                );
            }
            tokio::time::sleep(DAEMON_START_POLL_INTERVAL).await;
        }
    }

    fn parse_shutdown_reply(reply: Option<Event>) -> Result<crate::platform::lifecycle::Pid> {
        let pid = match reply {
            Some(Event::DaemonStatus { pid, .. }) => pid,
            Some(event) => {
                anyhow::bail!("expected DaemonStatus shutdown reply, received {event:?}")
            }
            None => anyhow::bail!("daemon closed IPC before sending DaemonStatus"),
        };
        if pid == 0 {
            anyhow::bail!("invalid daemon PID in shutdown reply: {pid}");
        }
        Ok(pid as crate::platform::lifecycle::Pid)
    }

    fn wait_for_pid_exit(pid: crate::platform::lifecycle::Pid) -> Result<()> {
        wait_for_pid_exit_with(
            pid,
            DAEMON_EXIT_TIMEOUT,
            DAEMON_EXIT_POLL_INTERVAL,
            crate::platform::lifecycle::process_exists,
            std::thread::sleep,
            Instant::now,
        )
    }

    fn wait_for_pid_exit_with(
        pid: crate::platform::lifecycle::Pid,
        timeout: Duration,
        poll_interval: Duration,
        mut process_exists: impl FnMut(crate::platform::lifecycle::Pid) -> Result<bool>,
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
                    "timed out after {}s waiting for Windows daemon PID {pid} to exit",
                    timeout.as_secs()
                );
            }
            sleep(poll_interval);
        }
    }

    async fn ipc_status() -> Result<Option<String>> {
        let mut client = match crate::ipc::client::IpcClient::connect_default().await {
            Ok(client) => client,
            Err(error) if is_daemon_absent_connect_error(&error) => return Ok(None),
            Err(error) => return Err(error.context("query Windows daemon status")),
        };
        client.send(&Command::DaemonStatus).await?;
        client
            .recv_until(|event| match event {
                Event::DaemonStatus {
                    pid,
                    uptime_ms,
                    state,
                    recording_id,
                } => Ok(ControlFlow::Break(format_daemon_status(
                    pid,
                    uptime_ms,
                    &format!("{state:?}"),
                    recording_id.as_deref().unwrap_or("-"),
                ))),
                Event::Error { kind, msg, .. } => anyhow::bail!("{kind}: {msg}"),
                _ => Ok(ControlFlow::Continue(())),
            })
            .await
    }

    fn is_daemon_absent_connect_error(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .and_then(std::io::Error::raw_os_error)
                .is_some_and(|code| matches!(code, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND))
        })
    }

    fn format_daemon_status(pid: u32, uptime_ms: u64, state: &str, recording_id: &str) -> String {
        format!(
            "daemon: running pid={pid} uptime={} state={state} recording={recording_id}",
            format_duration(uptime_ms)
        )
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

    fn service_strategy() -> &'static str {
        "user_session_logon_task"
    }

    fn daemon_command(exe: &std::path::Path) -> String {
        format!("{} --daemon", quote_arg(&exe.display().to_string()))
    }

    fn quote_arg(value: &str) -> String {
        if value.contains(' ') {
            format!("\"{}\"", value.replace('"', "\\\""))
        } else {
            value.to_string()
        }
    }

    fn unsupported<T>() -> Result<T> {
        // Task Scheduler, SCM, PowerShell, and registry APIs are intentionally not called.
        anyhow::bail!("Windows service management is not implemented yet")
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::cell::{Cell, RefCell};

        #[test]
        fn service_strategy_is_user_session_scoped() {
            assert_eq!(service_strategy(), "user_session_logon_task");
        }

        #[test]
        fn daemon_command_quotes_paths_with_spaces() {
            assert_eq!(
                daemon_command(std::path::Path::new("C:\\Program Files\\Shuo\\shuo.exe")),
                "\"C:\\Program Files\\Shuo\\shuo.exe\" --daemon"
            );
            assert_eq!(
                daemon_command(std::path::Path::new("C:\\Shuo\\shuo.exe")),
                "C:\\Shuo\\shuo.exe --daemon"
            );
        }

        #[test]
        fn daemon_status_line_matches_cli_contract() {
            assert_eq!(
                format_daemon_status(42, 65_000, "Idle", "-"),
                "daemon: running pid=42 uptime=1m5s state=Idle recording=-"
            );
        }

        #[tokio::test(flavor = "current_thread")]
        async fn start_reports_existing_daemon_without_spawning() {
            let spawned = Cell::new(false);
            let calls = RefCell::new(Vec::new());

            start_with(
                || async { Ok(Some("daemon: running pid=42".to_string())) },
                || {
                    spawned.set(true);
                    Ok(())
                },
                || async { Ok("daemon: running pid=43".to_string()) },
                |line| calls.borrow_mut().push(format!("already:{line}")),
                |line| calls.borrow_mut().push(format!("started:{line}")),
            )
            .await
            .unwrap();

            assert!(!spawned.get());
            assert_eq!(&*calls.borrow(), &["already:daemon: running pid=42"]);
        }

        #[tokio::test(flavor = "current_thread")]
        async fn start_spawns_then_waits_for_daemon_ready() {
            let calls = RefCell::new(Vec::new());

            start_with(
                || async { Ok(None) },
                || {
                    calls.borrow_mut().push("spawn".to_string());
                    Ok(())
                },
                || async {
                    calls.borrow_mut().push("wait".to_string());
                    Ok("daemon: running pid=43".to_string())
                },
                |line| calls.borrow_mut().push(format!("already:{line}")),
                |line| calls.borrow_mut().push(format!("started:{line}")),
            )
            .await
            .unwrap();

            assert_eq!(
                &*calls.borrow(),
                &["spawn", "wait", "started:daemon: running pid=43"]
            );
        }

        #[tokio::test(flavor = "current_thread")]
        async fn restart_stops_running_daemon_before_starting() {
            let calls = RefCell::new(Vec::new());

            restart_with(
                || async { Ok(Some("daemon: running pid=42".to_string())) },
                || async {
                    calls.borrow_mut().push("stop");
                    Ok(())
                },
                || async {
                    calls.borrow_mut().push("start");
                    Ok(())
                },
            )
            .await
            .unwrap();

            assert_eq!(&*calls.borrow(), &["stop", "start"]);
        }

        #[tokio::test(flavor = "current_thread")]
        async fn restart_starts_when_daemon_is_absent() {
            let calls = RefCell::new(Vec::new());

            restart_with(
                || async { Ok(None) },
                || async {
                    calls.borrow_mut().push("stop");
                    Ok(())
                },
                || async {
                    calls.borrow_mut().push("start");
                    Ok(())
                },
            )
            .await
            .unwrap();

            assert_eq!(&*calls.borrow(), &["start"]);
        }

        #[test]
        fn shutdown_reply_returns_daemon_pid() {
            let pid = parse_shutdown_reply(Some(Event::DaemonStatus {
                pid: 42,
                uptime_ms: 1,
                state: crate::ipc::protocol::WireState::Idle,
                recording_id: None,
            }))
            .unwrap();

            assert_eq!(pid, 42);
        }

        #[test]
        fn shutdown_reply_rejects_other_event_and_eof() {
            let other = parse_shutdown_reply(Some(Event::ConfigReloaded {
                path: "config.toml".to_string(),
            }))
            .unwrap_err();
            let eof = parse_shutdown_reply(None).unwrap_err();

            assert!(other.to_string().contains("DaemonStatus"), "{other:#}");
            assert!(eof.to_string().contains("closed"), "{eof:#}");
        }

        #[test]
        fn shutdown_reply_rejects_invalid_pid() {
            let error = parse_shutdown_reply(Some(Event::DaemonStatus {
                pid: 0,
                uptime_ms: 1,
                state: crate::ipc::protocol::WireState::Idle,
                recording_id: None,
            }))
            .unwrap_err();

            assert!(
                error.to_string().contains("invalid daemon PID"),
                "{error:#}"
            );
        }

        #[test]
        fn wait_for_pid_exit_stops_after_process_disappears() {
            let mut probes = vec![true, true, false].into_iter();
            let mut sleeps = 0;
            let now = Instant::now();

            wait_for_pid_exit_with(
                42,
                Duration::from_secs(20),
                Duration::from_millis(100),
                |_| Ok(probes.next().unwrap()),
                |_| sleeps += 1,
                || now,
            )
            .unwrap();

            assert_eq!(sleeps, 2);
        }

        #[test]
        fn wait_for_pid_exit_times_out_without_killing() {
            let start = Instant::now();
            let mut times = vec![start, start, start + Duration::from_secs(21)].into_iter();
            let mut probes = 0;

            let error = wait_for_pid_exit_with(
                42,
                Duration::from_secs(20),
                Duration::from_millis(100),
                |_| {
                    probes += 1;
                    Ok(true)
                },
                |_| {},
                || times.next().unwrap(),
            )
            .unwrap_err();

            assert!(error.to_string().contains("timed out"), "{error:#}");
            assert_eq!(probes, 2);
        }

        #[test]
        fn daemon_absent_connect_errors_are_narrowly_classified() {
            let missing = std::io::Error::from_raw_os_error(ERROR_FILE_NOT_FOUND);
            let missing_with_context: anyhow::Error =
                anyhow::Error::new(missing).context("connect Windows IPC pipe");
            let access_denied = anyhow::Error::new(std::io::Error::from_raw_os_error(5))
                .context("connect Windows IPC pipe");

            assert!(is_daemon_absent_connect_error(&missing_with_context));
            assert!(!is_daemon_absent_connect_error(&access_denied));
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
mod imp {
    use anyhow::Result;

    pub fn launchd_status() -> super::LaunchdStatus {
        super::LaunchdStatus::Unsupported
    }

    pub fn install() -> Result<()> {
        unsupported()
    }

    pub fn uninstall() -> Result<()> {
        unsupported()
    }

    pub async fn start() -> Result<()> {
        unsupported()
    }

    pub async fn stop() -> Result<()> {
        unsupported()
    }

    pub async fn restart() -> Result<()> {
        unsupported()
    }

    pub async fn status() -> Result<()> {
        unsupported()
    }

    fn unsupported<T>() -> Result<T> {
        anyhow::bail!("service management is not supported on this platform yet")
    }
}

pub fn launchd_status() -> LaunchdStatus {
    imp::launchd_status()
}

pub fn install() -> Result<()> {
    imp::install()
}

pub fn uninstall() -> Result<()> {
    imp::uninstall()
}

pub async fn start() -> Result<()> {
    imp::start().await
}

pub async fn stop() -> Result<()> {
    imp::stop().await
}

pub async fn restart() -> Result<()> {
    imp::restart().await
}

pub async fn status() -> Result<()> {
    imp::status().await
}
