use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde::Serialize;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::timeout;

const HELPER_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/apple_capture_helper"));
const SERVER_READY_TIMEOUT: Duration = Duration::from_secs(60);
const SERVER_START_TIMEOUT: Duration = Duration::from_secs(10);
const SERVER_STOP_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PCM_FRAME_SAMPLES: usize = 16_000;

pub(crate) struct AppleVpSource {
    helper_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CaptureSmokeResult {
    pub samples_in_first_frame: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CaptureLifecycleSmokeResult {
    pub first_start_ms: u64,
    pub second_start_ms: u64,
    pub first_frames: usize,
    pub second_frames: usize,
}

pub(crate) struct RunningAppleVpSource {
    pcm_rx: mpsc::Receiver<Result<Vec<i16>>>,
    stop: RunningStop,
    residual_after_stop: Vec<Vec<i16>>,
}

enum ServerRequest {
    Start {
        pcm_tx: mpsc::Sender<Result<Vec<i16>>>,
        reply: oneshot::Sender<Result<()>>,
    },
    Stop {
        reply: Option<oneshot::Sender<Result<Vec<Vec<i16>>>>>,
    },
}

enum RunningStop {
    Child(Child),
    Server {
        request_tx: mpsc::Sender<ServerRequest>,
        stopped: bool,
    },
}

impl AppleVpSource {
    #[allow(dead_code)]
    pub(crate) fn prepare_helper() -> anyhow::Result<Self> {
        Ok(Self {
            helper_path: ensure_helper_binary()?,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn helper_path(&self) -> &std::path::Path {
        &self.helper_path
    }

    #[allow(dead_code)]
    async fn start_capture_smoke(&self, duration_ms: u64) -> Result<RunningAppleVpSource> {
        start_helper_capture_smoke(self.helper_path(), duration_ms).await
    }

    pub(crate) async fn start(&self) -> Result<RunningAppleVpSource> {
        match reusable_server(self.helper_path().to_path_buf()).await {
            Ok(server) => match server.start_capture().await {
                Ok(running) => Ok(running),
                Err(error) => {
                    clear_reusable_server().await;
                    Err(error)
                }
            },
            Err(error) => Err(error),
        }
    }
}

#[derive(Clone)]
struct AppleCaptureServer {
    request_tx: mpsc::Sender<ServerRequest>,
}

impl AppleCaptureServer {
    async fn start_capture(&self) -> Result<RunningAppleVpSource> {
        let (pcm_tx, pcm_rx) = mpsc::channel(32);
        let (reply, reply_rx) = oneshot::channel();
        self.request_tx
            .send(ServerRequest::Start { pcm_tx, reply })
            .await
            .context("apple capture server unavailable")?;
        await_empty_server_reply(reply_rx, SERVER_START_TIMEOUT, "start").await?;
        Ok(RunningAppleVpSource {
            pcm_rx,
            stop: RunningStop::Server {
                request_tx: self.request_tx.clone(),
                stopped: false,
            },
            residual_after_stop: Vec::new(),
        })
    }
}

impl RunningAppleVpSource {
    pub(crate) async fn recv(&mut self) -> Result<Option<Vec<i16>>> {
        match self.pcm_rx.recv().await {
            Some(Ok(samples)) => Ok(Some(samples)),
            Some(Err(error)) => Err(error),
            None => Ok(None),
        }
    }

    pub(crate) fn request_stop(&mut self) {
        match &mut self.stop {
            RunningStop::Child(child) => {
                let _ = child.start_kill();
            }
            RunningStop::Server { .. } => {}
        }
    }

    pub(crate) async fn stop(&mut self) -> Result<()> {
        self.drain_after_stop().await.map(|_| ())
    }

    pub(crate) async fn drain_after_stop(&mut self) -> Result<Vec<Vec<i16>>> {
        match &mut self.stop {
            RunningStop::Child(child) => {
                let _ = child.kill().await;
                Ok(Vec::new())
            }
            RunningStop::Server {
                request_tx,
                stopped,
            } => {
                if *stopped {
                    return Ok(std::mem::take(&mut self.residual_after_stop));
                }
                *stopped = true;
                let (reply, rx) = oneshot::channel();
                if let Err(error) = request_tx
                    .send(ServerRequest::Stop { reply: Some(reply) })
                    .await
                    .context("apple capture server unavailable")
                {
                    clear_reusable_server().await;
                    return Err(error);
                }
                let queued = self.drain_queued_until_stop_reply(rx).await;
                match queued {
                    Ok(residual) => {
                        self.residual_after_stop = residual;
                        Ok(std::mem::take(&mut self.residual_after_stop))
                    }
                    Err(error) => {
                        clear_reusable_server().await;
                        Err(error)
                    }
                }
            }
        }
    }

    async fn drain_queued_until_stop_reply(
        &mut self,
        rx: oneshot::Receiver<Result<Vec<Vec<i16>>>>,
    ) -> Result<Vec<Vec<i16>>> {
        tokio::pin!(rx);
        let timeout = tokio::time::sleep(SERVER_STOP_TIMEOUT);
        tokio::pin!(timeout);
        let mut queued = Vec::new();
        let mut stop_reply = None;
        let mut pcm_closed = false;

        loop {
            tokio::select! {
                reply = &mut rx, if stop_reply.is_none() => {
                    stop_reply = Some(
                        reply
                            .context("apple capture server stop reply dropped")?
                    );
                }
                samples = self.pcm_rx.recv(), if stop_reply.is_none() && !pcm_closed => {
                    match samples {
                        Some(Ok(samples)) => queued.push(samples),
                        Some(Err(error)) => return Err(error),
                        None => pcm_closed = true,
                    }
                }
                _ = &mut timeout => {
                    anyhow::bail!("apple capture server stop timed out");
                }
            }

            if let Some(reply) = stop_reply.take() {
                let residual = reply?;
                while let Ok(samples) = self.pcm_rx.try_recv() {
                    queued.push(samples?);
                }
                queued.extend(residual);
                return Ok(queued);
            }
        }
    }
}

#[allow(dead_code)]
pub(crate) async fn capture_smoke(duration_ms: u64) -> Result<CaptureSmokeResult> {
    let (_, samples_in_first_frame) = spawn_helper_capture_smoke(duration_ms).await?;
    Ok(CaptureSmokeResult {
        samples_in_first_frame,
    })
}

#[allow(dead_code)]
pub(crate) async fn capture_lifecycle_smoke(
    duration_ms: u64,
    gap_ms: u64,
) -> Result<CaptureLifecycleSmokeResult> {
    let source = AppleVpSource::prepare_helper()?;
    run_helper_lifecycle_smoke(source.helper_path(), duration_ms, gap_ms).await
}

#[allow(dead_code)]
async fn spawn_helper_self_test() -> Result<HelperEvent> {
    let source = AppleVpSource::prepare_helper()?;
    run_helper_self_test(source.helper_path()).await
}

#[allow(dead_code)]
async fn run_helper_self_test(helper_path: &Path) -> Result<HelperEvent> {
    let mut child = Command::new(helper_path)
        .arg("--self-test")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawn apple capture helper {}", helper_path.display()))?;
    let stdout = child
        .stdout
        .take()
        .context("apple capture helper stdout unavailable")?;
    let mut lines = BufReader::new(stdout).lines();
    let line = lines
        .next_line()
        .await
        .context("read apple capture helper self-test")?
        .context("apple capture helper self-test emitted no event")?;
    let event = parse_helper_event(&line).context("parse apple capture helper self-test event")?;
    let status = child
        .wait()
        .await
        .context("wait for apple capture helper self-test")?;
    if !status.success() {
        anyhow::bail!("apple capture helper self-test exited with {status}");
    }
    Ok(event)
}

#[allow(dead_code)]
async fn spawn_helper_capture_smoke(duration_ms: u64) -> Result<(HelperEvent, usize)> {
    let source = AppleVpSource::prepare_helper()?;
    run_helper_capture_smoke(source.helper_path(), duration_ms).await
}

#[allow(dead_code)]
async fn run_helper_capture_smoke(
    helper_path: &Path,
    duration_ms: u64,
) -> Result<(HelperEvent, usize)> {
    let mut source = start_helper_capture_smoke(helper_path, duration_ms).await?;
    let first = source
        .recv()
        .await?
        .context("apple capture helper emitted no PCM frame")?;
    source
        .stop()
        .await
        .context("stop apple capture helper smoke")?;
    Ok((
        HelperEvent::Ready {
            sample_rate: 16_000,
            channels: 1,
        },
        first.len(),
    ))
}

static REUSABLE_SERVER: OnceLock<Arc<Mutex<Option<AppleCaptureServer>>>> = OnceLock::new();

async fn reusable_server(helper_path: PathBuf) -> Result<AppleCaptureServer> {
    let slot = REUSABLE_SERVER.get_or_init(|| Arc::new(Mutex::new(None)));
    let mut guard = slot.lock().await;
    if let Some(server) = guard.as_ref() {
        return Ok(server.clone());
    }
    let server = spawn_server(&helper_path).await?;
    *guard = Some(server.clone());
    Ok(server)
}

async fn clear_reusable_server() {
    if let Some(slot) = REUSABLE_SERVER.get() {
        *slot.lock().await = None;
    }
}

async fn spawn_server(helper_path: &Path) -> Result<AppleCaptureServer> {
    let mut child = Command::new(helper_path)
        .arg("--server")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| {
            format!(
                "spawn apple capture helper server {}",
                helper_path.display()
            )
        })?;
    let child_id = child.id();
    tracing::debug!(pid = child_id, "apple capture helper server spawned");
    let stdin = child
        .stdin
        .take()
        .context("apple capture helper server stdin unavailable")?;
    let stdout = child
        .stdout
        .take()
        .context("apple capture helper server stdout unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("apple capture helper server stderr unavailable")?;
    spawn_stderr_logger(stderr);

    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    match timeout(SERVER_READY_TIMEOUT, reader.read_line(&mut line)).await {
        Ok(result) => {
            result.context("read apple capture helper server ready event")?;
        }
        Err(_) => {
            let _ = child.kill().await;
            anyhow::bail!("apple capture helper server ready timed out");
        }
    }
    if line.is_empty() {
        anyhow::bail!("apple capture helper server emitted no ready event");
    }
    validate_server_ready_event(
        parse_helper_event(line.trim_end()).context("parse apple capture helper server ready")?,
    )?;

    let (request_tx, request_rx) = mpsc::channel(4);
    tokio::spawn(server_loop(child, child_id, stdin, reader, request_rx));
    Ok(AppleCaptureServer { request_tx })
}

async fn server_loop<R>(
    mut child: Child,
    child_id: Option<u32>,
    mut stdin: tokio::process::ChildStdin,
    mut reader: BufReader<R>,
    mut request_rx: mpsc::Receiver<ServerRequest>,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    while let Some(request) = request_rx.recv().await {
        match request {
            ServerRequest::Start { pcm_tx, reply } => {
                let result = async {
                    stdin
                        .write_all(&encode_server_command("start")?)
                        .await
                        .context("send apple capture server start")?;
                    read_ready_from_stream(&mut reader).await
                }
                .await;
                let started = result.is_ok();
                let reply_delivered = reply.send(result).is_ok();
                if started {
                    if reply_delivered {
                        if run_active_server_session(
                            &mut stdin,
                            &mut reader,
                            &mut request_rx,
                            pcm_tx,
                        )
                        .await
                        .is_err()
                        {
                            tracing::warn!(
                                pid = child_id,
                                "apple capture helper server session failed"
                            );
                            break;
                        }
                    } else {
                        let mut frame_reader = PcmFrameReader::new(&mut reader);
                        if stop_server_session(&mut stdin, &mut frame_reader)
                            .await
                            .is_err()
                        {
                            tracing::warn!(
                                pid = child_id,
                                "apple capture helper server cleanup after abandoned start failed"
                            );
                            break;
                        }
                    }
                }
            }
            ServerRequest::Stop { reply } => {
                if let Some(reply) = reply {
                    let _ = reply.send(Ok(Vec::new()));
                }
            }
        }
    }
    let _ = stdin
        .write_all(&encode_server_command("quit").unwrap_or_default())
        .await;
    let _ = child.kill().await;
    tracing::debug!(pid = child_id, "apple capture helper server exited");
}

async fn await_empty_server_reply(
    reply_rx: oneshot::Receiver<Result<()>>,
    timeout_duration: Duration,
    operation: &str,
) -> Result<()> {
    match timeout(timeout_duration, reply_rx).await {
        Ok(reply) => {
            reply.with_context(|| format!("apple capture server {operation} reply dropped"))?
        }
        Err(_) => anyhow::bail!("apple capture server {operation} timed out"),
    }
}

async fn run_active_server_session<R>(
    stdin: &mut tokio::process::ChildStdin,
    reader: &mut BufReader<R>,
    request_rx: &mut mpsc::Receiver<ServerRequest>,
    pcm_tx: mpsc::Sender<Result<Vec<i16>>>,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut frame_reader = PcmFrameReader::new(reader);
    loop {
        tokio::select! {
            request = request_rx.recv() => {
                match request {
                    Some(ServerRequest::Stop { reply }) => {
                        let result = stop_server_session(stdin, &mut frame_reader).await;
                        if let Some(reply) = reply {
                            let _ = reply.send(result);
                        }
                        return Ok(());
                    }
                    Some(ServerRequest::Start { reply, .. }) => {
                        let _ = reply.send(Err(anyhow::anyhow!(
                            "apple capture server is already recording"
                        )));
                    }
                    None => {
                        stdin
                            .write_all(&encode_server_command("stop")?)
                            .await
                            .context("send apple capture server stop after request channel closed")?;
                        let residual = read_until_stopped(&mut frame_reader).await?;
                        for samples in residual {
                            let _ = pcm_tx.send(Ok(samples)).await;
                        }
                        return Ok(());
                    }
                }
            }
            frame = frame_reader.read_one() => {
                match frame {
                    Ok(frame) if frame.is_last => return Ok(()),
                    Ok(frame) => {
                        let _ = pcm_tx.send(Ok(frame.samples)).await;
                    }
                    Err(error) => {
                        let _ = pcm_tx.send(Err(error.context("read apple capture server PCM"))).await;
                        return Err(anyhow::anyhow!("apple capture server PCM stream failed"));
                    }
                }
            }
        }
    }
}

async fn stop_server_session<R>(
    stdin: &mut tokio::process::ChildStdin,
    reader: &mut PcmFrameReader<'_, BufReader<R>>,
) -> Result<Vec<Vec<i16>>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    stdin
        .write_all(&encode_server_command("stop")?)
        .await
        .context("send apple capture server stop")?;
    read_until_stopped(reader).await
}

async fn read_ready_from_stream<R>(reader: &mut BufReader<R>) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .context("read apple capture server start ready")?;
    if line.is_empty() {
        anyhow::bail!("apple capture server closed before ready");
    }
    validate_ready_event(
        parse_helper_event(line.trim_end()).context("parse apple capture server start ready")?,
    )
}

async fn read_until_stopped<R>(reader: &mut PcmFrameReader<'_, R>) -> Result<Vec<Vec<i16>>>
where
    R: AsyncBufRead + AsyncRead + Unpin,
{
    let mut residual = Vec::new();
    loop {
        let frame = reader.read_one().await?;
        if frame.is_last {
            break;
        }
        residual.push(frame.samples);
    }
    let mut line = String::new();
    reader
        .reader_mut()
        .read_line(&mut line)
        .await
        .context("read apple capture server stopped event")?;
    if line.is_empty() {
        anyhow::bail!("apple capture server closed before stopped event");
    }
    match parse_helper_event(line.trim_end()).context("parse apple capture server stopped event")? {
        HelperEvent::Stopped => Ok(residual),
        HelperEvent::Error { message, code } => {
            let suffix = code.map(|code| format!(" ({code})")).unwrap_or_default();
            anyhow::bail!("apple capture helper error: {message}{suffix}")
        }
        other => anyhow::bail!("unexpected apple capture server event after stop: {other:?}"),
    }
}

async fn run_helper_lifecycle_smoke(
    helper_path: &Path,
    duration_ms: u64,
    gap_ms: u64,
) -> Result<CaptureLifecycleSmokeResult> {
    let mut child = Command::new(helper_path)
        .arg("--lifecycle-smoke-ms")
        .arg(duration_ms.to_string())
        .arg("--lifecycle-gap-ms")
        .arg(gap_ms.to_string())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawn apple capture helper {}", helper_path.display()))?;
    let stdout = child
        .stdout
        .take()
        .context("apple capture helper stdout unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("apple capture helper stderr unavailable")?;
    spawn_stderr_logger(stderr);

    let mut first_start_ms = None;
    let mut second_start_ms = None;
    let mut first_frames = None;
    let mut second_frames = None;
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .context("read apple capture helper lifecycle event")?
    {
        match parse_helper_event(&line).context("parse apple capture helper lifecycle event")? {
            HelperEvent::Lifecycle {
                phase,
                round,
                engine_start_ms,
                ..
            } if phase == "started" && round == Some(1) => {
                first_start_ms = engine_start_ms;
            }
            HelperEvent::Lifecycle {
                phase,
                round,
                engine_start_ms,
                ..
            } if phase == "started" && round == Some(2) => {
                second_start_ms = engine_start_ms;
            }
            HelperEvent::Lifecycle {
                phase,
                round,
                frames,
                ..
            } if phase == "stopped" && round == Some(1) => {
                first_frames = frames;
            }
            HelperEvent::Lifecycle {
                phase,
                round,
                frames,
                ..
            } if phase == "stopped" && round == Some(2) => {
                second_frames = frames;
            }
            HelperEvent::Error { message, code } => {
                let suffix = code.map(|code| format!(" ({code})")).unwrap_or_default();
                anyhow::bail!("apple capture helper error: {message}{suffix}");
            }
            _ => {}
        }
    }

    let status = child
        .wait()
        .await
        .context("wait for apple capture helper lifecycle smoke")?;
    if !status.success() {
        anyhow::bail!("apple capture helper lifecycle smoke exited with {status}");
    }
    Ok(CaptureLifecycleSmokeResult {
        first_start_ms: first_start_ms.context("missing first lifecycle start")?,
        second_start_ms: second_start_ms.context("missing second lifecycle start")?,
        first_frames: first_frames.context("missing first lifecycle stop")?,
        second_frames: second_frames.context("missing second lifecycle stop")?,
    })
}

async fn start_helper_capture_smoke(
    helper_path: &Path,
    duration_ms: u64,
) -> Result<RunningAppleVpSource> {
    start_helper_capture_with_args(
        helper_path,
        ["--capture-smoke-ms".to_string(), duration_ms.to_string()],
    )
    .await
}

async fn start_helper_capture_with_args(
    helper_path: &Path,
    args: impl IntoIterator<Item = String>,
) -> Result<RunningAppleVpSource> {
    let mut child = Command::new(helper_path)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawn apple capture helper {}", helper_path.display()))?;
    let stdout = child
        .stdout
        .take()
        .context("apple capture helper stdout unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("apple capture helper stderr unavailable")?;
    spawn_stderr_logger(stderr);
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .context("read apple capture helper ready event")?;
    if line.is_empty() {
        anyhow::bail!("apple capture helper emitted no ready event");
    }
    let event =
        parse_helper_event(line.trim_end()).context("parse apple capture helper ready event")?;
    validate_ready_event(event)?;

    let (pcm_tx, pcm_rx) = mpsc::channel(32);
    tokio::spawn(async move {
        loop {
            match read_one_pcm_frame(&mut reader).await {
                Ok(frame) if frame.is_last => return,
                Ok(frame) => {
                    if pcm_tx.send(Ok(frame.samples)).await.is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let _ = pcm_tx.send(Err(error)).await;
                    return;
                }
            }
        }
    });
    Ok(RunningAppleVpSource {
        pcm_rx,
        stop: RunningStop::Child(child),
        residual_after_stop: Vec::new(),
    })
}

fn validate_ready_event(event: HelperEvent) -> Result<()> {
    match event {
        HelperEvent::Ready {
            sample_rate: 16_000,
            channels: 1,
        } => Ok(()),
        HelperEvent::Ready {
            sample_rate,
            channels,
        } => anyhow::bail!(
            "unexpected apple capture helper format: {sample_rate} Hz, {channels} channels"
        ),
        HelperEvent::Error { message, code } => {
            let suffix = code.map(|code| format!(" ({code})")).unwrap_or_default();
            anyhow::bail!("apple capture helper error: {message}{suffix}")
        }
        HelperEvent::Lifecycle { phase, .. } => {
            anyhow::bail!("unexpected apple capture helper lifecycle event before ready: {phase}")
        }
        HelperEvent::ServerReady { .. } | HelperEvent::Stopped => {
            anyhow::bail!("unexpected apple capture helper server event before ready")
        }
    }
}

fn validate_server_ready_event(event: HelperEvent) -> Result<()> {
    match event {
        HelperEvent::ServerReady {
            sample_rate: 16_000,
            channels: 1,
        } => Ok(()),
        HelperEvent::ServerReady {
            sample_rate,
            channels,
        } => anyhow::bail!(
            "unexpected apple capture helper server format: {sample_rate} Hz, {channels} channels"
        ),
        HelperEvent::Error { message, code } => {
            let suffix = code.map(|code| format!(" ({code})")).unwrap_or_default();
            anyhow::bail!("apple capture helper error: {message}{suffix}")
        }
        other => {
            anyhow::bail!("unexpected apple capture helper event before server ready: {other:?}")
        }
    }
}

fn spawn_stderr_logger<R>(stderr: R)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    tracing::debug!(helper = "apple_capture_helper", message = %line)
                }
                Ok(None) => return,
                Err(error) => {
                    tracing::warn!(error = ?error, "read apple capture helper stderr failed");
                    return;
                }
            }
        }
    });
}

