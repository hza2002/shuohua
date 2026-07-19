//! Alibaba Cloud Model Studio Fun-ASR / Paraformer realtime provider.
//!
//! Official protocol:
//! - https://help.aliyun.com/en/model-studio/fun-asr-client-events
//! - https://help.aliyun.com/en/model-studio/fun-asr-server-events
//! - https://help.aliyun.com/en/model-studio/paraformer-client-events
//!
//! Audio is fixed to the shuohua canonical format: PCM 16 kHz, mono, signed
//! 16-bit little-endian. The provider buffers 100 ms per binary WebSocket
//! frame and flushes residual PCM before `finish-task`.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;
use toml::value::Table;

use crate::asr::types::*;
use crate::config::asr::aliyun::AliyunConfig;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const CHUNK_SAMPLES: usize = 1600; // 100 ms @ 16 kHz
const COMMAND_CAPACITY: usize = 64;
const EVENT_CAPACITY: usize = 64;
const IDLE_REUSE_LIMIT: Duration = Duration::from_secs(50);

pub struct AliyunProvider {
    config: AliyunConfig,
    idle: Arc<Mutex<Option<IdleConnection>>>,
}

struct IdleConnection {
    ws: Ws,
    since: Instant,
}

fn runtime_options(config: &AliyunConfig) -> crate::asr::providers::ProviderOptions {
    crate::asr::providers::ProviderOptions {
        local_vad: config.local_vad,
        open_timeout_ms: config.open_timeout_ms,
        finalize_timeout_ms: config.finalize_timeout_ms,
    }
}

impl AliyunProvider {
    pub(crate) fn new_from_path_with_overrides(
        path: &Path,
        overrides: Option<&Table>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            config: AliyunConfig::from_path_with_overrides(path, overrides)?,
            idle: Arc::new(Mutex::new(None)),
        })
    }

    pub fn options(&self) -> crate::asr::providers::ProviderOptions {
        runtime_options(&self.config)
    }

    pub async fn check_runtime(&self, ctx: SessionCtx) -> Result<(), AsrError> {
        let (mut session, mut events) = self.open(ctx).await?;
        session.send_pcm(&[0; CHUNK_SAMPLES], true).await?;
        let done = tokio::time::timeout(Duration::from_secs(12), async {
            while let Some(event) = events.recv().await {
                match event {
                    AsrEvent::Done => return Ok(()),
                    AsrEvent::Error { err } => return Err(err),
                    AsrEvent::Partial { .. }
                    | AsrEvent::Segment { .. }
                    | AsrEvent::Final { .. } => {}
                }
            }
            Err(AsrError::Protocol(
                "aliyun stream closed before done".into(),
            ))
        })
        .await
        .map_err(|_| AsrError::Timeout);
        let close_result = session.close().await;
        done??;
        close_result
    }
}

#[async_trait]
impl AsrProvider for AliyunProvider {
    fn name(&self) -> &str {
        "aliyun"
    }

    fn caps(&self) -> Caps {
        Caps {
            hotwords: self.config.supports_profile_context(),
            max_session_secs: None,
            multilingual: self.config.language_hints.is_empty(),
        }
    }

    async fn open(
        &self,
        ctx: SessionCtx,
    ) -> Result<(Box<dyn AsrSession>, mpsc::Receiver<AsrEvent>), AsrError> {
        let task_id = uuid::Uuid::new_v4().to_string();
        let run_task = build_run_task(&self.config, &ctx, &task_id);
        let ws = match self.take_idle().await {
            Some(mut reused) => match start_task(&mut reused, &run_task, &task_id).await {
                Ok(()) => reused,
                Err(error) => {
                    tracing::debug!(%error, "aliyun connection reuse failed; reconnecting once");
                    let mut fresh = self.connect().await?;
                    start_task(&mut fresh, &run_task, &task_id).await?;
                    fresh
                }
            },
            None => {
                let mut fresh = self.connect().await?;
                start_task(&mut fresh, &run_task, &task_id).await?;
                fresh
            }
        };

        let (cmd_tx, cmd_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (evt_tx, evt_rx) = mpsc::channel(EVENT_CAPACITY);
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let idle = self.idle.clone();
        tokio::spawn(async move {
            session_task(ws, task_id, cmd_rx, evt_tx, task_cancel, idle).await;
        });

        Ok((Box::new(AliyunSession { cmd_tx, cancel }), evt_rx))
    }
}

impl AliyunProvider {
    async fn connect(&self) -> Result<Ws, AsrError> {
        let endpoint = self.config.region.endpoint(&self.config.workspace_id);
        let mut request = endpoint
            .into_client_request()
            .map_err(|error| AsrError::Protocol(format!("build request: {error}")))?;
        let authorization = HeaderValue::from_str(&format!("Bearer {}", self.config.api_key))
            .map_err(|error| AsrError::Protocol(format!("authorization header: {error}")))?;
        request.headers_mut().insert("Authorization", authorization);
        tokio_tungstenite::connect_async(request)
            .await
            .map(|(ws, _)| ws)
            .map_err(connect_error)
    }

