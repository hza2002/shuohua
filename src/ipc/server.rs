//! UDS daemon server.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::ipc::protocol::{Command, Event, Stats, WireState, PROTO_VERSION};
use crate::state::history::{self, HistoryRecord, PipelineStepHistory, PipelineStepStatus};
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
    fs::remove_file(path).ok();
    let listener =
        UnixListener::bind(path).with_context(|| format!("bind UDS {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod 0600 {}", path.display()))?;
    Ok(listener)
}

#[derive(Clone)]
pub struct ServerControl {
    pub reload: crate::reload::Handle,
    pub started_at: Instant,
}

pub async fn run(listener: UnixListener, state: StateStore, control: ServerControl) -> Result<()> {
    loop {
        let (stream, _) = listener.accept().await.context("accept UDS client")?;
        let state = state.clone();
        let control = control.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, state, control).await {
                tracing::debug!(error = ?e, "IPC client ended");
            }
        });
    }
}

async fn handle_client(
    stream: UnixStream,
    state: StateStore,
    control: ServerControl,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let (tx, mut rx) = mpsc::channel::<Event>(CLIENT_QUEUE);

    let writer = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let line = match crate::ipc::protocol::encode_event(&event) {
                Ok(line) => line,
                Err(e) => {
                    tracing::error!(error = %e, "serialize IPC event failed");
                    continue;
                }
            };
            if writer.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    let mut subscribed = false;
    while let Some(line) = lines.next_line().await? {
        let command = match crate::ipc::protocol::decode_command(&line) {
            Ok(command) => command,
            Err(e) => {
                send_or_drop(
                    &tx,
                    Event::Error {
                        recording_id: None,
                        kind: "bad_command".to_string(),
                        msg: e.to_string(),
                    },
                );
                continue;
            }
        };

        match command {
            Command::Subscribe if !subscribed => {
                subscribed = true;
                let (snapshot, rx) = state.subscribe_with_snapshot();
                send_or_drop(&tx, snapshot_event(snapshot));
                spawn_state_forwarder(rx, tx.clone());
            }
            Command::Subscribe => {}
            Command::GetHistory {
                limit,
                before,
                query,
            } => {
                let records = read_history(limit, before.as_deref(), query.as_deref())
                    .unwrap_or_else(|e| {
                        tracing::warn!(error = ?e, "history read failed");
                        Vec::new()
                    });
                send_or_drop(&tx, Event::History { records });
            }
            Command::DaemonStatus => {
                let snapshot = state.snapshot();
                send_or_drop(
                    &tx,
                    Event::DaemonStatus {
                        pid: std::process::id(),
                        uptime_ms: control.started_at.elapsed().as_millis() as u64,
                        state: snapshot.state.into(),
                        recording_id: snapshot.recording_id,
                    },
                );
            }
            Command::ReloadConfig => match control.reload.reload_now() {
                Ok(()) => send_or_drop(
                    &tx,
                    Event::ConfigReloaded {
                        path: crate::config::default_path().display().to_string(),
                    },
                ),
                Err(e) => send_or_drop(
                    &tx,
                    Event::Error {
                        recording_id: None,
                        kind: "reload_config_failed".to_string(),
                        msg: e.to_string(),
                    },
                ),
            },
            Command::StartRecording | Command::StopRecording | Command::CancelRecording => {
                send_or_drop(
                    &tx,
                    Event::Error {
                        recording_id: None,
                        kind: "unsupported".to_string(),
                        msg: "command is not wired in M4".to_string(),
                    },
                );
            }
        }
    }

    drop(tx);
    let _ = writer.await;
    Ok(())
}