async fn read_one_pcm_frame<R>(reader: &mut R) -> Result<crate::platform::macos::helper::PcmFrame>
where
    R: AsyncRead + Unpin,
{
    PcmFrameReader::new(reader).read_one().await
}

struct PcmFrameReader<'a, R> {
    reader: &'a mut R,
    header: [u8; 5],
    header_len: usize,
    payload: Vec<u8>,
    payload_len: usize,
}

impl<'a, R> PcmFrameReader<'a, R>
where
    R: AsyncRead + Unpin,
{
    fn new(reader: &'a mut R) -> Self {
        Self {
            reader,
            header: [0; 5],
            header_len: 0,
            payload: Vec::new(),
            payload_len: 0,
        }
    }

    fn reader_mut(&mut self) -> &mut R {
        self.reader
    }

    async fn read_one(&mut self) -> Result<crate::platform::macos::helper::PcmFrame> {
        while self.header_len < self.header.len() {
            let read = self
                .reader
                .read(&mut self.header[self.header_len..])
                .await
                .context("read PCM frame header")?;
            if read == 0 {
                anyhow::bail!("read PCM frame header: early EOF");
            }
            self.header_len += read;
        }

        let sample_count = u32::from_le_bytes([
            self.header[1],
            self.header[2],
            self.header[3],
            self.header[4],
        ]) as usize;
        if sample_count > MAX_PCM_FRAME_SAMPLES {
            anyhow::bail!(
                "oversized PCM frame: {sample_count} samples exceeds {MAX_PCM_FRAME_SAMPLES}"
            );
        }

        let payload_len = sample_count * 2;
        if self.payload.len() != payload_len {
            self.payload.resize(payload_len, 0);
            self.payload_len = 0;
        }
        while self.payload_len < payload_len {
            let read = self
                .reader
                .read(&mut self.payload[self.payload_len..])
                .await
                .context("read PCM frame payload")?;
            if read == 0 {
                anyhow::bail!("read PCM frame payload: early EOF");
            }
            self.payload_len += read;
        }

        let mut frame = Vec::with_capacity(5 + payload_len);
        frame.extend_from_slice(&self.header);
        frame.extend_from_slice(&self.payload);
        self.header = [0; 5];
        self.header_len = 0;
        self.payload.clear();
        self.payload_len = 0;
        crate::platform::macos::helper::decode_pcm_frame(&frame)
    }
}

