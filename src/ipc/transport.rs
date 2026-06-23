#[cfg(unix)]
mod imp {
    use std::fs;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};
    use tokio::net::{UnixListener, UnixStream};

    pub type ReadHalf = tokio::net::unix::OwnedReadHalf;
    pub type WriteHalf = tokio::net::unix::OwnedWriteHalf;

    #[derive(Debug)]
    pub struct Listener {
        inner: UnixListener,
    }

    #[derive(Debug)]
    pub struct Stream {
        inner: UnixStream,
    }

    impl Listener {
        pub async fn accept(&self) -> Result<Stream> {
            let (stream, _) = self.inner.accept().await.context("accept IPC client")?;
            Ok(Stream { inner: stream })
        }
    }

    impl Stream {
        pub fn into_split(self) -> (ReadHalf, WriteHalf) {
            self.inner.into_split()
        }
    }

    pub fn default_endpoint() -> PathBuf {
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/tmp/shuohua-{uid}.sock"))
    }

    pub async fn connect(path: impl AsRef<Path>) -> Result<Stream> {
        let path = path.as_ref();
        let stream = UnixStream::connect(path)
            .await
            .with_context(|| format!("connect IPC {}", path.display()))?;
        Ok(Stream { inner: stream })
    }

    pub async fn bind_default() -> Result<Listener> {
        bind(default_endpoint()).await
    }

    pub async fn bind(path: impl AsRef<Path>) -> Result<Listener> {
        let path = path.as_ref();
        prepare_endpoint(path).await?;
        let listener =
            UnixListener::bind(path).with_context(|| format!("bind IPC {}", path.display()))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 0600 {}", path.display()))?;
        Ok(Listener { inner: listener })
    }

    async fn prepare_endpoint(path: &Path) -> Result<()> {
        if let Ok(meta) = fs::symlink_metadata(path) {
            if !meta.file_type().is_socket() {
                anyhow::bail!(
                    "refusing to use IPC endpoint {}: not a socket",
                    path.display()
                );
            }
        }
        match UnixStream::connect(path).await {
            Ok(_) => anyhow::bail!(
                "another shuo daemon is already running at {}",
                path.display()
            ),
            Err(error) => match error.raw_os_error() {
                Some(libc::ENOENT) => Ok(()),
                Some(libc::ECONNREFUSED) => remove_stale_endpoint(path),
                _ => Err(error).with_context(|| format!("probe IPC {}", path.display())),
            },
        }
    }

    fn remove_stale_endpoint(path: &Path) -> Result<()> {
        let meta = fs::symlink_metadata(path)
            .with_context(|| format!("inspect stale IPC endpoint {}", path.display()))?;
        if !meta.file_type().is_socket() {
            anyhow::bail!(
                "refusing to remove non-stale IPC endpoint {}: not a socket",
                path.display()
            );
        }
        let uid = unsafe { libc::geteuid() };
        if meta.uid() != uid {
            anyhow::bail!(
                "refusing to remove stale IPC endpoint {} owned by uid {}, expected {}",
                path.display(),
                meta.uid(),
                uid
            );
        }
        fs::remove_file(path)
            .with_context(|| format!("remove stale IPC endpoint {}", path.display()))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[tokio::test]
        async fn bind_rejects_live_socket_without_unlinking_it() {
            let sock = PathBuf::from(format!("/tmp/shuohua-ipc-live-{}.sock", ulid::Ulid::new()));
            let _ = fs::remove_file(&sock);
            let _listener = bind(&sock).await.unwrap();

            let error = bind(&sock).await.unwrap_err();

            assert!(error.to_string().contains("already running"), "{error:#}");
            connect(&sock)
                .await
                .expect("original live socket must remain reachable");
            let _ = fs::remove_file(sock);
        }

        #[tokio::test]
        async fn bind_recovers_stale_user_socket() {
            let sock = PathBuf::from(format!("/tmp/shuohua-ipc-stale-{}.sock", ulid::Ulid::new()));
            let _ = fs::remove_file(&sock);
            {
                let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
            }

            let _listener = bind(&sock).await.unwrap();

            connect(&sock)
                .await
                .expect("replacement socket should accept connections");
            let _ = fs::remove_file(sock);
        }

        #[tokio::test]
        async fn bind_rejects_regular_file_without_unlinking_it() {
            let sock = PathBuf::from(format!("/tmp/shuohua-ipc-file-{}.sock", ulid::Ulid::new()));
            let _ = fs::remove_file(&sock);
            fs::write(&sock, "not a socket").unwrap();

            let error = bind(&sock).await.unwrap_err();

            assert!(error.to_string().contains("not a socket"), "{error:#}");
            assert_eq!(fs::read_to_string(&sock).unwrap(), "not a socket");
            let _ = fs::remove_file(sock);
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::path::{Path, PathBuf};

    use anyhow::Result;
    use tokio::io::DuplexStream;

    pub type ReadHalf = tokio::io::ReadHalf<DuplexStream>;
    pub type WriteHalf = tokio::io::WriteHalf<DuplexStream>;

    #[derive(Debug)]
    pub struct Listener;

    #[derive(Debug)]
    pub struct Stream {
        inner: DuplexStream,
    }

    impl Listener {
        pub async fn accept(&self) -> Result<Stream> {
            anyhow::bail!("Windows IPC Named Pipe transport is not implemented")
        }
    }

    impl Stream {
        pub fn into_split(self) -> (ReadHalf, WriteHalf) {
            tokio::io::split(self.inner)
        }
    }

    pub fn default_endpoint() -> PathBuf {
        PathBuf::from(r"\\.\pipe\shuohua")
    }

    pub async fn connect(_path: impl AsRef<Path>) -> Result<Stream> {
        anyhow::bail!("Windows IPC Named Pipe transport is not implemented")
    }

    pub async fn bind_default() -> Result<Listener> {
        anyhow::bail!("Windows IPC Named Pipe transport is not implemented")
    }
}

#[cfg(test)]
pub use imp::bind;
pub use imp::{bind_default, connect, default_endpoint, Listener, ReadHalf, Stream, WriteHalf};
