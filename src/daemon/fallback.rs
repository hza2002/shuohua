use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn run_smart_fallback() -> Result<()> {
    let socket = crate::ipc::server::default_socket_path();
    match socket_status(&socket) {
        SocketStatus::AcceptsConnections => {}
        SocketStatus::Absent => {
            let stderr = smart_fallback_log("smart.stderr.log")?;
            let stdout = smart_fallback_log("smart.stdout.log")?;
            let child = Command::new(std::env::current_exe().context("resolve current exe")?)
                .arg("--daemon")
                .stdin(Stdio::null())
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::from(stderr))
                .spawn()
                .context("spawn shuo --daemon")?;
            drop(child);
            wait_for_socket(&socket, Duration::from_secs(2))?;
        }
        SocketStatus::Inaccessible(error) => {
            return Err(error).with_context(|| format!("connect UDS {}", socket.display()));
        }
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create TUI runtime")?;
    rt.block_on(crate::tui::run())
}

fn smart_fallback_log(name: &str) -> Result<std::fs::File> {
    let dir = crate::state::history::state_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("create state dir {}", dir.display()))?;
    let path = dir.join(name);
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))
}

fn socket_accepts_connections(path: &Path) -> bool {
    matches!(socket_status(path), SocketStatus::AcceptsConnections)
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

fn wait_for_socket(path: &Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if socket_accepts_connections(path) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!("daemon did not accept UDS connections within {:?}", timeout)
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