    async fn take_idle(&self) -> Option<Ws> {
        let idle = self.idle.lock().await.take()?;
        (idle.since.elapsed() < IDLE_REUSE_LIMIT).then_some(idle.ws)
    }
}

async fn start_task(ws: &mut Ws, run_task: &Value, task_id: &str) -> Result<(), AsrError> {
    ws.send(Message::Text(run_task.to_string().into()))
        .await
        .map_err(send_error)?;
    wait_task_started(ws, task_id).await
}

fn build_run_task(config: &AliyunConfig, ctx: &SessionCtx, task_id: &str) -> Value {
    let mut parameters = Map::from_iter([
        ("format".into(), json!("pcm")),
        ("sample_rate".into(), json!(16_000)),
        (
            "semantic_punctuation_enabled".into(),
            json!(config.semantic_punctuation_enabled),
        ),
        (
            "max_sentence_silence".into(),
            json!(config.max_sentence_silence),
        ),
        (
            "multi_threshold_mode_enabled".into(),
            json!(config.multi_threshold_mode_enabled),
        ),
        ("heartbeat".into(), json!(config.heartbeat)),
    ]);
    if !config.vocabulary_id.is_empty() {
        parameters.insert("vocabulary_id".into(), json!(config.vocabulary_id));
    }
    if !config.language_hints.is_empty() {
        parameters.insert("language_hints".into(), json!(config.language_hints));
    }
    // Fun-only：可选噪声阈值按需发送；不再有 Paraformer 参数族分支。
    if let Some(threshold) = config.speech_noise_threshold {
        parameters.insert("speech_noise_threshold".into(), json!(threshold));
    }

    let input = if config.supports_profile_context() {
        context_input(&ctx.hotwords)
    } else {
        json!({})
    };
    json!({
        "header": {
            "action": "run-task",
            "task_id": task_id,
            "streaming": "duplex"
        },
        "payload": {
            "task_group": "audio",
            "task": "asr",
            "function": "recognition",
            "model": config.model,
            "parameters": parameters,
            "input": input
        }
    })
}

fn context_input(hotwords: &[String]) -> Value {
    let mut text = String::new();
    for word in hotwords
        .iter()
        .map(|word| word.trim())
        .filter(|word| !word.is_empty())
    {
        let separator = if text.is_empty() { "" } else { "，" };
        let remaining = 400usize.saturating_sub(text.chars().count());
        if remaining == 0 {
            break;
        }
        let addition = format!("{separator}{word}");
        text.extend(addition.chars().take(remaining));
    }
    if text.is_empty() {
        json!({})
    } else {
        json!({
            "context": [{
                "role": "user",
                "content": [{"type": "input_text", "text": text}]
            }]
        })
    }
}

async fn wait_task_started(ws: &mut Ws, task_id: &str) -> Result<(), AsrError> {
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(text))) => {
                let response: ServerMessage = serde_json::from_str(&text)
                    .map_err(|error| AsrError::Protocol(format!("decode start event: {error}")))?;
                ensure_task_id(&response.header, task_id)?;
                match response.header.event.as_deref() {
                    Some("task-started") => return Ok(()),
                    Some("task-failed") => return Err(server_failure(&response.header)),
                    Some(event) => {
                        return Err(AsrError::Protocol(format!(
                            "unexpected event before task-started: {event}"
                        )))
                    }
                    None => {
                        return Err(AsrError::Protocol(
                            "start event missing header.event".into(),
                        ))
                    }
                }
            }
            Some(Ok(Message::Ping(payload))) => {
                ws.send(Message::Pong(payload)).await.map_err(send_error)?
            }
            Some(Ok(Message::Close(_))) | None => {
                return Err(AsrError::Network(
                    "aliyun websocket closed before task-started".into(),
                ))
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => return Err(AsrError::Network(error.to_string())),
        }
    }
}

