use std::fs;
use std::future::Future;
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::ipc::protocol::{Command, Event};

const LABEL: &str = "com.hza2002.shuohua";
const DAEMON_STATUS_TIMEOUT: Duration = Duration::from_secs(1);
const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(2);
const DAEMON_START_POLL_INTERVAL: Duration = Duration::from_millis(200);
const DAEMON_EXIT_TIMEOUT: Duration = Duration::from_secs(20);
const DAEMON_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub fn plist_path() -> PathBuf {
    home_dir().join("Library/LaunchAgents/com.hza2002.shuohua.plist")
}

pub fn plist_program() -> Option<PathBuf> {
    plist_program_argument(&plist_path())
}

pub fn launchd_status() -> super::LaunchdStatus {
    let path = plist_path();
    if path.exists() {
        super::LaunchdStatus::Installed(path)
    } else {
        super::LaunchdStatus::NotInstalled(path)
    }
}

pub async fn install() -> Result<()> {
    install_with(ensure_accessibility_for_service, run_launchctl, |plist| {
        println!(
            "{}",
            crate::i18n::tr(
                "cli.service.installed",
                &[("path", plist.display().to_string())]
            )
        );
    })
    .await?;
    // plist 钉死的是当前 exe 绝对路径；当前 binary 不在 preferred 路径时只警告、不失败
    // （源码构建/开发态从别处跑 install 是合法的），把用户引回 ~/.local/bin。
    for finding in install_drift_findings() {
        println!("{}", crate::install::render_drift(&finding));
    }
    Ok(())
}

async fn install_with(
    request_accessibility: impl FnOnce(),
    run_launchctl: impl Fn(&[&str], &str) -> Result<()>,
    print_installed: impl FnOnce(&std::path::Path),
) -> Result<()> {
    install_with_plan(
        request_accessibility,
        write_service_plist,
        run_launchctl,
        wait_for_daemon_ready,
        print_installed,
    )
    .await
}

async fn install_with_plan<F>(
    request_accessibility: impl FnOnce(),
    write_plist: impl FnOnce() -> Result<PathBuf>,
    run_launchctl: impl Fn(&[&str], &str) -> Result<()>,
    wait_ready: impl FnOnce() -> F,
    print_installed: impl FnOnce(&std::path::Path),
) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    request_accessibility();
    let plist = write_plist()?;
    bootstrap_written_plist(&plist, run_launchctl)?;
    wait_ready().await?;
    print_installed(&plist);
    Ok(())
}

fn write_service_plist() -> Result<PathBuf> {
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
    Ok(plist)
}

fn bootstrap_written_plist(
    plist: &std::path::Path,
    run_launchctl: impl Fn(&[&str], &str) -> Result<()>,
) -> Result<()> {
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
    )
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
    start_with(
        ensure_accessibility_for_service,
        run_launchctl,
        wait_for_daemon_ready,
        || {
            println!(
                "{}",
                crate::i18n::tr("cli.service.started", &[("label", LABEL.to_string())])
            );
        },
    )
    .await
}

fn start_with<F>(
    request_accessibility: impl FnOnce(),
    run_launchctl: impl FnOnce(&[&str], &str) -> Result<()>,
    wait_ready: impl FnOnce() -> F,
    print_started: impl FnOnce(),
) -> impl Future<Output = Result<()>>
where
    F: Future<Output = Result<()>>,
{
    start_with_plan(
        request_accessibility,
        run_launchctl,
        wait_ready,
        print_started,
    )
}

async fn start_with_plan<F>(
    request_accessibility: impl FnOnce(),
    run_launchctl: impl FnOnce(&[&str], &str) -> Result<()>,
    wait_ready: impl FnOnce() -> F,
    print_started: impl FnOnce(),
) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    request_accessibility();
    run_launchctl(
        &["kickstart", "-k", &format!("{}/{}", gui_domain(), LABEL)],
        "cli.service.action_start",
    )?;
    wait_ready().await?;
    print_started();
    Ok(())
}