fn ensure_helper_binary() -> anyhow::Result<PathBuf> {
    let path = helper_cache_path()?;
    let lock_path = path.with_extension("lock");
    crate::platform::macos::helper::ensure_helper_binary_at(&path, &lock_path, HELPER_BYTES)
}

fn helper_cache_path() -> anyhow::Result<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg)
    } else {
        let home = std::env::var("HOME")?;
        PathBuf::from(home).join(".cache")
    };
    Ok(base.join("shuohua/apple_capture_helper"))
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[allow(dead_code)]
#[serde(tag = "event")]
enum HelperEvent {
    #[serde(rename = "ready")]
    Ready { sample_rate: u32, channels: u16 },
    #[serde(rename = "server_ready")]
    ServerReady { sample_rate: u32, channels: u16 },
    #[serde(rename = "stopped")]
    Stopped,
    #[serde(rename = "lifecycle")]
    Lifecycle {
        phase: String,
        #[serde(default)]
        round: Option<u8>,
        #[serde(default)]
        engine_start_ms: Option<u64>,
        #[serde(default)]
        frames: Option<usize>,
        #[serde(default)]
        duration_ms: Option<u64>,
    },
    #[serde(rename = "error")]
    Error {
        message: String,
        #[serde(default)]
        code: Option<String>,
    },
}

