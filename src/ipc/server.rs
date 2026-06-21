//! UDS daemon server.

use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{
    mpsc::{self, error::TrySendError},
    watch,
};
use tokio_util::sync::CancellationToken;

#[cfg(test)]
use crate::history::HistoryRecord;
use crate::history::{
    AnalyticsQuery, AudioDeleteResult, DeleteResult, HistoryEvent, HistoryPageResult, HistoryQuery,
    HistoryService, PipelineStepHistory, PipelineStepStatus,
};
use crate::ipc::protocol::{Command, Event, WireState, PROTO_VERSION};
use crate::state::{DaemonState, StateEvent, StateSnapshot, StateStore};

const CLIENT_QUEUE: usize = 256;
pub fn default_socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/shuohua-{uid}.sock"))
}

pub async fn bind_default() -> Result<UnixListener> {
    bind(default_socket_path()).await
}

pub async fn bind(path: impl AsRef<Path>) -> Result<UnixListener> {
    let path = path.as_ref();
    prepare_socket_path(path).await?;
    let listener =
        UnixListener::bind(path).with_context(|| format!("bind UDS {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod 0600 {}", path.display()))?;
    Ok(listener)
}

async fn prepare_socket_path(path: &Path) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.file_type().is_socket() {
            anyhow::bail!("refusing to use UDS path {}: not a socket", path.display());
        }
    }
    match UnixStream::connect(path).await {
        Ok(_) => anyhow::bail!(
            "another shuo daemon is already running at {}",
            path.display()
        ),
        Err(error) => match error.raw_os_error() {
            Some(libc::ENOENT) => Ok(()),
            Some(libc::ECONNREFUSED) => remove_stale_socket(path),
            _ => Err(error).with_context(|| format!("probe UDS {}", path.display())),
        },
    }
}

fn remove_stale_socket(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)
        .with_context(|| format!("inspect stale UDS {}", path.display()))?;
    if !meta.file_type().is_socket() {
        anyhow::bail!(
            "refusing to remove non-stale UDS path {}: not a socket",
            path.display()
        );
    }
    let uid = unsafe { libc::geteuid() };
    if meta.uid() != uid {
        anyhow::bail!(
            "refusing to remove stale UDS {} owned by uid {}, expected {}",
            path.display(),
            meta.uid(),
            uid
        );
    }
    fs::remove_file(path).with_context(|| format!("remove stale UDS {}", path.display()))
}

#[derive(Clone)]
pub struct ServerControl {
    pub reload: crate::reload::Handle,
    pub started_at: Instant,
    pub shutdown: watch::Sender<bool>,
}

pub async fn run(
    listener: UnixListener,
    state: StateStore,
    history: HistoryService,
    control: ServerControl,
) -> Result<()> {
    loop {
        let (stream, _) = listener.accept().await.context("accept UDS client")?;
        let state = state.clone();
        let history = history.clone();
        let control = control.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, state, history, control).await {
                tracing::debug!(error = ?e, "IPC client ended");
            }
        });
    }
}