fn ensure_accessibility_for_service() {
    ensure_accessibility_trust(crate::platform::permissions::accessibility_trusted, || {
        let _ = crate::platform::permissions::request_accessibility_trust();
    });
}

fn ensure_accessibility_trust(
    accessibility_trusted: impl FnOnce() -> bool,
    request: impl FnOnce(),
) {
    if !accessibility_trusted() {
        request();
    }
}

pub async fn stop() -> Result<()> {
    stop_with(
        request_daemon_shutdown,
        wait_for_pid_exit,
        || {
            println!(
                "{}",
                crate::i18n::tr("cli.service.stopped", &[("label", LABEL.to_string())])
            );
        },
        || {
            println!(
                "daemon: {}",
                crate::i18n::tr("cli.service.not_running", &[])
            );
        },
    )
    .await
}

async fn stop_with<F>(
    request_shutdown: impl FnOnce() -> F,
    wait_for_exit: impl FnOnce(libc::pid_t) -> Result<()>,
    print_stopped: impl FnOnce(),
    print_not_running: impl FnOnce(),
) -> Result<()>
where
    F: Future<Output = Result<Option<libc::pid_t>>>,
{
    let Some(pid) = request_shutdown().await? else {
        print_not_running();
        return Ok(());
    };
    wait_for_exit(pid)?;
    print_stopped();
    Ok(())
}