#[allow(dead_code)]
fn parse_helper_event(line: &str) -> Result<HelperEvent, serde_json::Error> {
    serde_json::from_str(line)
}

#[derive(Serialize)]
struct ServerCommand<'a> {
    cmd: &'a str,
}

fn encode_server_command(cmd: &str) -> Result<Vec<u8>> {
    let mut line = serde_json::to_vec(&ServerCommand { cmd })?;
    line.push(b'\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn parse_ready_event() {
        let event =
            parse_helper_event(r#"{"event":"ready","sample_rate":16000,"channels":1}"#).unwrap();

        assert_eq!(
            event,
            HelperEvent::Ready {
                sample_rate: 16_000,
                channels: 1
            }
        );
    }

    #[test]
    fn parse_error_event() {
        let event = parse_helper_event(
            r#"{"event":"error","message":"microphone denied","code":"tcc_denied"}"#,
        )
        .unwrap();

        assert_eq!(
            event,
            HelperEvent::Error {
                message: "microphone denied".to_string(),
                code: Some("tcc_denied".to_string())
            }
        );
    }

    #[test]
    fn parse_server_events() {
        let ready =
            parse_helper_event(r#"{"event":"server_ready","sample_rate":16000,"channels":1}"#)
                .unwrap();
        assert_eq!(
            ready,
            HelperEvent::ServerReady {
                sample_rate: 16_000,
                channels: 1
            }
        );

        let stopped = parse_helper_event(r#"{"event":"stopped"}"#).unwrap();
        assert_eq!(stopped, HelperEvent::Stopped);
    }

    #[test]
    fn encode_server_command_is_json_line() {
        let command = encode_server_command("start").unwrap();

        assert_eq!(
            command,
            br#"{"cmd":"start"}"#.iter().copied().chain([b'\n']).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn server_reply_timeout_reports_operation() {
        let (_reply, reply_rx) = oneshot::channel::<Result<()>>();

        let err = await_empty_server_reply(reply_rx, Duration::from_millis(1), "start")
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("apple capture server start timed out"),
            "{err:#}"
        );
    }

    #[tokio::test]
    async fn read_one_pcm_frame_rejects_oversized_frame() {
        let mut frame = vec![0];
        frame.extend_from_slice(&(MAX_PCM_FRAME_SAMPLES as u32 + 1).to_le_bytes());

        let err = read_one_pcm_frame(&mut &frame[..]).await.unwrap_err();

        assert!(err.to_string().contains("oversized PCM frame"), "{err:#}");
    }

    #[tokio::test]
    async fn pcm_frame_reader_preserves_partial_header_after_cancel() {
        let frame = crate::platform::macos::helper::encode_pcm_frame(&[7, 8, 9], false);
        let (mut tx, mut rx) = tokio::io::duplex(64);
        tx.write_all(&frame[..2]).await.unwrap();
        let mut reader = PcmFrameReader::new(&mut rx);

        tokio::select! {
            result = reader.read_one() => panic!("partial frame unexpectedly completed: {result:?}"),
            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
        }

        tx.write_all(&frame[2..]).await.unwrap();

        let parsed = reader.read_one().await.unwrap();
        assert_eq!(parsed.samples, vec![7, 8, 9]);
        assert!(!parsed.is_last);
    }

    #[tokio::test]
    async fn read_until_stopped_collects_residual_pcm() {
        let mut stream = Vec::new();
        stream.extend(crate::platform::macos::helper::encode_pcm_frame(
            &[1, 2, 3],
            false,
        ));
        stream.extend(crate::platform::macos::helper::encode_pcm_frame(
            &[4, 5],
            false,
        ));
        stream.extend(crate::platform::macos::helper::encode_pcm_frame(&[], true));
        stream.extend(br#"{"event":"stopped"}"#);
        stream.push(b'\n');
        let mut reader = BufReader::new(&stream[..]);
        let mut frame_reader = PcmFrameReader::new(&mut reader);

        let residual = read_until_stopped(&mut frame_reader).await.unwrap();

        assert_eq!(residual, vec![vec![1, 2, 3], vec![4, 5]]);
    }

    #[tokio::test]
    async fn drain_queued_until_stop_reply_preserves_queued_before_residual() {
        let (pcm_tx, pcm_rx) = mpsc::channel(2);
        let (reply_tx, reply_rx) = oneshot::channel();
        let mut source = RunningAppleVpSource {
            pcm_rx,
            stop: RunningStop::Server {
                request_tx: mpsc::channel(1).0,
                stopped: true,
            },
            residual_after_stop: Vec::new(),
        };

        pcm_tx.send(Ok(vec![1])).await.unwrap();
        pcm_tx.send(Ok(vec![2])).await.unwrap();
        tokio::spawn(async move {
            let _ = reply_tx.send(Ok(vec![vec![3]]));
        });

        let drained = source
            .drain_queued_until_stop_reply(reply_rx)
            .await
            .unwrap();

        assert_eq!(drained, vec![vec![1], vec![2], vec![3]]);
    }

    #[test]
    fn parse_lifecycle_event() {
        let event = parse_helper_event(
            r#"{"event":"lifecycle","phase":"started","round":2,"engine_start_ms":41}"#,
        )
        .unwrap();

        assert_eq!(
            event,
            HelperEvent::Lifecycle {
                phase: "started".to_string(),
                round: Some(2),
                engine_start_ms: Some(41),
                frames: None,
                duration_ms: None,
            }
        );
    }

    #[tokio::test]
    async fn helper_self_test_reports_ready() {
        let event = spawn_helper_self_test().await.unwrap();

        assert_eq!(
            event,
            HelperEvent::Ready {
                sample_rate: 16_000,
                channels: 1
            }
        );
    }

    #[tokio::test]
    #[ignore = "touches microphone/TCC; run manually during Apple capture spike"]
    async fn helper_capture_smoke_reports_ready_and_pcm() {
        let (event, sample_count) = spawn_helper_capture_smoke(800).await.unwrap();

        assert_eq!(
            event,
            HelperEvent::Ready {
                sample_rate: 16_000,
                channels: 1
            }
        );
        assert!(sample_count > 0, "expected at least one PCM frame");
    }

    #[tokio::test]
    #[ignore = "touches microphone/TCC; run manually during Apple capture spike"]
    async fn apple_vp_source_capture_smoke_can_recv_and_stop() {
        let source = AppleVpSource::prepare_helper().unwrap();
        let mut running = source.start_capture_smoke(5_000).await.unwrap();
        let samples = running.recv().await.unwrap().unwrap();

        assert!(!samples.is_empty(), "expected PCM samples");
        running.stop().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "touches microphone/TCC; run manually during Apple capture server spike"]
    async fn apple_vp_source_reuses_server_across_two_recordings() {
        let source = AppleVpSource::prepare_helper().unwrap();

        let mut first = source.start().await.unwrap();
        let first_samples = first.recv().await.unwrap().unwrap();
        assert!(!first_samples.is_empty(), "expected first PCM samples");
        first.stop().await.unwrap();

        let mut second = source.start().await.unwrap();
        let second_samples = second.recv().await.unwrap().unwrap();
        assert!(!second_samples.is_empty(), "expected second PCM samples");
        second.stop().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "touches microphone/TCC; run manually to measure AVAudioEngine start/stop reuse"]
    async fn helper_lifecycle_smoke_reports_two_start_rounds() {
        let result = capture_lifecycle_smoke(800, 2_000).await.unwrap();

        eprintln!(
            "apple lifecycle smoke: first_start_ms={} second_start_ms={} first_frames={} second_frames={}",
            result.first_start_ms, result.second_start_ms, result.first_frames, result.second_frames
        );
        assert!(result.first_frames > 0, "expected first round frames");
        assert!(result.second_frames > 0, "expected second round frames");
    }
}