enum PcmCommand {
    Audio { samples: Vec<i16>, is_last: bool },
}

struct AliyunSession {
    cmd_tx: mpsc::Sender<PcmCommand>,
    cancel: CancellationToken,
}

#[async_trait]
impl AsrSession for AliyunSession {
    async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<(), AsrError> {
        super::send_session_command(
            &self.cmd_tx,
            PcmCommand::Audio {
                samples: pcm.to_vec(),
                is_last,
            },
            "aliyun session task ended",
        )
        .await
    }

    async fn close(self: Box<Self>) -> Result<(), AsrError> {
        self.cancel.cancel();
        Ok(())
    }
}

impl Drop for AliyunSession {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn session_task(
    ws: Ws,
    task_id: String,
    mut cmd_rx: mpsc::Receiver<PcmCommand>,
    evt_tx: mpsc::Sender<AsrEvent>,
    cancel: CancellationToken,
    idle: Arc<Mutex<Option<IdleConnection>>>,
) {
    let (mut sink, mut stream) = ws.split();
    let started_at = Instant::now();
    let mut audio = Vec::with_capacity(CHUNK_SAMPLES * 2);
    let mut finishing = false;
    let mut partial_seq = 0;
    let mut emitted_sentences = BTreeSet::new();

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                return;
            }
            command = cmd_rx.recv(), if !finishing => {
                let Some(PcmCommand::Audio { samples, is_last }) = command else {
                    return;
                };
                audio.extend(samples);
                while audio.len() >= CHUNK_SAMPLES {
                    let chunk: Vec<i16> = audio.drain(..CHUNK_SAMPLES).collect();
                    if let Err(error) = super::bounded_session_io(
                        &cancel,
                        "aliyun websocket send",
                        sink.send(Message::Binary(pcm_bytes(&chunk).into())),
                    ).await {
                        emit_non_cancel_error(&evt_tx, error).await;
                        return;
                    }
                }
                if is_last {
                    if !audio.is_empty() {
                        if let Err(error) = super::bounded_session_io(
                            &cancel,
                            "aliyun websocket send",
                            sink.send(Message::Binary(pcm_bytes(&audio).into())),
                        ).await {
                            emit_non_cancel_error(&evt_tx, error).await;
                            return;
                        }
                    }
                    audio.clear();
                    let finish = json!({
                        "header": {"action": "finish-task", "task_id": task_id, "streaming": "duplex"},
                        "payload": {"input": {}}
                    });
                    if let Err(error) = super::bounded_session_io(
                        &cancel,
                        "aliyun websocket send",
                        sink.send(Message::Text(finish.to_string().into())),
                    ).await {
                        emit_non_cancel_error(&evt_tx, error).await;
                        return;
                    }
                    finishing = true;
                }
            }
            message = stream.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        match handle_server_message(
                            &text,
                            &task_id,
                            started_at,
                            &mut partial_seq,
                            &mut emitted_sentences,
                            &evt_tx,
                            ).await {
                            Ok(ServerAction::Continue) => {}
                            Ok(ServerAction::Done) => {
                                break;
                            }
                            Err(error) => {
                                emit_error(&evt_tx, error).await;
                                return;
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if let Err(error) = super::bounded_session_io(
                            &cancel,
                            "aliyun websocket pong",
                            sink.send(Message::Pong(payload)),
                        ).await {
                            emit_non_cancel_error(&evt_tx, error).await;
                            return;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        emit_error(&evt_tx, AsrError::Network("aliyun websocket closed before task-finished".into())).await;
                        return;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        emit_error(&evt_tx, AsrError::Network(error.to_string())).await;
                        return;
                    }
                }
            }
        }
    }

    if let Ok(ws) = sink.reunite(stream) {
        *idle.lock().await = Some(IdleConnection {
            ws,
            since: Instant::now(),
        });
    }
    let _ = evt_tx.send(AsrEvent::Done).await;
}

