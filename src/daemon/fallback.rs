use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

pub fn run_smart_fallback() -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create TUI runtime")?;
    let socket = crate::ipc::server::default_socket_path();
    match socket_status(&socket) {
        SocketStatus::AcceptsConnections => {
            match rt.block_on(wait_for_daemon_ready(None, Duration::from_secs(2)))? {
                WaitOutcome::Ready => {}
                WaitOutcome::PermissionPreflight(outcome) => {
                    anyhow::bail!(
                        "{:?} permission is required for {}; complete the system prompt, then run `shuo` again",
                        outcome.permission,
                        outcome.executable.display()
                    );
                }
            }
        }
        SocketStatus::Absent => {
            crate::daemon::permission_outcome::clear()?;
            let stderr = smart_fallback_log("smart.stderr.log")?;
            let stdout = smart_fallback_log("smart.stdout.log")?;
            let mut child = Command::new(std::env::current_exe().context("resolve current exe")?)
                .arg("--daemon")
                .stdin(Stdio::null())
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::from(stderr))
                .spawn()
                .context("spawn shuo --daemon")?;
            match rt.block_on(wait_for_daemon_ready(
                Some(&mut child),
                Duration::from_secs(2),
            )) {
                Ok(WaitOutcome::Ready) => {}
                Ok(WaitOutcome::PermissionPreflight(outcome)) => {
                    anyhow::bail!(
                        "{:?} permission is required for {}; this daemon will exit after the system prompt completes, then run `shuo` again",
                        outcome.permission,
                        outcome.executable.display()
                    );
                }
                Err(error) => {
                    terminate_child(&mut child);
                    return Err(error);
                }
            }
        }
        SocketStatus::Inaccessible(error) => {
            return Err(error).with_context(|| format!("connect UDS {}", socket.display()));
        }
    }

    rt.block_on(crate::tui::run())
}

fn terminate_child(child: &mut std::process::Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn smart_fallback_log(name: &str) -> Result<std::fs::File> {
    let dir = crate::paths::StateDirs::discover().root().to_path_buf();
    std::fs::create_dir_all(&dir).with_context(|| format!("create state dir {}", dir.display()))?;
    let path = dir.join(name);
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))
}

enum SocketStatus {
    AcceptsConnections,
    Absent,
    Inaccessible(std::io::Error),
}

fn socket_status_from_connect_result(result: std::io::Result<()>) -> SocketStatus {
    match result {
        Ok(()) => SocketStatus::AcceptsConnections,
        Err(error) => match error.raw_os_error() {
            Some(libc::ENOENT | libc::ECONNREFUSED) => SocketStatus::Absent,
            _ => SocketStatus::Inaccessible(error),
        },
    }
}

fn socket_status(path: &Path) -> SocketStatus {
    socket_status_from_connect_result(std::os::unix::net::UnixStream::connect(path).map(|_| ()))
}

async fn wait_for_daemon_ready(
    mut child: Option<&mut std::process::Child>,
    timeout: Duration,
) -> Result<WaitOutcome> {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if let Some(outcome) = crate::daemon::permission_outcome::read()? {
            return Ok(WaitOutcome::PermissionPreflight(outcome));
        }
        match daemon_ready().await? {
            Some(true) => return Ok(WaitOutcome::Ready),
            Some(false) => {}
            None => {}
        }
        if let Some(child) = child.as_deref_mut() {
            if child
                .try_wait()
                .context("check shuo --daemon status")?
                .is_some()
            {
                if let Some(outcome) = crate::daemon::permission_outcome::read()? {
                    return Ok(WaitOutcome::PermissionPreflight(outcome));
                }
                anyhow::bail!(
                    "daemon stopped before becoming ready; inspect the daemon log and run `shuo` again"
                );
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!(
        "daemon did not become ready within {:?}; inspect the daemon log",
        timeout
    )
}

enum WaitOutcome {
    Ready,
    PermissionPreflight(crate::daemon::permission_outcome::PermissionOutcome),
}

async fn daemon_ready() -> Result<Option<bool>> {
    let mut client =
        match crate::ipc::client::IpcClient::connect(crate::ipc::server::default_socket_path())
            .await
        {
            Ok(client) => client,
            Err(error) if crate::ipc::client::connect_error_is_absent(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
    client
        .send(&crate::ipc::protocol::Command::DaemonStatus)
        .await?;
    match client.recv().await? {
        Some(crate::ipc::protocol::Event::DaemonStatus { ready, .. }) => Ok(Some(ready)),
        Some(event) => anyhow::bail!("expected DaemonStatus, received {event:?}"),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(errno: libc::c_int) -> std::io::Result<()> {
        Err(std::io::Error::from_raw_os_error(errno))
    }

    #[test]
    fn socket_status_treats_only_missing_or_refused_as_absent() {
        assert!(matches!(
            socket_status_from_connect_result(Ok(())),
            SocketStatus::AcceptsConnections
        ));
        assert!(matches!(
            socket_status_from_connect_result(err(libc::ENOENT)),
            SocketStatus::Absent
        ));
        assert!(matches!(
            socket_status_from_connect_result(err(libc::ECONNREFUSED)),
            SocketStatus::Absent
        ));
        assert!(matches!(
            socket_status_from_connect_result(err(libc::EACCES)),
            SocketStatus::Inaccessible(_)
        ));
        assert!(matches!(
            socket_status_from_connect_result(err(libc::EPROTOTYPE)),
            SocketStatus::Inaccessible(_)
        ));
    }
}
