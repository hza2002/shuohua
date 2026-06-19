use anyhow::{Context, Result};
use std::os::fd::AsRawFd;

pub(super) struct DaemonLock(std::fs::File);

impl DaemonLock {
    pub(super) fn acquire() -> Result<Self> {
        let dir = crate::state::history::state_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create state dir {}", dir.display()))?;
        let path = dir.join("daemon.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("open daemon lock {}", path.display()))?;
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            anyhow::bail!("another shuo daemon is already starting or running");
        }
        Ok(Self(file))
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        let _ = unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}