async fn request_daemon_shutdown() -> Result<Option<libc::pid_t>> {
    tokio::time::timeout(DAEMON_STATUS_TIMEOUT, async {
        let mut client =
            match crate::ipc::client::IpcClient::connect(crate::ipc::server::default_socket_path())
                .await
            {
                Ok(client) => client,
                Err(error) if crate::ipc::client::connect_error_is_absent(&error) => {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
        client.send(&Command::Shutdown).await?;
        parse_shutdown_reply(client.recv().await?).map(Some)
    })
    .await
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

pub async fn restart() -> Result<()> {
    restart_with(stop, start).await
}

async fn restart_with<F, G>(stop: impl FnOnce() -> F, start: impl FnOnce() -> G) -> Result<()>
where
    F: Future<Output = Result<()>>,
    G: Future<Output = Result<()>>,
{
    stop().await?;
    start().await
}

pub async fn status() -> Result<()> {
    match tokio::time::timeout(DAEMON_STATUS_TIMEOUT, uds_status()).await {
        Ok(Ok(Some(line))) => {
            println!("{line}");
        }
        Ok(Ok(None)) => {
            println!(
                "daemon: {}",
                crate::i18n::tr("cli.service.not_running", &[])
            );
        }
        Ok(Err(e)) => return Err(e),
        Err(_) => anyhow::bail!(
            "{}",
            crate::i18n::tr(
                "cli.service.status_timeout",
                &[("seconds", DAEMON_STATUS_TIMEOUT.as_secs().to_string())]
            )
        ),
    }
    let plist = plist_path();
    if plist.exists() {
        let diagnostic = latest_launchd_accessibility_diagnostic();
        let findings = install_drift_findings();
        write_launchd_diagnostics(
            &mut std::io::stdout(),
            &plist,
            diagnostic.as_deref(),
            &findings,
        )?;
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

fn write_launchd_diagnostics(
    out: &mut impl Write,
    plist: &std::path::Path,
    accessibility_diagnostic: Option<&str>,
    drift_findings: &[crate::install::DriftFinding],
) -> Result<()> {
    writeln!(
        out,
        "launchd.plist: {}",
        crate::i18n::tr(
            "cli.service.plist_installed",
            &[("path", plist.display().to_string())]
        )
    )?;
    if let Some(diagnostic) = accessibility_diagnostic {
        writeln!(out, "{diagnostic}")?;
    }
    for finding in drift_findings {
        writeln!(out, "{}", crate::install::render_drift(finding))?;
    }
    Ok(())
}

async fn uds_status() -> Result<Option<String>> {
    let mut client =
        match crate::ipc::client::IpcClient::connect(crate::ipc::server::default_socket_path())
            .await
        {
            Ok(client) => client,
            Err(error) if crate::ipc::client::connect_error_is_absent(&error) => return Ok(None),
            Err(error) => return Err(error),
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

async fn wait_for_daemon_ready() -> Result<()> {
    wait_for_daemon_ready_with(uds_status).await
}

async fn wait_for_daemon_ready_with<F, Fut>(mut probe: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<String>>>,
{
    let deadline = tokio::time::Instant::now() + DAEMON_START_TIMEOUT;
    let mut consecutive_ready = 0u8;
    let mut last_error = None;
    while tokio::time::Instant::now() < deadline {
        match probe().await {
            Ok(Some(_)) => {
                consecutive_ready += 1;
                if consecutive_ready >= 2 {
                    return Ok(());
                }
            }
            Ok(None) => {
                consecutive_ready = 0;
            }
            Err(error) => {
                consecutive_ready = 0;
                last_error = Some(error);
            }
        }
        tokio::time::sleep(DAEMON_START_POLL_INTERVAL).await;
    }
    let mut msg = crate::i18n::tr(
        "cli.service.start_health_failed",
        &[("seconds", DAEMON_START_TIMEOUT.as_secs().to_string())],
    );
    if let Some(error) = last_error {
        msg.push_str(&format!("\n{error:#}"));
    }
    if let Some(diagnostic) = latest_launchd_accessibility_diagnostic() {
        msg.push('\n');
        msg.push_str(&diagnostic);
    }
    for finding in install_drift_findings() {
        msg.push('\n');
        msg.push_str(&crate::install::render_drift(&finding));
    }
    anyhow::bail!("{msg}")
}

fn install_drift_findings() -> Vec<crate::install::DriftFinding> {
    let Ok(current) = std::env::current_exe() else {
        return Vec::new();
    };
    let Ok(preferred) = crate::install::InstallLayout::preferred_bin() else {
        return Vec::new();
    };
    let plist = plist_program_argument(&plist_path());
    let path_first = crate::install::path_first_binary();
    crate::install::diagnose_drift(
        &current,
        &preferred,
        plist.as_deref(),
        path_first.as_deref(),
    )
}

fn latest_launchd_accessibility_diagnostic() -> Option<String> {
    let log = read_recent_daemon_logs().ok()?;
    let exe = plist_program_argument(&plist_path()).or_else(|| std::env::current_exe().ok())?;
    launchd_accessibility_diagnostic(&log, &exe)
}

fn launchd_accessibility_diagnostic(log: &str, exe: &std::path::Path) -> Option<String> {
    if !log.contains("CGEventTapCreate failed") || !log.contains("Accessibility") {
        return None;
    }
    Some(crate::i18n::tr(
        "cli.service.diagnostic_accessibility",
        &[("path", exe.display().to_string())],
    ))
}

fn read_recent_daemon_logs() -> Result<String> {
    let state_dir = crate::paths::StateDirs::discover().root().to_path_buf();
    let mut combined = String::new();
    for path in [
        Some(state_dir.join("launchd.stderr.log")),
        latest_daemon_log_path(&state_dir)?,
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(text) = read_tail(&path, 16 * 1024) {
            combined.push_str(&text);
            combined.push('\n');
        }
    }
    Ok(combined)
}

fn latest_daemon_log_path(state_dir: &std::path::Path) -> Result<Option<PathBuf>> {
    let logs_dir = state_dir.join("logs");
    let mut latest = None;
    for entry in match fs::read_dir(&logs_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read {}", logs_dir.display())),
    } {
        let entry = entry.with_context(|| format!("read entry under {}", logs_dir.display()))?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("shuo-") || !name.ends_with(".log") {
            continue;
        }
        let modified = entry
            .metadata()
            .with_context(|| format!("stat {}", path.display()))?
            .modified()
            .with_context(|| format!("modified time {}", path.display()))?;
        if latest
            .as_ref()
            .is_none_or(|(latest_modified, _)| modified > *latest_modified)
        {
            latest = Some((modified, path));
        }
    }
    Ok(latest.map(|(_, path)| path))
}

fn read_tail(path: &std::path::Path, max_bytes: u64) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let len = file
        .metadata()
        .with_context(|| format!("stat {}", path.display()))?
        .len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start))
        .with_context(|| format!("seek {}", path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("read {}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn plist_program_argument(plist: &std::path::Path) -> Option<PathBuf> {
    let text = fs::read_to_string(plist).ok()?;
    let key = text.find("<key>ProgramArguments</key>")?;
    let array = text[key..].find("<array>")? + key;
    let string_start = text[array..].find("<string>")? + array;
    let value_start = string_start + "<string>".len();
    let value_end = text[value_start..].find("</string>")? + value_start;
    Some(PathBuf::from(
        text[value_start..value_end]
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'"),
    ))
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

    #[tokio::test(flavor = "current_thread")]
    async fn start_requests_accessibility_before_launchctl() {
        let calls = RefCell::new(Vec::<String>::new());

        super::start_with_plan(
            || calls.borrow_mut().push("accessibility".to_string()),
            |_, _| {
                calls.borrow_mut().push("launchctl".to_string());
                Ok(())
            },
            || {
                calls.borrow_mut().push("health".to_string());
                async { Ok(()) }
            },
            || calls.borrow_mut().push("print".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(
            &*calls.borrow(),
            &["accessibility", "launchctl", "health", "print"]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn start_does_not_print_success_when_health_check_fails() {
        let calls = RefCell::new(Vec::<String>::new());

        let err = super::start_with_plan(
            || calls.borrow_mut().push("accessibility".to_string()),
            |_, _| {
                calls.borrow_mut().push("launchctl".to_string());
                Ok(())
            },
            || {
                calls.borrow_mut().push("health".to_string());
                async { Err(anyhow!("daemon did not stay running")) }
            },
            || calls.borrow_mut().push("print".to_string()),
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("daemon did not stay running"));
        assert_eq!(&*calls.borrow(), &["accessibility", "launchctl", "health"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn install_requests_accessibility_before_launchctl() {
        let calls = RefCell::new(Vec::<String>::new());
        let plist = std::path::PathBuf::from("/tmp/com.hza2002.shuohua.plist");

        super::install_with_plan(
            || calls.borrow_mut().push("accessibility".to_string()),
            || Ok(plist.clone()),
            |_, action| {
                calls.borrow_mut().push(action.to_string());
                Ok(())
            },
            || {
                calls.borrow_mut().push("health".to_string());
                async { Ok(()) }
            },
            |_| calls.borrow_mut().push("print".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(
            &*calls.borrow(),
            &[
                "accessibility",
                "cli.service.action_uninstall",
                "cli.service.action_install",
                "cli.service.action_start",
                "health",
                "print"
            ]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_for_daemon_ready_requires_two_successful_probes() {
        let probes = RefCell::new(
            vec![
                Ok(Some("daemon: running pid=1".to_string())),
                Ok(Some("daemon: running pid=1".to_string())),
            ]
            .into_iter(),
        );

        super::wait_for_daemon_ready_with(|| {
            let next = probes.borrow_mut().next().unwrap();
            async move { next }
        })
        .await
        .unwrap();
    }

    #[test]
    fn ensure_accessibility_trust_skips_prompt_when_already_trusted() {
        let calls = RefCell::new(Vec::<String>::new());

        super::ensure_accessibility_trust(
            || {
                calls.borrow_mut().push("check".to_string());
                true
            },
            || calls.borrow_mut().push("prompt".to_string()),
        );

        assert_eq!(&*calls.borrow(), &["check"]);
    }

    #[test]
    fn ensure_accessibility_trust_prompts_when_not_trusted() {
        let calls = RefCell::new(Vec::<String>::new());

        super::ensure_accessibility_trust(
            || {
                calls.borrow_mut().push("check".to_string());
                false
            },
            || calls.borrow_mut().push("prompt".to_string()),
        );

        assert_eq!(&*calls.borrow(), &["check", "prompt"]);
    }

    #[test]
    fn launchd_accessibility_diagnostic_detects_event_tap_failure() {
        crate::i18n::init("en-US");
        let log = "\
2026-06-30T01:49:54.045+08:00  INFO daemon ready uds=/tmp/shuohua-501.sock trigger=F16
2026-06-30T01:49:54.049+08:00 ERROR hotkey event tap exited error=CGEventTapCreate failed. Default-mode taps require Accessibility permission
";

        let diagnostic = super::launchd_accessibility_diagnostic(
            log,
            std::path::Path::new("/usr/local/bin/shuo"),
        )
        .unwrap();

        assert!(diagnostic.contains("Accessibility"), "{diagnostic}");
        assert!(diagnostic.contains("/usr/local/bin/shuo"), "{diagnostic}");
    }

    #[test]
    fn launchd_accessibility_diagnostic_ignores_unrelated_logs() {
        let diagnostic = super::launchd_accessibility_diagnostic(
            "2026-06-30T01:49:54Z ERROR something else",
            std::path::Path::new("/usr/local/bin/shuo"),
        );

        assert!(diagnostic.is_none());
    }

    #[test]
    fn print_launchd_diagnostics_reports_drift_even_when_daemon_running() {
        crate::i18n::init("en-US");
        let mut out = Vec::new();
        let plist = std::path::Path::new("/Users/u/Library/LaunchAgents/com.hza2002.shuohua.plist");
        let finding = crate::install::DriftFinding::PathFirstNotPreferred {
            path_first: std::path::PathBuf::from("/usr/local/bin/shuo"),
            preferred: std::path::PathBuf::from("/Users/u/.local/bin/shuo"),
        };

        super::write_launchd_diagnostics(&mut out, plist, None, &[finding]).unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("launchd.plist: installed"), "{text}");
        assert!(text.contains("install.drift"), "{text}");
    }

    #[test]
    fn plist_program_argument_reads_first_program_argument() {
        let dir = std::env::temp_dir().join(format!("shuohua-plist-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let plist = dir.join("com.hza2002.shuohua.plist");
        std::fs::write(
            &plist,
            r#"
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.hza2002.shuohua</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/shuo</string>
    <string>--daemon</string>
  </array>
</dict>
</plist>
"#,
        )
        .unwrap();

        assert_eq!(
            super::plist_program_argument(&plist).unwrap(),
            std::path::PathBuf::from("/usr/local/bin/shuo")
        );

        let _ = std::fs::remove_dir_all(dir);
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

    #[tokio::test(flavor = "current_thread")]
    async fn stop_prints_only_after_pid_exit_is_confirmed() {
        let calls = RefCell::new(Vec::new());

        super::stop_with(
            || async {
                calls.borrow_mut().push("request");
                Ok(Some(42))
            },
            |pid| {
                assert_eq!(pid, 42);
                calls.borrow_mut().push("wait");
                Ok(())
            },
            || calls.borrow_mut().push("print"),
            || calls.borrow_mut().push("not-running"),
        )
        .await
        .unwrap();

        assert_eq!(*calls.borrow(), ["request", "wait", "print"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stop_is_ok_when_daemon_is_not_running() {
        let calls = RefCell::new(Vec::new());

        super::stop_with(
            || async {
                calls.borrow_mut().push("request");
                Ok(None)
            },
            |_| {
                calls.borrow_mut().push("wait");
                Ok(())
            },
            || calls.borrow_mut().push("print"),
            || calls.borrow_mut().push("not-running"),
        )
        .await
        .unwrap();

        assert_eq!(*calls.borrow(), ["request", "not-running"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restart_propagates_stop_error_without_starting() {
        let starts = Cell::new(0);

        let error = super::restart_with(
            || async { Err(anyhow!("stop failed")) },
            || {
                starts.set(starts.get() + 1);
                async { Ok(()) }
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
                async { Ok(()) }
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
