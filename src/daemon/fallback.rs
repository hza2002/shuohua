use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn run_smart_fallback() -> Result<()> {
    let socket = crate::ipc::transport::default_endpoint();
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
            return Err(error).with_context(|| format!("connect IPC {}", socket.display()));
        }
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create TUI runtime")?;
    rt.block_on(crate::tui::run())
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
    connect_endpoint(path)
}

#[cfg(unix)]
fn connect_endpoint(path: &Path) -> SocketStatus {
    socket_status_from_connect_result(std::os::unix::net::UnixStream::connect(path).map(|_| ()))
}

#[cfg(windows)]
fn connect_endpoint(path: &Path) -> SocketStatus {
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create Windows IPC probe runtime")
        .and_then(|runtime| {
            runtime.block_on(async {
                let _stream = crate::ipc::transport::connect(path).await?;
                Ok(())
            })
        });
    socket_status_from_windows_connect_result(result)
}

#[cfg(windows)]
fn socket_status_from_windows_connect_result(result: Result<()>) -> SocketStatus {
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_PATH_NOT_FOUND: i32 = 3;
    const ERROR_PIPE_BUSY: i32 = 231;

    match result {
        Ok(()) => SocketStatus::AcceptsConnections,
        Err(error) => {
            let raw_os_error = error.chain().find_map(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .and_then(std::io::Error::raw_os_error)
            });
            match raw_os_error {
                Some(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) => SocketStatus::Absent,
                Some(ERROR_PIPE_BUSY) => SocketStatus::AcceptsConnections,
                _ => SocketStatus::Inaccessible(std::io::Error::other(error.to_string())),
            }
        }
    }
}

fn wait_for_socket(path: &Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if socket_accepts_connections(path) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!("daemon did not accept IPC connections within {:?}", timeout)
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

    #[cfg(windows)]
    #[test]
    fn windows_socket_status_treats_pipe_not_found_as_absent() {
        let error = anyhow::Error::new(std::io::Error::from_raw_os_error(2))
            .context("connect Windows IPC pipe");

        assert!(matches!(
            socket_status_from_windows_connect_result(Err(error)),
            SocketStatus::Absent
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_socket_status_treats_pipe_busy_as_present() {
        let error = anyhow::Error::new(std::io::Error::from_raw_os_error(231))
            .context("connect Windows IPC pipe");

        assert!(matches!(
            socket_status_from_windows_connect_result(Err(error)),
            SocketStatus::AcceptsConnections
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_socket_status_can_probe_live_named_pipe() {
        let path = std::path::PathBuf::from(format!(
            r"\\.\pipe\shuohua-fallback-probe-{}",
            ulid::Ulid::new()
        ));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _listener = runtime
            .block_on(crate::ipc::transport::bind(&path))
            .unwrap();

        assert!(matches!(
            socket_status(&path),
            SocketStatus::AcceptsConnections
        ));
    }
}
