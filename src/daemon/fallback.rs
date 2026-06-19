use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn run_smart_fallback() -> Result<()> {
    let socket = crate::ipc::server::default_socket_path();
    if !socket_accepts_connections(&socket) {
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
    std::os::unix::net::UnixStream::connect(path).is_ok()
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