fn spawn_state_forwarder(
    mut rx: tokio::sync::broadcast::Receiver<StateEvent>,
    tx: mpsc::Sender<Event>,
) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => send_or_drop(&tx, event.into()),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(lagged = n, "IPC client lagged");
                    send_or_drop(
                        &tx,
                        Event::Error {
                            recording_id: None,
                            kind: "lag".to_string(),
                            msg: format!("client lagged by {n} events"),
                        },
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn send_or_drop(tx: &mpsc::Sender<Event>, event: Event) {
    if tx.try_send(event).is_err() {
        tracing::warn!("IPC client queue full");
        let _ = tx.try_send(Event::Error {
            recording_id: None,
            kind: "lag".to_string(),
            msg: "client queue full".to_string(),
        });
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
        stats: Stats::default(),
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
            StateEvent::HistoryAppended { record } => Event::HistoryAppended { record },
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

fn read_history(
    limit: usize,
    before: Option<&str>,
    query: Option<&str>,
) -> Result<Vec<HistoryRecord>> {
    read_history_from_dir(&history::history_dir(), limit, before, query)
}

fn read_history_from_dir(
    dir: &Path,
    limit: usize,
    before: Option<&str>,
    query: Option<&str>,
) -> Result<Vec<HistoryRecord>> {
    let before = before
        .map(|value| {
            time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        })
        .transpose()
        .context("parse history before timestamp")?;
    let query = query.map(str::to_lowercase);
    let mut records = Vec::new();
    for path in history::monthly_history_files_in_dir(dir)? {
        let body = fs::read_to_string(&path)
            .with_context(|| format!("read history {}", path.display()))?;
        for line in body.lines().filter(|line| !line.trim().is_empty()) {
            let record: HistoryRecord = serde_json::from_str(line)
                .with_context(|| format!("parse history line in {}", path.display()))?;
            if before.is_some_and(|before| record.started_at >= before) {
                continue;
            }
            if let Some(query) = query.as_deref() {
                if !history_matches(&record, query) {
                    continue;
                }
            }
            records.push(record);
        }
    }
    records.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    records.truncate(limit);
    Ok(records)
}

fn history_matches(record: &HistoryRecord, query: &str) -> bool {
    let haystack = [
        record.id.as_str(),
        record.app.as_deref().unwrap_or_default(),
        record.asr.text.as_str(),
        &record.text,
    ]
    .join("\n")
    .to_lowercase();
    haystack.contains(query)
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    use super::*;
    use crate::ipc::protocol::{decode_event, encode_command};
    use crate::state::history::{AsrHistory, HistoryStatus, PipelineStepStatus};

    #[tokio::test]
    async fn subscribe_fans_out_snapshot_and_live_events() {
        let path =
            std::env::temp_dir().join(format!("shuohua-ipc-test-{}.sock", ulid::Ulid::new()));
        let listener = bind(&path).await.unwrap();
        let state = StateStore::new();
        let cfg_path =
            std::env::temp_dir().join(format!("shuohua-ipc-test-{}.toml", ulid::Ulid::new()));
        std::fs::write(&cfg_path, "[hotkey]\ntrigger=\"f16\"\n").unwrap();
        let (_rx, reload) = crate::reload::watch_with_handle(cfg_path).unwrap();
        let server = tokio::spawn(run(
            listener,
            state.clone(),
            ServerControl {
                reload,
                started_at: Instant::now(),
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

    #[test]
    fn read_history_from_dir_reads_monthly_files_newest_first() {
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

        let records = read_history_from_dir(&dir, 2, None, None).unwrap();
        let ids: Vec<_> = records.iter().map(|record| record.id.as_str()).collect();
        assert_eq!(ids, vec!["jul-b", "jul-a"]);

        let records =
            read_history_from_dir(&dir, 10, Some("2026-07-04T00:00:00Z"), Some("六月")).unwrap();
        let ids: Vec<_> = records.iter().map(|record| record.id.as_str()).collect();
        assert_eq!(ids, vec!["jun"]);

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
        history::append_record(path, &record).unwrap();
    }

    fn history_record(id: &str, started_at: time::OffsetDateTime, text: &str) -> HistoryRecord {
        HistoryRecord {
            version: 2,
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