async fn handle_client(
    stream: UnixStream,
    state: StateStore,
    history: HistoryService,
    control: ServerControl,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let (tx, mut rx) = mpsc::channel::<Event>(CLIENT_QUEUE);
    let client_cancel = CancellationToken::new();
    let forwarder_cancel = CancellationToken::new();

    let writer_cancel = client_cancel.clone();
    let writer_forwarder_cancel = forwarder_cancel.clone();
    let writer = tokio::spawn(async move {
        loop {
            let event = tokio::select! {
                _ = writer_cancel.cancelled() => break,
                event = rx.recv() => {
                    let Some(event) = event else { break };
                    event
                }
            };
            let line = match crate::ipc::protocol::encode_event(&event) {
                Ok(line) => line,
                Err(e) => {
                    tracing::error!(error = %e, "serialize IPC event failed");
                    continue;
                }
            };
            if writer.write_all(line.as_bytes()).await.is_err() {
                writer_cancel.cancel();
                writer_forwarder_cancel.cancel();
                break;
            }
        }
    });

    let mut subscribed = false;
    let mut state_forwarder = None;
    let mut history_forwarder = None;
    let mut graceful_close = false;
    loop {
        let line = tokio::select! {
            _ = client_cancel.cancelled() => break,
            line = lines.next_line() => {
                let Some(line) = line? else { break };
                line
            }
        };
        let command = match crate::ipc::protocol::decode_command(&line) {
            Ok(command) => command,
            Err(e) => {
                if !send_or_drop(
                    &tx,
                    Event::Error {
                        recording_id: None,
                        kind: "bad_command".to_string(),
                        msg: e.to_string(),
                    },
                ) {
                    client_cancel.cancel();
                    forwarder_cancel.cancel();
                    break;
                }
                continue;
            }
        };

        match command {
            Command::Subscribe if !subscribed => {
                subscribed = true;
                let (snapshot, rx) = state.subscribe_with_snapshot();
                if !send_or_drop(&tx, snapshot_event(snapshot)) {
                    client_cancel.cancel();
                    forwarder_cancel.cancel();
                    break;
                }
                state_forwarder = Some(spawn_state_forwarder(
                    rx,
                    tx.clone(),
                    forwarder_cancel.clone(),
                    client_cancel.clone(),
                ));
                history_forwarder = Some(spawn_history_forwarder(
                    history.subscribe(),
                    tx.clone(),
                    forwarder_cancel.clone(),
                    client_cancel.clone(),
                ));
            }
            Command::Subscribe => {}
            Command::GetHistory {
                limit,
                before,
                before_id,
                query,
            } => {
                let event = match history_query(limit, before, before_id, query) {
                    Ok(query) => history_response_event(history_page(history.clone(), query).await),
                    Err(error) => bad_command_event(error),
                };
                if !send_or_drop(&tx, event) {
                    client_cancel.cancel();
                    forwarder_cancel.cancel();
                    break;
                }
            }
            Command::GetHistoryStats => {
                let event = history_stats_event(history_stats(history.clone()).await);
                if !send_or_drop(&tx, event) {
                    client_cancel.cancel();
                    forwarder_cancel.cancel();
                    break;
                }
            }
            Command::GetHistoryAnalytics { period, anchor } => {
                let event = history_analytics_event(
                    history_analytics(history.clone(), AnalyticsQuery::new(period, anchor)).await,
                );
                if !send_or_drop(&tx, event) {
                    client_cancel.cancel();
                    forwarder_cancel.cancel();
                    break;
                }
            }
            Command::DeleteAudio { id } => {
                let event = audio_deleted_event(delete_audio(history.clone(), id).await);
                if !send_or_drop(&tx, event) {
                    client_cancel.cancel();
                    forwarder_cancel.cancel();
                    break;
                }
            }
            Command::DeleteHistory { id } => {
                let event = history_deleted_event(delete_history(history.clone(), id).await);
                if !send_or_drop(&tx, event) {
                    client_cancel.cancel();
                    forwarder_cancel.cancel();
                    break;
                }
            }
            Command::DaemonStatus => {
                let snapshot = state.snapshot();
                if !send_or_drop(
                    &tx,
                    Event::DaemonStatus {
                        pid: std::process::id(),
                        uptime_ms: control.started_at.elapsed().as_millis() as u64,
                        state: snapshot.state.into(),
                        recording_id: snapshot.recording_id,
                    },
                ) {
                    client_cancel.cancel();
                    forwarder_cancel.cancel();
                    break;
                }
            }
            Command::Shutdown => {
                let snapshot = state.snapshot();
                if !send_or_drop(
                    &tx,
                    Event::DaemonStatus {
                        pid: std::process::id(),
                        uptime_ms: control.started_at.elapsed().as_millis() as u64,
                        state: snapshot.state.into(),
                        recording_id: snapshot.recording_id,
                    },
                ) {
                    client_cancel.cancel();
                    forwarder_cancel.cancel();
                    break;
                }
                let _ = control.shutdown.send(true);
                graceful_close = true;
                break;
            }
            Command::ReloadConfig => match control.reload.reload_now() {
                Ok(()) => {
                    if !send_or_drop(
                        &tx,
                        Event::ConfigReloaded {
                            path: crate::config::default_path().display().to_string(),
                        },
                    ) {
                        client_cancel.cancel();
                        forwarder_cancel.cancel();
                        break;
                    }
                }
                Err(e) => {
                    if !send_or_drop(
                        &tx,
                        Event::Error {
                            recording_id: None,
                            kind: "reload_config_failed".to_string(),
                            msg: e.to_string(),
                        },
                    ) {
                        client_cancel.cancel();
                        forwarder_cancel.cancel();
                        break;
                    }
                }
            },
            Command::StartRecording | Command::StopRecording | Command::CancelRecording => {
                if !send_or_drop(
                    &tx,
                    Event::Error {
                        recording_id: None,
                        kind: "unsupported".to_string(),
                        msg: "recording control over IPC is not supported".to_string(),
                    },
                ) {
                    client_cancel.cancel();
                    forwarder_cancel.cancel();
                    break;
                }
            }
        }
    }

    if !graceful_close {
        client_cancel.cancel();
        forwarder_cancel.cancel();
    } else if state_forwarder.is_some() || history_forwarder.is_some() {
        forwarder_cancel.cancel();
    }
    if let Some(task) = state_forwarder {
        let _ = task.await;
    }
    if let Some(task) = history_forwarder {
        let _ = task.await;
    }
    drop(tx);
    let _ = writer.await;
    Ok(())
}

