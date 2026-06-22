use anyhow::{Context, Result};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

pub(crate) struct DaemonLockGuard(std::fs::File);

pub(crate) fn acquire_daemon_lock() -> Result<DaemonLockGuard> {
    acquire_daemon_lock_at(&default_lock_path())
}

fn acquire_daemon_lock_at(path: &std::path::Path) -> Result<DaemonLockGuard> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(path)
        .with_context(|| format!("open daemon lock {}", path.display()))?;
    let meta = file
        .metadata()
        .with_context(|| format!("inspect daemon lock {}", path.display()))?;
    let uid = unsafe { libc::geteuid() };
    if meta.uid() != uid {
        anyhow::bail!(
            "daemon lock {} is owned by uid {}, expected {}",
            path.display(),
            meta.uid(),
            uid
        );
    }
    if !meta.is_file() {
        anyhow::bail!("daemon lock {} is not a regular file", path.display());
    }
    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o600 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 0600 daemon lock {}", path.display()))?;
    }
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        anyhow::bail!("another shuo daemon is already starting or running");
    }
    Ok(DaemonLockGuard(file))
}

fn default_lock_path() -> std::path::PathBuf {
    let uid = unsafe { libc::getuid() };
    std::path::PathBuf::from(format!("/tmp/shuohua-{uid}.lock"))
}

impl Drop for DaemonLockGuard {
    fn drop(&mut self) {
        let _ = unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

pub(crate) fn process_exists(pid: libc::pid_t) -> Result<bool> {
    debug_assert!(pid > 0);
    let result = unsafe { libc::kill(pid, 0) };
    let errno = if result == -1 {
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    } else {
        0
    };
    process_exists_from_probe_result(result, errno)
}

fn process_exists_from_probe_result(result: libc::c_int, errno: libc::c_int) -> Result<bool> {
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

#[cfg(test)]
mod tests {
    #[test]
    fn default_lock_path_is_independent_of_state_dir_environment() {
        std::env::set_var("XDG_STATE_HOME", "/tmp/shuohua-state-a");
        let first = super::default_lock_path();
        std::env::set_var("XDG_STATE_HOME", "/tmp/shuohua-state-b");
        let second = super::default_lock_path();
        std::env::remove_var("XDG_STATE_HOME");

        assert_eq!(first, second);
        assert!(first.to_string_lossy().starts_with("/tmp/shuohua-"));
        assert!(first.to_string_lossy().ends_with(".lock"));
    }

    #[test]
    fn process_probe_treats_eperm_as_running_and_esrch_as_exited() {
        assert!(super::process_exists_from_probe_result(-1, libc::EPERM).unwrap());
        assert!(!super::process_exists_from_probe_result(-1, libc::ESRCH).unwrap());
        assert!(super::process_exists_from_probe_result(0, 0).unwrap());
        assert!(super::process_exists_from_probe_result(-1, libc::EIO).is_err());
    }
}
