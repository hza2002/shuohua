#[cfg(unix)]
mod imp {
    use anyhow::{Context, Result};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    pub(crate) type Pid = libc::pid_t;

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

    pub(crate) fn process_exists(pid: Pid) -> Result<bool> {
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
}

#[cfg(windows)]
mod imp {
    use anyhow::{Context, Result};
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, HANDLE, WAIT_ABANDONED,
        WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Threading::{
        CreateMutexW, OpenProcess, ReleaseMutex, WaitForSingleObject,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    pub(crate) type Pid = u32;

    pub(crate) struct DaemonLockGuard(Handle);

    pub(crate) fn acquire_daemon_lock() -> Result<DaemonLockGuard> {
        let identity = crate::windows_identity::WindowsSessionIdentity::current()
            .context("resolve Windows daemon identity")?;
        let lock_name = scoped_lock_name(&identity.scoped_name_suffix());
        let mut attrs =
            crate::windows_identity::SecurityAttributes::for_current_user_ipc(&identity)
                .context("create Windows daemon mutex security descriptor")?;
        let name = wide_null(&lock_name);
        let handle = unsafe { CreateMutexW(attrs.as_mut_ptr().cast(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("create Windows daemon mutex {lock_name}"));
        }
        let handle = Handle(handle);
        match wait_for_mutex(handle.raw()) {
            Ok(MutexAcquire::Acquired) => Ok(DaemonLockGuard(handle)),
            Ok(MutexAcquire::AbandonedRecovered) => {
                tracing::warn!("recovered abandoned Windows daemon mutex");
                Ok(DaemonLockGuard(handle))
            }
            Err(error) => {
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    anyhow::bail!("another shuo daemon is already starting or running");
                }
                Err(error).context("acquire Windows daemon mutex")
            }
        }
    }

    fn scoped_lock_name(scope: &str) -> String {
        format!("Local\\shuohua-daemon-{scope}")
    }

    #[derive(Debug)]
    enum MutexAcquire {
        Acquired,
        AbandonedRecovered,
    }

    fn wait_for_mutex(handle: HANDLE) -> std::io::Result<MutexAcquire> {
        wait_for_mutex_result(unsafe { WaitForSingleObject(handle, 0) })
    }

    fn wait_for_mutex_result(result: u32) -> std::io::Result<MutexAcquire> {
        match result {
            WAIT_OBJECT_0 => Ok(MutexAcquire::Acquired),
            WAIT_ABANDONED => Ok(MutexAcquire::AbandonedRecovered),
            WAIT_TIMEOUT => Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
            WAIT_FAILED => Err(std::io::Error::last_os_error()),
            code => Err(std::io::Error::other(format!(
                "unexpected WaitForSingleObject result {code}"
            ))),
        }
    }

    pub(crate) fn process_exists(pid: Pid) -> Result<bool> {
        debug_assert!(pid > 0);
        process_exists_with_open_result(open_process_for_probe(pid))
    }

    fn open_process_for_probe(pid: Pid) -> std::io::Result<Handle> {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(Handle(handle))
        }
    }

    fn process_exists_with_open_result(result: std::io::Result<Handle>) -> Result<bool> {
        match result {
            Ok(_) => Ok(true),
            Err(error) => match error.raw_os_error().map(|code| code as u32) {
                Some(ERROR_INVALID_PARAMETER) => Ok(false),
                Some(ERROR_ACCESS_DENIED) => Ok(true),
                _ => Err(error).context("check daemon process failed"),
            },
        }
    }

    impl Drop for DaemonLockGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = ReleaseMutex(self.0.raw());
            }
        }
    }

    struct Handle(HANDLE);

    impl Handle {
        fn raw(&self) -> HANDLE {
            self.0
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    fn wide_null(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn lock_name_uses_user_session_namespace() {
            assert_eq!(
                scoped_lock_name("abcdef0123456789abcdef01"),
                "Local\\shuohua-daemon-abcdef0123456789abcdef01"
            );
        }

        #[test]
        fn process_probe_treats_invalid_pid_as_exited_and_access_denied_as_running() {
            assert!(
                !process_exists_with_open_result(Err(std::io::Error::from_raw_os_error(
                    ERROR_INVALID_PARAMETER as i32
                )))
                .unwrap()
            );
            assert!(
                process_exists_with_open_result(Err(std::io::Error::from_raw_os_error(
                    ERROR_ACCESS_DENIED as i32
                )))
                .unwrap()
            );
            assert!(
                process_exists_with_open_result(Err(std::io::Error::from_raw_os_error(123)))
                    .is_err()
            );
        }

        #[test]
        fn mutex_wait_result_recovers_abandoned_mutex_explicitly() {
            assert!(matches!(
                wait_for_mutex_result(WAIT_OBJECT_0).unwrap(),
                MutexAcquire::Acquired
            ));
            assert!(matches!(
                wait_for_mutex_result(WAIT_ABANDONED).unwrap(),
                MutexAcquire::AbandonedRecovered
            ));
            assert_eq!(
                wait_for_mutex_result(WAIT_TIMEOUT).unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }
}

pub(crate) use imp::{acquire_daemon_lock, process_exists, Pid};