fn spawn_state_forwarder(
    mut rx: tokio::sync::broadcast::Receiver<StateEvent>,
    tx: mpsc::Sender<Event>,
    forwarder_cancel: CancellationToken,
    client_cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let event = tokio::select! {
                _ = forwarder_cancel.cancelled() => break,
                event = rx.recv() => event,
            };
            match event {
                Ok(event) => {
                    if !send_or_drop(&tx, event.into()) {
                        client_cancel.cancel();
                        forwarder_cancel.cancel();
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(lagged = n, "IPC client lagged");
                    if !send_or_drop(
                        &tx,
                        Event::Error {
                            recording_id: None,
                            kind: "lag".to_string(),
                            msg: format!("client lagged by {n} events"),
                        },
                    ) {
                        client_cancel.cancel();
                        forwarder_cancel.cancel();
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

fn spawn_history_forwarder(
    mut rx: tokio::sync::broadcast::Receiver<HistoryEvent>,
    tx: mpsc::Sender<Event>,
    forwarder_cancel: CancellationToken,
    client_cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let event = tokio::select! {
                _ = forwarder_cancel.cancelled() => break,
                event = rx.recv() => event,
            };
            match event {
                Ok(HistoryEvent::Appended(record)) => {
                    if !send_or_drop(&tx, Event::HistoryAppended { record }) {
                        client_cancel.cancel();
                        forwarder_cancel.cancel();
                        break;
                    }
                }
                Ok(HistoryEvent::Changed) => {
                    if !send_or_drop(&tx, Event::HistoryChanged) {
                        client_cancel.cancel();
                        forwarder_cancel.cancel();
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(lagged = n, "IPC history client lagged");
                    if !send_or_drop(
                        &tx,
                        Event::Error {
                            recording_id: None,
                            kind: "lag".to_string(),
                            msg: format!("client lagged by {n} history events"),
                        },
                    ) {
                        client_cancel.cancel();
                        forwarder_cancel.cancel();
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

static LAST_QUEUE_FULL_WARN: Mutex<Option<Instant>> = Mutex::new(None);

fn send_or_drop(tx: &mpsc::Sender<Event>, event: Event) -> bool {
    match tx.try_send(event) {
        Ok(()) => true,
        Err(TrySendError::Closed(_)) => false,
        Err(TrySendError::Full(_)) => {
            let mut last = LAST_QUEUE_FULL_WARN.lock().unwrap();
            let now = Instant::now();
            let should_warn = last.is_none_or(|t| now.duration_since(t) > Duration::from_secs(1));
            if should_warn {
                tracing::warn!("IPC client queue full");
                *last = Some(now);
            }
            false
        }
    }
}

fn snapshot_event(snapshot: StateSnapshot) -> Event {
    Event::Snapshot {
        proto_version: PROTO_VERSION,
        state: snapshot.state.into(),
        recording: snapshot.recording_id,
        started_at: snapshot.started_at.map(format_time),
        app: snapshot.app_bundle_id,
        app_name: snapshot.app_name,
        dur_ms: snapshot.dur_ms,
        words: snapshot.words,
        segments: snapshot.segments,
        partial: snapshot.partial,
    }
}

fn history_response_event(result: Result<HistoryPageResult>) -> Event {
    match result {
        Ok(page) => Event::History {
            records: page.records,
            matched: page.matched,
            stats: page.stats,
        },
        Err(e) => {
            tracing::warn!(error = ?e, "history read failed");
            Event::Error {
                recording_id: None,
                kind: "history_read".to_string(),
                msg: e.to_string(),
            }
        }
    }
}

fn history_stats_event(result: Result<crate::history::HistoryStatsSnapshot>) -> Event {
    match result {
        Ok(snapshot) => Event::HistoryStats { snapshot },
        Err(e) => history_error_event("history_stats", e),
    }
}

fn history_analytics_event(result: Result<crate::history::AnalyticsSnapshot>) -> Event {
    match result {
        Ok(snapshot) => Event::HistoryAnalytics { snapshot },
        Err(e) => history_error_event("history_analytics", e),
    }
}

fn audio_deleted_event(result: Result<AudioDeleteResult>) -> Event {
    match result {
        Ok(result) => Event::AudioDeleted {
            id: result.id,
            deleted: result.deleted,
        },
        Err(e) => history_error_event("audio_delete", e),
    }
}

fn history_deleted_event(result: Result<DeleteResult>) -> Event {
    match result {
        Ok(result) => Event::HistoryDeleted {
            id: result.id,
            record_deleted: result.record_deleted,
            audio_deleted: result.audio_deleted,
            audio_error: result.audio_error,
        },
        Err(e) => history_error_event("history_delete", e),
    }
}

fn history_error_event(kind: &str, error: anyhow::Error) -> Event {
    tracing::warn!(error = ?error, kind, "history command failed");
    Event::Error {
        recording_id: None,
        kind: kind.to_string(),
        msg: error.to_string(),
    }
}

fn bad_command_event(error: anyhow::Error) -> Event {
    Event::Error {
        recording_id: None,
        kind: "bad_command".to_string(),
        msg: error.to_string(),
    }
}

impl From<DaemonState> for WireState {
    fn from(value: DaemonState) -> Self {
        match value {
            DaemonState::Idle => WireState::Idle,
            DaemonState::Recording => WireState::Recording,
            DaemonState::Stopping => WireState::Stopping,
            DaemonState::Error => WireState::Error,
        }
    }
}

impl From<StateEvent> for Event {
    fn from(value: StateEvent) -> Self {
        match value {
            StateEvent::StateChanged {
                state,
                recording_id,
                started_at,
            } => Event::StateChanged {
                state: state.into(),
                recording_id,
                started_at: started_at.map(format_time),
            },
            StateEvent::AppChanged {
                bundle_id,
                app_name,
            } => Event::AppChanged {
                app: bundle_id,
                app_name,
            },
            StateEvent::StatsChanged { dur_ms, words } => Event::StatsChanged { dur_ms, words },
            StateEvent::Partial { recording_id, text } => Event::Partial { recording_id, text },
            StateEvent::Segment { recording_id, text } => Event::Segment { recording_id, text },
            StateEvent::PipelineStep { recording_id, step } => {
                pipeline_step_event(recording_id, step)
            }
            StateEvent::AudioMeter {
                recording_id,
                meter,
            } => Event::AudioMeter {
                recording_id,
                meter,
            },
            StateEvent::SessionMeta { recording_id, meta } => {
                Event::SessionMeta { recording_id, meta }
            }
            StateEvent::SessionPhase {
                recording_id,
                phase,
            } => Event::SessionPhase {
                recording_id,
                phase,
            },
            StateEvent::Error {
                recording_id,
                kind,
                msg,
            } => Event::Error {
                recording_id,
                kind,
                msg,
            },
        }
    }
}

fn format_time(value: time::OffsetDateTime) -> String {
    value
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| value.to_string())
}

fn pipeline_step_event(recording_id: String, step: PipelineStepHistory) -> Event {
    Event::PipelineStep {
        recording_id,
        name: step.name,
        status: step_status(step.status).to_string(),
        duration_ms: step.duration_ms,
        text: step.text,
        error: step.error,
    }
}

fn step_status(status: PipelineStepStatus) -> &'static str {
    match status {
        PipelineStepStatus::Ok => "ok",
        PipelineStepStatus::Error => "error",
        PipelineStepStatus::Timeout => "timeout",
        PipelineStepStatus::Skipped => "skipped",
    }
}

fn history_query(
    limit: usize,
    before: Option<String>,
    before_id: Option<String>,
    query: Option<String>,
) -> Result<HistoryQuery> {
    if before_id.is_some() && before.is_none() {
        anyhow::bail!("before_id requires before");
    }
    let before = before
        .map(|value| {
            time::OffsetDateTime::parse(&value, &time::format_description::well_known::Rfc3339)
                .with_context(|| format!("parse history before timestamp {value}"))
        })
        .transpose()?;
    Ok(HistoryQuery {
        limit,
        before,
        before_id,
        query,
    })
}

async fn history_page(history: HistoryService, query: HistoryQuery) -> Result<HistoryPageResult> {
    tokio::task::spawn_blocking(move || history.page(query))
        .await
        .context("join history page task")?
}

async fn history_stats(history: HistoryService) -> Result<crate::history::HistoryStatsSnapshot> {
    tokio::task::spawn_blocking(move || history.stats())
        .await
        .context("join history stats task")
}

async fn history_analytics(
    history: HistoryService,
    query: AnalyticsQuery,
) -> Result<crate::history::AnalyticsSnapshot> {
    tokio::task::spawn_blocking(move || history.analytics(query))
        .await
        .context("join history analytics task")?
}

async fn delete_audio(history: HistoryService, id: String) -> Result<AudioDeleteResult> {
    tokio::task::spawn_blocking(move || history.delete_audio(&id))
        .await
        .context("join audio delete task")?
}

async fn delete_history(history: HistoryService, id: String) -> Result<DeleteResult> {
    tokio::task::spawn_blocking(move || history.delete(&id))
        .await
        .context("join history delete task")?
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use time::macros::datetime;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    use super::*;
    use crate::history::{
        stats::tests_support::TestHooks, AsrHistory, HistoryService, HistoryStatus,
        PipelineStepStatus,
    };
    use crate::ipc::protocol::{decode_event, encode_command};

    fn test_reload_handle() -> crate::reload::Handle {
        let dir = std::env::temp_dir().join(format!("shuohua-reload-handle-{}", ulid::Ulid::new()));
        let root = dir.join("shuohua");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("config.toml"),
            r#"
[hotkey]
trigger = "f16"
"#,
        )
        .unwrap();
        let (_rx, handle) =
            crate::reload::watch_with_handle(root.join("config.toml"), None).unwrap();
        handle
    }

    fn test_shutdown_sender() -> tokio::sync::watch::Sender<bool> {
        let (tx, _rx) = tokio::sync::watch::channel(false);
        tx
    }

    #[tokio::test]
    async fn subscribe_fans_out_snapshot_and_live_events() {
        let path =
            std::env::temp_dir().join(format!("shuohua-ipc-test-{}.sock", ulid::Ulid::new()));
        let listener = bind(&path).await.unwrap();
        let state = StateStore::new();
        let cfg_path =
            std::env::temp_dir().join(format!("shuohua-ipc-test-{}.toml", ulid::Ulid::new()));
        std::fs::write(&cfg_path, "[hotkey]\ntrigger=\"f16\"\n").unwrap();
        let (_rx, reload) = crate::reload::watch_with_handle(cfg_path, None).unwrap();
        let history = HistoryService::with_dir(
            std::env::temp_dir().join(format!("shuohua-ipc-history-{}", ulid::Ulid::new())),
        );
        let server = tokio::spawn(run(
            listener,
            state.clone(),
            history,
            ServerControl {
                reload,
                started_at: Instant::now(),
                shutdown: test_shutdown_sender(),
            },
        ));

        let mut a = TestClient::connect(&path).await;
        let mut b = TestClient::connect(&path).await;
        a.subscribe().await;
        b.subscribe().await;

        assert!(matches!(a.read_event().await, Event::Snapshot { .. }));
        assert!(matches!(b.read_event().await, Event::Snapshot { .. }));

        state.set_recording("01HXYZ".to_string(), time::OffsetDateTime::now_utc());
        state.segment("01HXYZ".to_string(), "hello".to_string());
        state.audio_meter(
            "01HXYZ".to_string(),
            crate::state::AudioMeter {
                rms: 0.25,
                peak: 0.75,
                clipped: false,
                vad_probability: Some(0.8),
                vad_speech: Some(true),
            },
        );

        assert!(matches!(a.read_event().await, Event::StateChanged { .. }));
        assert!(matches!(b.read_event().await, Event::StateChanged { .. }));
        assert_eq!(
            a.read_event().await,
            Event::Segment {
                recording_id: "01HXYZ".to_string(),
                text: "hello".to_string()
            }
        );
        assert_eq!(
            b.read_event().await,
            Event::Segment {
                recording_id: "01HXYZ".to_string(),
                text: "hello".to_string()
            }
        );
        assert_eq!(
            a.read_event().await,
            Event::AudioMeter {
                recording_id: "01HXYZ".to_string(),
                meter: crate::state::AudioMeter {
                    rms: 0.25,
                    peak: 0.75,
                    clipped: false,
                    vad_probability: Some(0.8),
                    vad_speech: Some(true),
                }
            }
        );

        server.abort();
        fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn state_forwarder_stops_when_client_disconnects() {
        let (tx, rx) = mpsc::channel::<Event>(1);
        let (state_tx, state_rx) = tokio::sync::broadcast::channel::<StateEvent>(1);
        let forwarder_cancel = CancellationToken::new();
        let client_cancel = CancellationToken::new();
        let task = spawn_state_forwarder(
            state_rx,
            tx.clone(),
            forwarder_cancel.clone(),
            client_cancel.clone(),
        );

        drop(rx);
        let _ = state_tx.send(StateEvent::StatsChanged {
            dur_ms: 1,
            words: 1,
        });

        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();

        assert!(forwarder_cancel.is_cancelled());
        assert!(client_cancel.is_cancelled());
    }

    #[tokio::test]
    async fn state_forwarder_stops_when_client_queue_is_full() {
        let (tx, _rx) = mpsc::channel::<Event>(1);
        tx.try_send(Event::StatsChanged {
            dur_ms: 0,
            words: 0,
        })
        .unwrap();
        let (state_tx, state_rx) = tokio::sync::broadcast::channel::<StateEvent>(1);
        let forwarder_cancel = CancellationToken::new();
        let client_cancel = CancellationToken::new();
        let task = spawn_state_forwarder(
            state_rx,
            tx.clone(),
            forwarder_cancel.clone(),
            client_cancel.clone(),
        );

        let _ = state_tx.send(StateEvent::StatsChanged {
            dur_ms: 1,
            words: 1,
        });

        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();

        assert!(forwarder_cancel.is_cancelled());
        assert!(client_cancel.is_cancelled());
    }

    #[tokio::test]
    async fn shutdown_command_acknowledges_and_requests_shutdown() {
        let sock = PathBuf::from(format!("/tmp/shuohua-ipc-{}.sock", ulid::Ulid::new()));
        let _ = fs::remove_file(&sock);
        let listener = bind(&sock).await.unwrap();
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        let control = ServerControl {
            reload: test_reload_handle(),
            started_at: Instant::now(),
            shutdown: shutdown_tx,
        };
        let history = HistoryService::with_dir(
            std::env::temp_dir().join(format!("shuohua-ipc-history-{}", ulid::Ulid::new())),
        );
        let server = tokio::spawn(run(listener, StateStore::new(), history, control));
        let mut client = crate::ipc::client::IpcClient::connect(&sock).await.unwrap();

        client.send(&Command::Shutdown).await.unwrap();
        let event = client.recv().await.unwrap().unwrap();

        assert!(matches!(event, Event::DaemonStatus { .. }));
        shutdown_rx.changed().await.unwrap();
        assert!(*shutdown_rx.borrow_and_update());
        server.abort();
        let _ = fs::remove_file(sock);
    }

    #[tokio::test]
    async fn shutdown_after_subscribe_closes_client_handler() {
        let sock = PathBuf::from(format!(
            "/tmp/shuohua-ipc-sub-shutdown-{}.sock",
            ulid::Ulid::new()
        ));
        let _ = fs::remove_file(&sock);
        let listener = bind(&sock).await.unwrap();
        let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
        let control = ServerControl {
            reload: test_reload_handle(),
            started_at: Instant::now(),
            shutdown: shutdown_tx,
        };
        let history = HistoryService::with_dir(
            std::env::temp_dir().join(format!("shuohua-ipc-history-{}", ulid::Ulid::new())),
        );
        let server = tokio::spawn(run(listener, StateStore::new(), history, control));
        let mut client = crate::ipc::client::IpcClient::connect(&sock).await.unwrap();

        client.send(&Command::Subscribe).await.unwrap();
        assert!(matches!(
            client.recv().await.unwrap().unwrap(),
            Event::Snapshot { .. }
        ));
        client.send(&Command::Shutdown).await.unwrap();
        assert!(matches!(
            client.recv().await.unwrap().unwrap(),
            Event::DaemonStatus { .. }
        ));

        tokio::time::timeout(std::time::Duration::from_millis(200), client.recv())
            .await
            .expect("subscribed shutdown connection should close")
            .unwrap();
        server.abort();
        let _ = fs::remove_file(sock);
    }

    #[tokio::test]
    async fn bind_rejects_live_socket_without_unlinking_it() {
        let sock = PathBuf::from(format!("/tmp/shuohua-ipc-live-{}.sock", ulid::Ulid::new()));
        let _ = fs::remove_file(&sock);
        let _listener = bind(&sock).await.unwrap();

        let error = bind(&sock).await.unwrap_err();

        assert!(error.to_string().contains("already running"), "{error:#}");
        UnixStream::connect(&sock)
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

        UnixStream::connect(&sock)
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

    #[test]
    fn state_error_maps_to_wire_error() {
        let event = Event::from(StateEvent::Error {
            recording_id: Some("01HXYZ".to_string()),
            kind: "history_append".to_string(),
            msg: "disk full".to_string(),
        });

        assert_eq!(
            event,
            Event::Error {
                recording_id: Some("01HXYZ".to_string()),
                kind: "history_append".to_string(),
                msg: "disk full".to_string(),
            }
        );
    }

    #[test]
    fn history_service_page_reads_monthly_files_newest_first() {
        let dir = std::env::temp_dir().join(format!("shuohua-ipc-history-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();

        write_history_record(
            &dir.join("2026-06.jsonl"),
            history_record("jun", datetime!(2026-06-20 12:00:00 UTC), "六月记录"),
        );
        write_history_record(
            &dir.join("2026-07.jsonl"),
            history_record("jul-a", datetime!(2026-07-03 12:00:00 UTC), "七月较早"),
        );
        write_history_record(
            &dir.join("2026-07.jsonl"),
            history_record("jul-b", datetime!(2026-07-04 12:00:00 UTC), "七月较晚"),
        );

        let service = HistoryService::with_dir(dir.clone());
        let records = service
            .page(HistoryQuery {
                limit: 2,
                ..HistoryQuery::default()
            })
            .unwrap();
        let ids: Vec<_> = records.iter().map(|record| record.id.as_str()).collect();
        assert_eq!(ids, vec!["jul-b", "jul-a"]);

        let records = service
            .page(HistoryQuery {
                limit: 10,
                before: Some(datetime!(2026-07-04 00:00:00 UTC)),
                query: Some("六月".to_string()),
                ..HistoryQuery::default()
            })
            .unwrap();
        let ids: Vec<_> = records.iter().map(|record| record.id.as_str()).collect();
        assert_eq!(ids, vec!["jun"]);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn history_service_page_clamps_large_limits() {
        let dir = std::env::temp_dir().join(format!("shuohua-ipc-history-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        for n in 0..600 {
            write_history_record(
                &dir.join("2026-06.jsonl"),
                history_record(
                    &format!("record-{n:03}"),
                    datetime!(2026-06-20 12:00:00 UTC) + time::Duration::seconds(n),
                    "text",
                ),
            );
        }

        let records = HistoryService::with_dir(dir.clone())
            .page(HistoryQuery {
                limit: usize::MAX,
                ..HistoryQuery::default()
            })
            .unwrap();

        assert_eq!(records.len(), 500);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn read_history_ignores_only_truncated_final_line() {
        let dir = std::env::temp_dir().join(format!("shuohua-ipc-history-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("2026-06.jsonl");
        write_history_record(
            &path,
            history_record("valid", datetime!(2026-06-20 12:00:00 UTC), "保留"),
        );
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(br#"{"version":1,"id":"truncated""#)
            .unwrap();

        let records = HistoryService::with_dir(dir.clone())
            .page(HistoryQuery {
                limit: 10,
                ..HistoryQuery::default()
            })
            .unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "valid");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn read_history_rejects_corrupt_complete_line() {
        let dir = std::env::temp_dir().join(format!("shuohua-ipc-history-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("2026-06.jsonl");
        fs::write(&path, "not-json\n").unwrap();

        let error = HistoryService::with_dir(dir.clone())
            .page(HistoryQuery {
                limit: 10,
                ..HistoryQuery::default()
            })
            .unwrap_err();

        assert!(
            error.to_string().contains("parse history line"),
            "{error:#}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn history_response_reports_read_errors() {
        let event = history_response_event(Err(anyhow::anyhow!(
            "parse history line in /tmp/history/2026-06.jsonl"
        )));

        match event {
            Event::Error {
                recording_id,
                kind,
                msg,
            } => {
                assert_eq!(recording_id, None);
                assert_eq!(kind, "history_read");
                assert!(msg.contains("parse history line in"), "{msg}");
            }
            event => panic!("expected history_read error, got {event:?}"),
        }
    }

    #[tokio::test]
    async fn before_id_without_before_maps_to_bad_command() {
        let sock = PathBuf::from(format!(
            "/tmp/shuohua-ipc-before-id-{}.sock",
            ulid::Ulid::new()
        ));
        let _ = fs::remove_file(&sock);
        let listener = bind(&sock).await.unwrap();
        let history = HistoryService::with_dir(
            std::env::temp_dir().join(format!("shuohua-ipc-history-{}", ulid::Ulid::new())),
        );
        let server = tokio::spawn(run(
            listener,
            StateStore::new(),
            history,
            ServerControl {
                reload: test_reload_handle(),
                started_at: Instant::now(),
                shutdown: test_shutdown_sender(),
            },
        ));
        let mut client = crate::ipc::client::IpcClient::connect(&sock).await.unwrap();

        client
            .send(&Command::GetHistory {
                limit: 10,
                before: None,
                before_id: Some("01HXYZ".to_string()),
                query: None,
            })
            .await
            .unwrap();

        match client.recv().await.unwrap().unwrap() {
            Event::Error { kind, msg, .. } => {
                assert_eq!(kind, "bad_command");
                assert!(msg.contains("before_id"), "{msg}");
            }
            event => panic!("expected bad_command error, got {event:?}"),
        }
        server.abort();
        let _ = fs::remove_file(sock);
    }

    #[tokio::test]
    async fn first_stats_request_initializes_history() {
        let sock = PathBuf::from(format!("/tmp/shuohua-ipc-stats-{}.sock", ulid::Ulid::new()));
        let _ = fs::remove_file(&sock);
        let dir = std::env::temp_dir().join(format!("shuohua-ipc-history-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        write_history_record(
            &dir.join("2026-06.jsonl"),
            history_record("one", datetime!(2026-06-20 12:00:00 UTC), "hello world"),
        );
        let scan_attempts = Arc::new(AtomicUsize::new(0));
        let history = HistoryService::with_test_hooks(
            dir.clone(),
            time::macros::offset!(+0),
            TestHooks::default().with_before_scan_attempt({
                let scan_attempts = Arc::clone(&scan_attempts);
                move || {
                    scan_attempts.fetch_add(1, Ordering::SeqCst);
                }
            }),
        );
        let listener = bind(&sock).await.unwrap();
        let server = tokio::spawn(run(
            listener,
            StateStore::new(),
            history,
            ServerControl {
                reload: test_reload_handle(),
                started_at: Instant::now(),
                shutdown: test_shutdown_sender(),
            },
        ));
        let mut client = crate::ipc::client::IpcClient::connect(&sock).await.unwrap();

        client.send(&Command::Subscribe).await.unwrap();
        assert!(matches!(
            client.recv().await.unwrap().unwrap(),
            Event::Snapshot { .. }
        ));
        assert_eq!(scan_attempts.load(Ordering::SeqCst), 0);

        client.send(&Command::GetHistoryStats).await.unwrap();

        match client.recv().await.unwrap().unwrap() {
            Event::HistoryStats { snapshot } => {
                assert_eq!(snapshot.total.records, 1);
                assert_eq!(snapshot.total.words, 2);
                assert!(scan_attempts.load(Ordering::SeqCst) > 0);
            }
            event => panic!("expected history stats, got {event:?}"),
        }
        server.abort();
        let _ = fs::remove_file(sock);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn subscribed_clients_receive_external_history_changed() {
        let path = PathBuf::from(format!("/tmp/sh-ipc-chg-{}.sock", ulid::Ulid::new()));
        let listener = bind(&path).await.unwrap();
        let state = StateStore::new();
        let dir = std::env::temp_dir().join(format!("shuohua-ipc-history-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        let history = HistoryService::with_dir(dir.clone());
        assert_eq!(history.stats().total.records, 0);
        let server = tokio::spawn(run(
            listener,
            state,
            history.clone(),
            ServerControl {
                reload: test_reload_handle(),
                started_at: Instant::now(),
                shutdown: test_shutdown_sender(),
            },
        ));
        let mut client = TestClient::connect(&path).await;
        client.subscribe().await;
        assert!(matches!(client.read_event().await, Event::Snapshot { .. }));

        let changed_path = dir.join("2026-06.jsonl");
        write_history_record(
            &changed_path,
            history_record(
                "01HXYZABCDEF0123456789AAA1",
                datetime!(2026-06-20 12:00:00 UTC),
                "external",
            ),
        );
        history.mark_history_paths_changed(&[changed_path]);

        assert_eq!(client.read_event().await, Event::HistoryChanged);
        server.abort();
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn subscribed_clients_receive_history_appended_record() {
        let path = PathBuf::from(format!("/tmp/sh-ipc-app-{}.sock", ulid::Ulid::new()));
        let listener = bind(&path).await.unwrap();
        let dir = std::env::temp_dir().join(format!("shuohua-ipc-history-{}", ulid::Ulid::new()));
        let history = HistoryService::with_dir(dir.clone());
        let server = tokio::spawn(run(
            listener,
            StateStore::new(),
            history.clone(),
            ServerControl {
                reload: test_reload_handle(),
                started_at: Instant::now(),
                shutdown: test_shutdown_sender(),
            },
        ));
        let mut client = TestClient::connect(&path).await;
        client.subscribe().await;
        assert!(matches!(client.read_event().await, Event::Snapshot { .. }));

        let record = history_record(
            "01HXYZABCDEF0123456789AAA1",
            datetime!(2026-06-20 12:00:00 UTC),
            "done",
        );
        history.append(record.clone()).unwrap();

        match client.read_event().await {
            Event::HistoryAppended {
                record: event_record,
            } => assert_eq!(*event_record, record),
            event => panic!("expected history_appended event, got {event:?}"),
        }
        server.abort();
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn delete_response_includes_record_deleted() {
        let sock = PathBuf::from(format!(
            "/tmp/shuohua-ipc-delete-{}.sock",
            ulid::Ulid::new()
        ));
        let _ = fs::remove_file(&sock);
        let dir = std::env::temp_dir().join(format!("shuohua-ipc-history-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        write_history_record(
            &dir.join("2026-06.jsonl"),
            history_record(
                "01HXYZABCDEF0123456789AAA1",
                datetime!(2026-06-20 12:00:00 UTC),
                "delete",
            ),
        );
        let history = HistoryService::with_dir(dir.clone());
        let listener = bind(&sock).await.unwrap();
        let server = tokio::spawn(run(
            listener,
            StateStore::new(),
            history,
            ServerControl {
                reload: test_reload_handle(),
                started_at: Instant::now(),
                shutdown: test_shutdown_sender(),
            },
        ));
        let mut client = crate::ipc::client::IpcClient::connect(&sock).await.unwrap();

        client
            .send(&Command::DeleteHistory {
                id: "01HXYZABCDEF0123456789AAA1".to_string(),
            })
            .await
            .unwrap();

        match client.recv().await.unwrap().unwrap() {
            Event::HistoryDeleted {
                id, record_deleted, ..
            } => {
                assert_eq!(id, "01HXYZABCDEF0123456789AAA1");
                assert!(record_deleted);
            }
            event => panic!("expected history deleted, got {event:?}"),
        }
        server.abort();
        let _ = fs::remove_file(sock);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn history_changed_and_delete_response_work_in_either_order() {
        let path = PathBuf::from(format!("/tmp/sh-ipc-del-{}.sock", ulid::Ulid::new()));
        let dir = std::env::temp_dir().join(format!("shuohua-ipc-history-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        write_history_record(
            &dir.join("2026-06.jsonl"),
            history_record(
                "01HXYZABCDEF0123456789AAA1",
                datetime!(2026-06-20 12:00:00 UTC),
                "delete",
            ),
        );
        let history = HistoryService::with_dir(dir.clone());
        let listener = bind(&path).await.unwrap();
        let server = tokio::spawn(run(
            listener,
            StateStore::new(),
            history,
            ServerControl {
                reload: test_reload_handle(),
                started_at: Instant::now(),
                shutdown: test_shutdown_sender(),
            },
        ));
        let mut client = TestClient::connect(&path).await;
        client.subscribe().await;
        assert!(matches!(client.read_event().await, Event::Snapshot { .. }));

        let line = encode_command(&Command::DeleteHistory {
            id: "01HXYZABCDEF0123456789AAA1".to_string(),
        })
        .unwrap();
        client.writer.write_all(line.as_bytes()).await.unwrap();

        let first = client.read_event().await;
        let second = client.read_event().await;
        let events = [first, second];
        assert!(events.iter().any(|event| matches!(
            event,
            Event::HistoryDeleted {
                id,
                record_deleted: true,
                ..
            } if id == "01HXYZABCDEF0123456789AAA1"
        )));
        assert!(events
            .iter()
            .any(|event| matches!(event, Event::HistoryChanged)));
        server.abort();
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(dir);
    }

    struct TestClient {
        lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
        writer: tokio::net::unix::OwnedWriteHalf,
    }

    impl TestClient {
        async fn connect(path: &Path) -> Self {
            let stream = UnixStream::connect(path).await.unwrap();
            let (reader, writer) = stream.into_split();
            Self {
                lines: BufReader::new(reader).lines(),
                writer,
            }
        }

        async fn subscribe(&mut self) {
            let line = encode_command(&Command::Subscribe).unwrap();
            self.writer.write_all(line.as_bytes()).await.unwrap();
        }

        async fn read_event(&mut self) -> Event {
            let line =
                tokio::time::timeout(std::time::Duration::from_secs(1), self.lines.next_line())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
            decode_event(&line).unwrap()
        }
    }

    fn write_history_record(path: &Path, record: HistoryRecord) {
        crate::history::store::append_record(path, &record).unwrap();
    }

    fn history_record(id: &str, started_at: time::OffsetDateTime, text: &str) -> HistoryRecord {
        HistoryRecord {
            version: 1,
            id: id.to_string(),
            started_at,
            ended_at: started_at + time::Duration::seconds(1),
            duration_ms: 1000,
            status: HistoryStatus::Submitted,
            app: Some("com.example.App".to_string()),
            text: text.to_string(),
            text_stats: crate::text_stats::compute(text),
            asr: AsrHistory {
                provider: "test".to_string(),
                text: text.to_string(),
                duration_ms: 1000,
                audio_ms: 1000,
                sessions: Vec::new(),
            },
            pipeline: vec![PipelineStepHistory {
                name: "test".to_string(),
                status: PipelineStepStatus::Ok,
                duration_ms: 1.0,
                text: None,
                error: None,
            }],
            error: None,
        }
    }
}