fn pcm_bytes(samples: &[i16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[derive(Debug, Deserialize)]
struct ServerMessage {
    header: ServerHeader,
    #[serde(default)]
    payload: ServerPayload,
}

#[derive(Debug, Deserialize)]
struct ServerHeader {
    #[serde(default)]
    task_id: String,
    event: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ServerPayload {
    #[serde(default)]
    output: ServerOutput,
}

#[derive(Debug, Default, Deserialize)]
struct ServerOutput {
    sentence: Option<Sentence>,
}

#[derive(Debug, Deserialize)]
struct Sentence {
    #[serde(default)]
    begin_time: u64,
    end_time: Option<u64>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    heartbeat: bool,
    #[serde(default)]
    sentence_end: bool,
    #[serde(default)]
    sentence_id: u64,
}

enum ServerAction {
    Continue,
    Done,
}

async fn handle_server_message(
    text: &str,
    task_id: &str,
    started_at: Instant,
    partial_seq: &mut u64,
    emitted_sentences: &mut BTreeSet<u64>,
    evt_tx: &mpsc::Sender<AsrEvent>,
) -> Result<ServerAction, AsrError> {
    let message: ServerMessage = serde_json::from_str(text)
        .map_err(|error| AsrError::Protocol(format!("decode event: {error}")))?;
    ensure_task_id(&message.header, task_id)?;
    match message.header.event.as_deref() {
        Some("result-generated") => {
            let Some(sentence) = message.payload.output.sentence else {
                return Ok(ServerAction::Continue);
            };
            if sentence.heartbeat || sentence.text.is_empty() {
                return Ok(ServerAction::Continue);
            }
            if sentence.sentence_end {
                if emitted_sentences.insert(sentence.sentence_id) {
                    let begin = started_at + Duration::from_millis(sentence.begin_time);
                    let end = started_at
                        + Duration::from_millis(sentence.end_time.unwrap_or(sentence.begin_time));
                    evt_tx
                        .send(AsrEvent::Segment {
                            text: sentence.text,
                            started_at: begin,
                            ended_at: end.max(begin),
                        })
                        .await
                        .map_err(|_| AsrError::Canceled)?;
                }
            } else {
                *partial_seq += 1;
                evt_tx
                    .send(AsrEvent::Partial {
                        text: sentence.text,
                        seq: *partial_seq,
                    })
                    .await
                    .map_err(|_| AsrError::Canceled)?;
            }
            Ok(ServerAction::Continue)
        }
        Some("task-finished") => Ok(ServerAction::Done),
        Some("task-failed") => Err(server_failure(&message.header)),
        Some("task-started") => Ok(ServerAction::Continue),
        Some(event) => Err(AsrError::Protocol(format!(
            "unknown aliyun event {event:?}"
        ))),
        None => Err(AsrError::Protocol("event missing header.event".into())),
    }
}

fn ensure_task_id(header: &ServerHeader, expected: &str) -> Result<(), AsrError> {
    if header.task_id == expected {
        Ok(())
    } else {
        Err(AsrError::Protocol(format!(
            "task_id mismatch: expected {expected}, got {}",
            header.task_id
        )))
    }
}

fn server_failure(header: &ServerHeader) -> AsrError {
    let code = header.error_code.as_deref().unwrap_or("UNKNOWN");
    let message = header.error_message.as_deref().unwrap_or("task failed");
    let detail = format!("{code}: {message}");
    if code.contains("AUTH") || code.contains("UNAUTHORIZED") {
        AsrError::Auth(detail)
    } else if code.contains("THROTT") || code.contains("QUOTA") {
        AsrError::Quota
    } else {
        AsrError::Server(detail)
    }
}

async fn emit_error(tx: &mpsc::Sender<AsrEvent>, error: AsrError) {
    let _ = tx.send(AsrEvent::Error { err: error }).await;
}

async fn emit_non_cancel_error(tx: &mpsc::Sender<AsrEvent>, error: AsrError) {
    if !matches!(error, AsrError::Canceled) {
        emit_error(tx, error).await;
    }
}

fn connect_error(error: tokio_tungstenite::tungstenite::Error) -> AsrError {
    use tokio_tungstenite::tungstenite::Error::Http;
    if let Http(response) = &error {
        return match response.status().as_u16() {
            401 | 403 => AsrError::Auth(format!("HTTP {}", response.status())),
            429 => AsrError::Quota,
            code => AsrError::Network(format!("HTTP {code}")),
        };
    }
    AsrError::Network(error.to_string())
}

fn send_error(error: tokio_tungstenite::tungstenite::Error) -> AsrError {
    AsrError::Network(format!("ws send: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::asr::aliyun::AliyunRegion;

    fn config(model: &str) -> AliyunConfig {
        AliyunConfig {
            _name: None,
            api_key: "key".into(),
            workspace_id: "ws".into(),
            region: AliyunRegion::Beijing,
            model: model.into(),
            vocabulary_id: String::new(),
            language_hints: Vec::new(),
            semantic_punctuation_enabled: false,
            max_sentence_silence: 1300,
            multi_threshold_mode_enabled: false,
            heartbeat: true,
            speech_noise_threshold: None,
            local_vad: crate::config::asr::LocalVadMode::Auto,
            open_timeout_ms: 12_000,
            finalize_timeout_ms: 12_000,
        }
    }

    #[test]
    fn run_task_uses_binary_pcm_contract_and_fun_context() {
        let ctx = SessionCtx {
            language: LanguageMode::Multilingual { hint: vec![] },
            hotwords: vec!["shuohua".into(), "Fun-ASR".into()],
        };
        let value = build_run_task(&config("fun-asr-realtime"), &ctx, "task");
        assert_eq!(value["payload"]["parameters"]["format"], "pcm");
        assert_eq!(value["payload"]["parameters"]["sample_rate"], 16_000);
        assert_eq!(
            value["payload"]["input"]["context"][0]["content"][0]["text"],
            "shuohua，Fun-ASR"
        );
    }

    #[test]
    fn fun_sends_speech_noise_threshold_only_when_set_and_no_paraformer_params() {
        let ctx = SessionCtx {
            language: LanguageMode::Multilingual { hint: vec![] },
            hotwords: vec![],
        };
        // 未设置阈值：不发送，也从不发送已删除的 Paraformer 参数族。
        let value = build_run_task(&config("fun-asr-realtime"), &ctx, "task");
        let params = value["payload"]["parameters"].as_object().unwrap();
        assert!(!params.contains_key("speech_noise_threshold"));
        assert!(!params.contains_key("disfluency_removal_enabled"));

        // 设置阈值：按需发送。
        let mut cfg = config("fun-asr-realtime");
        cfg.speech_noise_threshold = Some(0.3);
        let value = build_run_task(&cfg, &ctx, "task");
        assert_eq!(
            value["payload"]["parameters"]["speech_noise_threshold"],
            json!(0.3)
        );
    }

    #[test]
    fn runtime_options_come_from_aliyun_config() {
        let mut config = config("fun-asr-realtime");
        config.local_vad = crate::config::asr::LocalVadMode::On;
        config.open_timeout_ms = 23_000;
        config.finalize_timeout_ms = 17_000;

        let options = runtime_options(&config);

        assert_eq!(options.local_vad, crate::config::asr::LocalVadMode::On);
        assert_eq!(options.open_timeout_ms, 23_000);
        assert_eq!(options.finalize_timeout_ms, 17_000);
    }

    #[tokio::test]
    async fn result_events_map_to_partial_segment_and_done() {
        let (tx, mut rx) = mpsc::channel(8);
        let start = Instant::now();
        let mut seq = 0;
        let mut emitted = BTreeSet::new();
        let partial = r#"{"header":{"task_id":"t","event":"result-generated"},"payload":{"output":{"sentence":{"begin_time":10,"end_time":null,"text":"你","sentence_end":false,"sentence_id":1}}}}"#;
        let final_result = r#"{"header":{"task_id":"t","event":"result-generated"},"payload":{"output":{"sentence":{"begin_time":10,"end_time":200,"text":"你好。","sentence_end":true,"sentence_id":1}}}}"#;
        handle_server_message(partial, "t", start, &mut seq, &mut emitted, &tx)
            .await
            .unwrap();
        handle_server_message(final_result, "t", start, &mut seq, &mut emitted, &tx)
            .await
            .unwrap();
        assert!(matches!(rx.recv().await, Some(AsrEvent::Partial { .. })));
        assert!(matches!(rx.recv().await, Some(AsrEvent::Segment { .. })));
    }
}
