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
    use std::pin::Pin;
    use std::task::{Context as TaskContext, Poll};
    use std::time::Duration;

    use anyhow::{Context, Result};
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
    };
    use tokio::time;

    use crate::windows_identity::{SecurityAttributes, WindowsSessionIdentity};

    const ERROR_PIPE_BUSY: i32 = 231;
    const PIPE_BUSY_MAX_ATTEMPTS: u32 = 20;
    const PIPE_BUSY_RETRY_DELAY: Duration = Duration::from_millis(50);

    pub type ReadHalf = tokio::io::ReadHalf<PipeStream>;
    pub type WriteHalf = tokio::io::WriteHalf<PipeStream>;

    #[derive(Debug)]
    pub struct Listener {
        next: tokio::sync::Mutex<NamedPipeServer>,
        path: PathBuf,
    }

    #[derive(Debug)]
    pub struct Stream {
        inner: PipeStream,
    }

    #[derive(Debug)]
    pub enum PipeStream {
        Client(NamedPipeClient),
        Server(NamedPipeServer),
    }

    impl Listener {
        pub async fn accept(&self) -> Result<Stream> {
            let mut guard = self.next.lock().await;
            guard.connect().await.context("accept Windows IPC client")?;
            let connected = std::mem::replace(
                &mut *guard,
                create_server(&self.path, false)
                    .context("create next Windows IPC pipe instance")?,
            );
            Ok(Stream {
                inner: PipeStream::Server(connected),
            })
        }
    }

    impl Stream {
        pub fn into_split(self) -> (ReadHalf, WriteHalf) {
            tokio::io::split(self.inner)
        }
    }

    impl AsyncRead for PipeStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut TaskContext<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            match &mut *self {
                Self::Client(client) => Pin::new(client).poll_read(cx, buf),
                Self::Server(server) => Pin::new(server).poll_read(cx, buf),
            }
        }
    }

    impl AsyncWrite for PipeStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut TaskContext<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            match &mut *self {
                Self::Client(client) => Pin::new(client).poll_write(cx, buf),
                Self::Server(server) => Pin::new(server).poll_write(cx, buf),
            }
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            cx: &mut TaskContext<'_>,
        ) -> Poll<std::io::Result<()>> {
            match &mut *self {
                Self::Client(client) => Pin::new(client).poll_flush(cx),
                Self::Server(server) => Pin::new(server).poll_flush(cx),
            }
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            cx: &mut TaskContext<'_>,
        ) -> Poll<std::io::Result<()>> {
            match &mut *self {
                Self::Client(client) => Pin::new(client).poll_shutdown(cx),
                Self::Server(server) => Pin::new(server).poll_shutdown(cx),
            }
        }
    }

    pub fn default_endpoint() -> PathBuf {
        match WindowsSessionIdentity::current() {
            Ok(identity) => scoped_endpoint(&identity.scoped_name_suffix()),
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    "falling back to process-scoped Windows IPC endpoint"
                );
                scoped_endpoint(&format!("fallback-{}", std::process::id()))
            }
        }
    }

    fn scoped_endpoint(scope: &str) -> PathBuf {
        PathBuf::from(format!(r"\\.\pipe\shuohua-{scope}"))
    }

    pub async fn connect(path: impl AsRef<Path>) -> Result<Stream> {
        let path = path.as_ref();
        let mut attempts = 1;
        loop {
            match ClientOptions::new().open(path.as_os_str()) {
                Ok(client) => {
                    return Ok(Stream {
                        inner: PipeStream::Client(client),
                    });
                }
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    let Some(delay) = pipe_busy_retry_delay(attempts) else {
                        return Err(error).with_context(|| {
                            format!("connect Windows IPC pipe {}", path.display())
                        });
                    };
                    attempts += 1;
                    time::sleep(delay).await;
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("connect Windows IPC pipe {}", path.display()));
                }
            }
        }
    }

    fn pipe_busy_retry_delay(attempt: u32) -> Option<Duration> {
        (attempt < PIPE_BUSY_MAX_ATTEMPTS).then_some(PIPE_BUSY_RETRY_DELAY)
    }

    pub async fn bind_default() -> Result<Listener> {
        bind(default_endpoint()).await
    }

    pub async fn bind(path: impl AsRef<Path>) -> Result<Listener> {
        let path = path.as_ref().to_path_buf();
        let server = create_server(&path, true)
            .with_context(|| format!("bind Windows IPC pipe {}", path.display()))?;
        Ok(Listener {
            next: tokio::sync::Mutex::new(server),
            path,
        })
    }

    fn create_server(path: &Path, first: bool) -> std::io::Result<NamedPipeServer> {
        let identity = WindowsSessionIdentity::current().map_err(std::io::Error::other)?;
        let mut attrs =
            SecurityAttributes::for_current_user_ipc(&identity).map_err(std::io::Error::other)?;
        create_server_with_security(path, first, attrs.as_mut_ptr())
    }

    fn create_server_with_security(
        path: &Path,
        first: bool,
        attrs: *mut std::ffi::c_void,
    ) -> std::io::Result<NamedPipeServer> {
        let mut options = ServerOptions::new();
        if first {
            options.first_pipe_instance(true);
        }
        unsafe { options.create_with_security_attributes_raw(path.as_os_str(), attrs) }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn scoped_endpoint_uses_pipe_namespace_and_hash_suffix() {
            assert_eq!(
                scoped_endpoint("abcdef0123456789abcdef01"),
                PathBuf::from(r"\\.\pipe\shuohua-abcdef0123456789abcdef01")
            );
        }

        #[test]
        fn default_endpoint_is_no_longer_global_product_name() {
            let endpoint = default_endpoint();

            assert!(endpoint.to_string_lossy().starts_with(r"\\.\pipe\shuohua-"));
            assert_ne!(endpoint, PathBuf::from(r"\\.\pipe\shuohua"));
        }

        #[test]
        fn pipe_busy_retry_policy_is_bounded_and_short() {
            assert_eq!(pipe_busy_retry_delay(1), Some(Duration::from_millis(50)));
            assert_eq!(
                pipe_busy_retry_delay(PIPE_BUSY_MAX_ATTEMPTS - 1),
                Some(Duration::from_millis(50))
            );
            assert_eq!(pipe_busy_retry_delay(PIPE_BUSY_MAX_ATTEMPTS), None);
        }
    }
}

#[cfg(test)]
pub use imp::bind;
pub use imp::{bind_default, connect, default_endpoint, Listener, ReadHalf, Stream, WriteHalf};
