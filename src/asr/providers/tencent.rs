//! Tencent Cloud realtime ASR provider.
//!
//! 协议: https://cloud.tencent.com/document/product/1093/48982
//! Endpoint: wss://asr.cloud.tencent.com/asr/v2/{appid}
//!
//! 协议要点：
//! - 鉴权放 URL query：参数排序后用 SecretKey 做 HMAC-SHA1，再 Base64 + URL encode。
//! - 音频固定 16kHz s16le mono PCM；`voice_format=1` 在 provider 内写死。
//! - binary message 直接发送 PCM bytes；结束时发送 text `{"type":"end"}`。

use crate::asr::types::*;
use crate::config::asr::tencent::TencentConfig;
use async_trait::async_trait;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, KeyInit, Mac};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;
use toml::value::Table;

type HmacSha1 = Hmac<sha1::Sha1>;
type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const HOST: &str = "asr.cloud.tencent.com";
const CHUNK_SAMPLES: usize = 3200;
const CHUNK_BYTES: usize = CHUNK_SAMPLES * 2;
const VOICE_FORMAT_PCM: u8 = 1;
const QUERY_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

pub struct TencentProvider {
    config: TencentConfig,
}

impl TencentProvider {
    fn from_config(config: TencentConfig) -> Self {
        Self { config }
    }

    pub(crate) fn new_from_path_with_overrides(
        path: &Path,
        overrides: Option<&Table>,
    ) -> anyhow::Result<Self> {
        Ok(Self::from_config(TencentConfig::from_path_with_overrides(
            path, overrides,
        )?))
    }

    pub fn finalize_timeout_ms(&self) -> u64 {
        self.config.finalize_timeout_ms
    }

    pub fn options(&self) -> crate::asr::providers::ProviderOptions {
        crate::asr::providers::ProviderOptions {
            local_vad: self.config.local_vad,
            open_timeout_ms: self.config.open_timeout_ms,
            finalize_timeout_ms: self.config.finalize_timeout_ms,
        }
    }

    pub async fn check_runtime(&self, ctx: SessionCtx) -> Result<(), AsrError> {
        let (mut session, mut events) = self.open(ctx).await?;
        session.send_pcm(&[0i16; 1600], true).await?;
        let done = tokio::time::timeout(Duration::from_millis(self.finalize_timeout_ms()), async {
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
                "tencent stream closed before done".into(),
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
impl AsrProvider for TencentProvider {
    fn name(&self) -> &str {
        "tencent"
    }

    fn caps(&self) -> Caps {
        Caps {
            hotwords: true,
            max_session_secs: None,
            multilingual: self.config.engine_model_type == "16k_zh_en",
        }
    }

    async fn open(
        &self,
        ctx: SessionCtx,
    ) -> Result<(Box<dyn AsrSession>, mpsc::Receiver<AsrEvent>), AsrError> {
        let voice_id = uuid::Uuid::new_v4().to_string();
        let url = signed_url(&self.config, &ctx, &voice_id, now_unix_secs())?;
        let (mut ws, _resp) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(connect_err)?;

        match ws.next().await {
            Some(Ok(Message::Text(text))) => validate_open_response(&text)?,
            Some(Ok(Message::Close(_))) => {
                return Err(AsrError::Network(
                    "tencent websocket closed during handshake".into(),
                ));
            }
            Some(Ok(_)) => {}
            Some(Err(e)) => return Err(AsrError::Network(e.to_string())),
            None => return Err(AsrError::Network("tencent websocket ended".into())),
        }

        let (cmd_tx, cmd_rx) = mpsc::channel::<PcmCmd>(64);
        let (evt_tx, evt_rx) = mpsc::channel::<AsrEvent>(64);
        let cancel = CancellationToken::new();
        let cancel_for_task = cancel.clone();
        tokio::spawn(async move {
            session_task(ws, cmd_rx, evt_tx, cancel_for_task).await;
        });

        Ok((Box::new(TencentSession { cmd_tx, cancel }), evt_rx))
    }
}

fn connect_err(e: tokio_tungstenite::tungstenite::Error) -> AsrError {
    use tokio_tungstenite::tungstenite::Error::*;
    match &e {
        Http(resp) => match resp.status().as_u16() {
            401 | 403 => AsrError::Auth(format!("HTTP {}", resp.status())),
            429 => AsrError::Quota,
            code => AsrError::Network(format!("HTTP {code}")),
        },
        _ => AsrError::Network(e.to_string()),
    }
}

enum PcmCmd {
    Audio { bytes: Vec<u8>, is_last: bool },
}

pub struct TencentSession {
    cmd_tx: mpsc::Sender<PcmCmd>,
    cancel: CancellationToken,
}

#[async_trait]
impl AsrSession for TencentSession {
    async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<(), AsrError> {
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for &s in pcm {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        super::send_session_command(
            &self.cmd_tx,
            PcmCmd::Audio { bytes, is_last },
            "tencent session task ended",
        )
        .await
    }

    async fn close(self: Box<Self>) -> Result<(), AsrError> {
        self.cancel.cancel();
        Ok(())
    }
}

impl Drop for TencentSession {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn session_task(
    ws: Ws,
    mut cmd_rx: mpsc::Receiver<PcmCmd>,
    evt_tx: mpsc::Sender<AsrEvent>,
    cancel: CancellationToken,
) {
    let (mut sink, mut stream) = ws.split();
    let started_at = Instant::now();
    let mut audio_buf = Vec::with_capacity(CHUNK_BYTES);
    let mut partial_seq = 0;
    let mut emitted_segments = BTreeSet::new();
    let mut last_partial: Option<TencentResult> = None;
    let mut end_sent = false;

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                return;
            }
            cmd = cmd_rx.recv(), if !end_sent => {
                match cmd {
                    None => {
                        return;
                    }
                    Some(PcmCmd::Audio { bytes, is_last }) => {
                        let messages = build_outbound_messages(&mut audio_buf, &bytes, is_last);
                        for message in messages {
                            if let Err(error) = super::bounded_session_io(
                                &cancel,
                                "tencent websocket send",
                                sink.send(message),
                            ).await {
                                if !matches!(error, AsrError::Canceled) {
                                    let _ = evt_tx.send(AsrEvent::Error { err: error }).await;
                                }
                                return;
                            }
                        }
                        if is_last {
                            end_sent = true;
                        }
                    }
                }
            }
            msg = stream.next() => {
                let Some(msg) = msg else {
                    let _ = evt_tx.send(AsrEvent::Error {
                        err: AsrError::Network("websocket stream ended before final response".into()),
                    }).await;
                    return;
                };
                match msg {
                    Err(e) => {
                        let _ = evt_tx.send(AsrEvent::Error { err: AsrError::Network(e.to_string()) }).await;
                        return;
                    }
                    Ok(Message::Text(text)) => {
                        match handle_text_response(
                            &text,
                            started_at,
                            &mut partial_seq,
                            &mut emitted_segments,
                            &mut last_partial,
                            &evt_tx,
                        ).await {
                            ResponseAction::Continue => {}
                            ResponseAction::Done => {
                                let _ = evt_tx.send(AsrEvent::Done).await;
                                return;
                            }
                            ResponseAction::Errored => return,
                        }
                    }
                    Ok(Message::Close(_)) => {
                        let _ = evt_tx.send(AsrEvent::Error {
                            err: AsrError::Network("websocket closed before final response".into()),
                        }).await;
                        return;
                    }
                    Ok(_) => {}
                }
            }
        }
    }
}

fn build_outbound_messages(audio_buf: &mut Vec<u8>, bytes: &[u8], is_last: bool) -> Vec<Message> {
    audio_buf.extend_from_slice(bytes);
    let mut out = Vec::new();
    while audio_buf.len() >= CHUNK_BYTES {
        let chunk: Vec<u8> = audio_buf.drain(..CHUNK_BYTES).collect();
        out.push(Message::Binary(chunk.into()));
    }
    if is_last {
        if !audio_buf.is_empty() {
            let chunk = std::mem::take(audio_buf);
            out.push(Message::Binary(chunk.into()));
        }
        out.push(Message::Text(r#"{"type":"end"}"#.into()));
    }
    out
}

fn signed_url(
    cfg: &TencentConfig,
    ctx: &SessionCtx,
    voice_id: &str,
    timestamp: u64,
) -> Result<String, AsrError> {
    let expired = timestamp + 24 * 60 * 60;
    let nonce = (timestamp ^ (ulid::Ulid::generate().random() as u64)) % 1_000_000_000;
    let mut params = BTreeMap::new();
    params.insert("convert_num_mode", cfg.convert_num_mode.to_string());
    params.insert("engine_model_type", cfg.engine_model_type.clone());
    params.insert("expired", expired.to_string());
    params.insert("filter_dirty", cfg.filter_dirty.to_string());
    params.insert("filter_modal", cfg.filter_modal.to_string());
    params.insert("filter_punc", u8::from(cfg.filter_punc).to_string());
    insert_if_not_empty(&mut params, "customization_id", &cfg.customization_id);
    insert_if_not_empty(&mut params, "hotword_id", &cfg.hotword_id);
    if let Some(hotword_list) = build_hotword_list(&ctx.hotwords, cfg.hotword_weight) {
        params.insert("hotword_list", hotword_list);
    }
    params.insert("needvad", u8::from(cfg.need_vad).to_string());
    if cfg.need_vad {
        params.insert("vad_silence_time", cfg.vad_silence_time.to_string());
        params.insert("max_speak_time", cfg.max_speak_time.to_string());
    }
    params.insert("noise_threshold", format_float(cfg.noise_threshold));
    params.insert("nonce", nonce.to_string());
    insert_if_not_empty(&mut params, "replace_text_id", &cfg.replace_text_id);
    params.insert("secretid", cfg.secret_id.clone());
    params.insert("sentence_strategy", cfg.sentence_strategy.to_string());
    params.insert("timestamp", timestamp.to_string());
    params.insert("voice_format", VOICE_FORMAT_PCM.to_string());
    params.insert("voice_id", voice_id.to_string());

    let signing_query = join_query(&params, false);
    let sign_source = format!("{HOST}/asr/v2/{}?{signing_query}", cfg.app_id);
    let signature = sign_hmac_sha1_base64(&cfg.secret_key, &sign_source)?;

    let mut url_query = join_query(&params, true);
    url_query.push_str("&signature=");
    url_query.push_str(&url_encode(&signature));
    Ok(format!(
        "wss://{HOST}/asr/v2/{}?{url_query}",
        url_encode(&cfg.app_id)
    ))
}

fn insert_if_not_empty(
    params: &mut BTreeMap<&'static str, String>,
    key: &'static str,
    value: &str,
) {
    if !value.trim().is_empty() {
        params.insert(key, value.trim().to_string());
    }
}

fn format_float(value: f64) -> String {
    let mut out = value.to_string();
    if out == "-0" {
        out = "0".to_string();
    }
    out
}

fn sign_hmac_sha1_base64(secret_key: &str, source: &str) -> Result<String, AsrError> {
    let mut mac = HmacSha1::new_from_slice(secret_key.as_bytes())
        .map_err(|e| AsrError::Protocol(format!("build signature: {e}")))?;
    mac.update(source.as_bytes());
    Ok(base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}

fn join_query(params: &BTreeMap<&'static str, String>, encode_values: bool) -> String {
    params
        .iter()
        .map(|(key, value)| {
            if encode_values {
                format!("{key}={}", url_encode(value))
            } else {
                format!("{key}={value}")
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn url_encode(value: &str) -> String {
    utf8_percent_encode(value, QUERY_ENCODE_SET).to_string()
}

fn build_hotword_list(words: &[String], weight: u8) -> Option<String> {
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    for word in words {
        let word = word.trim();
        if word.is_empty() || word.contains(char::is_whitespace) || !seen.insert(word.to_string()) {
            continue;
        }
        items.push(format!("{word}|{weight}"));
        if items.len() >= 128 {
            break;
        }
    }
    if items.is_empty() {
        None
    } else {
        Some(items.join(","))
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

enum ResponseAction {
    Continue,
    Done,
    Errored,
}

async fn handle_text_response(
    text: &str,
    started_at: Instant,
    seq: &mut u64,
    emitted_segments: &mut BTreeSet<i64>,
    last_partial: &mut Option<TencentResult>,
    evt_tx: &mpsc::Sender<AsrEvent>,
) -> ResponseAction {
    let parsed = match parse_response(text) {
        Ok(parsed) => parsed,
        Err(e) => {
            let _ = evt_tx.send(AsrEvent::Error { err: e }).await;
            return ResponseAction::Errored;
        }
    };

    if parsed.code != 0 {
        tracing::warn!(
            code = parsed.code,
            message = %parsed.message,
            "tencent asr server error"
        );
        let _ = evt_tx
            .send(AsrEvent::Error {
                err: map_server_error(parsed.code, &parsed.message),
            })
            .await;
        return ResponseAction::Errored;
    }

    if let Some(result) = parsed.result {
        match result.slice_type {
            1 => {
                if !result.voice_text_str.is_empty() {
                    *seq += 1;
                    let _ = evt_tx
                        .send(AsrEvent::Partial {
                            text: result.voice_text_str.clone(),
                            seq: *seq,
                        })
                        .await;
                    *last_partial = Some(result);
                }
            }
            2 => {
                emit_segment_if_new(&result, started_at, emitted_segments, evt_tx).await;
                if last_partial
                    .as_ref()
                    .is_some_and(|p| p.index == result.index)
                {
                    *last_partial = None;
                }
            }
            _ => {}
        }
    }

    if parsed.final_flag == Some(1) {
        if let Some(result) = last_partial.take() {
            emit_segment_if_new(&result, started_at, emitted_segments, evt_tx).await;
        }
        ResponseAction::Done
    } else {
        ResponseAction::Continue
    }
}

async fn emit_segment_if_new(
    result: &TencentResult,
    started_at: Instant,
    emitted_segments: &mut BTreeSet<i64>,
    evt_tx: &mpsc::Sender<AsrEvent>,
) {
    if result.voice_text_str.is_empty() || !emitted_segments.insert(result.index) {
        return;
    }
    let _ = evt_tx
        .send(AsrEvent::Segment {
            text: result.voice_text_str.clone(),
            started_at: started_at + Duration::from_millis(result.start_time.unwrap_or(0)),
            ended_at: started_at + Duration::from_millis(result.end_time.unwrap_or(0)),
        })
        .await;
}

fn validate_open_response(text: &str) -> Result<(), AsrError> {
    let parsed = parse_response(text)?;
    if parsed.code == 0 {
        Ok(())
    } else {
        Err(map_server_error(parsed.code, &parsed.message))
    }
}

fn parse_response(text: &str) -> Result<TencentResponse, AsrError> {
    serde_json::from_str(text).map_err(|e| AsrError::Protocol(format!("decode JSON: {e}")))
}

fn map_server_error(code: i64, message: &str) -> AsrError {
    let detail = if message.trim().is_empty() {
        format!("tencent code {code}")
    } else {
        format!(
            "tencent code {code}: {}",
            sanitize_remote_error_text(message, 256)
        )
    };
    match code {
        4002 | 4003 => AsrError::Auth(detail),
        4004..=4006 => AsrError::Quota,
        4008 => AsrError::Timeout,
        4009 => AsrError::Network(detail),
        4001 | 4007 | 4010 => AsrError::Protocol(detail),
        5000..=5002 => AsrError::Server(detail),
        _ => AsrError::Server(detail),
    }
}

fn sanitize_remote_error_text(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut last_was_space = false;
    for ch in text.trim().chars() {
        let mapped = if ch.is_control() { ' ' } else { ch };
        if mapped.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(mapped);
            last_was_space = false;
        }
        if out.chars().count() >= max_chars {
            out.push_str("... [truncated]");
            break;
        }
    }
    out.trim().to_string()
}

#[derive(Debug, Deserialize)]
struct TencentResponse {
    #[serde(default)]
    code: i64,
    #[serde(default, alias = "message")]
    message: String,
    #[serde(default)]
    result: Option<TencentResult>,
    #[serde(default, rename = "final")]
    final_flag: Option<u8>,
}

#[derive(Debug, Clone, Deserialize)]
struct TencentResult {
    #[serde(default)]
    slice_type: u8,
    #[serde(default)]
    index: i64,
    #[serde(default)]
    start_time: Option<u64>,
    #[serde(default)]
    end_time: Option<u64>,
    #[serde(default)]
    voice_text_str: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> TencentConfig {
        TencentConfig {
            _name: None,
            app_id: "1250000000".into(),
            secret_id: "sid".into(),
            secret_key: "key".into(),
            engine_model_type: "16k_zh".into(),
            need_vad: false,
            filter_dirty: 0,
            filter_modal: 0,
            filter_punc: false,
            convert_num_mode: 1,
            vad_silence_time: 1000,
            max_speak_time: 60_000,
            noise_threshold: 0.0,
            hotword_weight: 10,
            hotword_id: String::new(),
            customization_id: String::new(),
            replace_text_id: String::new(),
            sentence_strategy: 0,
            local_vad: crate::config::asr::LocalVadMode::Auto,
            open_timeout_ms: 12_000,
            finalize_timeout_ms: 12_000,
        }
    }

    #[test]
    fn signed_url_sorts_query_and_url_encodes_signature() {
        let mut cfg = test_config();
        cfg.need_vad = true;
        cfg.filter_dirty = 2;
        cfg.filter_modal = 1;
        cfg.convert_num_mode = 3;
        cfg.vad_silence_time = 1500;
        cfg.max_speak_time = 30_000;
        cfg.noise_threshold = -0.5;
        cfg.hotword_id = "table-1".into();
        cfg.customization_id = "model-1".into();
        cfg.replace_text_id = "replace-1".into();
        cfg.sentence_strategy = 1;
        let ctx = SessionCtx {
            language: LanguageMode::Multilingual { hint: vec![] },
            hotwords: vec!["Rust".into(), "  ".into(), "有 空格".into()],
        };

        let url = signed_url(&cfg, &ctx, "voice-1", 1_700_000_000).unwrap();

        assert!(url.starts_with("wss://asr.cloud.tencent.com/asr/v2/1250000000?"));
        let query = url.split_once('?').unwrap().1;
        assert!(query.contains("engine_model_type=16k_zh"));
        assert!(query.contains("voice_format=1"));
        assert!(query.contains("needvad=1"));
        assert!(!query.contains("need_vad="));
        assert!(query.contains("filter_dirty=2"));
        assert!(query.contains("filter_modal=1"));
        assert!(query.contains("convert_num_mode=3"));
        assert!(query.contains("vad_silence_time=1500"));
        assert!(query.contains("max_speak_time=30000"));
        assert!(query.contains("noise_threshold=-0.5"));
        assert!(query.contains("hotword_id=table-1"));
        assert!(query.contains("customization_id=model-1"));
        assert!(query.contains("replace_text_id=replace-1"));
        assert!(query.contains("sentence_strategy=1"));
        assert!(query.contains("hotword_list=Rust%7C10"));
        assert!(query.contains("signature="));
        assert!(!query.contains("secret_key="));
        assert!(!query.contains("signature=+"));
        assert!(!query.contains("signature=/"));
    }

    #[test]
    fn signed_url_omits_server_vad_thresholds_when_vad_is_disabled() {
        let cfg = test_config();
        let ctx = SessionCtx {
            language: LanguageMode::Single("zh-CN".into()),
            hotwords: vec![],
        };

        let url = signed_url(&cfg, &ctx, "voice-1", 1_700_000_000).unwrap();
        let query = url.split_once('?').unwrap().1;

        assert!(query.contains("needvad=0"));
        assert!(!query.contains("vad_silence_time="));
        assert!(!query.contains("max_speak_time="));
        assert!(!query.contains("filter_empty_result="));
        assert!(!query.contains("word_info="));
        assert!(!query.contains("emotion_recognition="));
    }

    #[test]
    fn hmac_sha1_matches_rfc_2202_test_vector() {
        let key = "\x0b".repeat(20);
        let signature = sign_hmac_sha1_base64(&key, "Hi There").unwrap();

        assert_eq!(signature, "thcxhlUFcmTii8C2+zeMjvFGvgA=");
    }

    #[test]
    fn outbound_messages_chunk_pcm_and_end_after_empty_last_flush() {
        let mut buf = Vec::new();
        let messages = build_outbound_messages(&mut buf, &[1u8; CHUNK_BYTES + 2], false);
        assert_eq!(messages.len(), 1);
        assert_eq!(buf.len(), 2);
        assert!(matches!(&messages[0], Message::Binary(bytes) if bytes.len() == CHUNK_BYTES));

        let messages = build_outbound_messages(&mut buf, &[], true);
        assert_eq!(messages.len(), 2);
        assert!(matches!(&messages[0], Message::Binary(bytes) if bytes.len() == 2));
        assert!(
            matches!(&messages[1], Message::Text(text) if text.as_str() == r#"{"type":"end"}"#)
        );
        assert!(buf.is_empty());
    }

    #[tokio::test]
    async fn maps_partial_segment_duplicate_segment_and_done() {
        let (evt_tx, mut evt_rx) = mpsc::channel(8);
        let mut seq = 0;
        let mut emitted = BTreeSet::new();
        let mut last_partial = None;
        let started_at = Instant::now();

        let action = handle_text_response(
            r#"{"code":0,"result":{"slice_type":1,"index":0,"voice_text_str":"你好"}}"#,
            started_at,
            &mut seq,
            &mut emitted,
            &mut last_partial,
            &evt_tx,
        )
        .await;
        assert!(matches!(action, ResponseAction::Continue));
        assert!(matches!(
            evt_rx.recv().await.unwrap(),
            AsrEvent::Partial { text, seq: 1 } if text == "你好"
        ));

        let action = handle_text_response(
            r#"{"code":0,"result":{"slice_type":2,"index":0,"start_time":10,"end_time":20,"voice_text_str":"你好。"}}"#,
            started_at,
            &mut seq,
            &mut emitted,
            &mut last_partial,
            &evt_tx,
        )
        .await;
        assert!(matches!(action, ResponseAction::Continue));
        assert!(matches!(
            evt_rx.recv().await.unwrap(),
            AsrEvent::Segment { text, .. } if text == "你好。"
        ));

        let _ = handle_text_response(
            r#"{"code":0,"result":{"slice_type":2,"index":0,"voice_text_str":"你好。"}}"#,
            started_at,
            &mut seq,
            &mut emitted,
            &mut last_partial,
            &evt_tx,
        )
        .await;
        assert!(evt_rx.try_recv().is_err());

        let action = handle_text_response(
            r#"{"code":0,"final":1}"#,
            started_at,
            &mut seq,
            &mut emitted,
            &mut last_partial,
            &evt_tx,
        )
        .await;
        assert!(matches!(action, ResponseAction::Done));
    }

    #[tokio::test]
    async fn promotes_last_partial_to_segment_on_final() {
        let (evt_tx, mut evt_rx) = mpsc::channel(8);
        let mut seq = 0;
        let mut emitted = BTreeSet::new();
        let mut last_partial = None;
        let started_at = Instant::now();

        let _ = handle_text_response(
            r#"{"code":0,"result":{"slice_type":1,"index":2,"voice_text_str":"尾句"}}"#,
            started_at,
            &mut seq,
            &mut emitted,
            &mut last_partial,
            &evt_tx,
        )
        .await;
        let _ = evt_rx.recv().await.unwrap();

        let action = handle_text_response(
            r#"{"code":0,"final":1}"#,
            started_at,
            &mut seq,
            &mut emitted,
            &mut last_partial,
            &evt_tx,
        )
        .await;

        assert!(matches!(action, ResponseAction::Done));
        assert!(matches!(
            evt_rx.recv().await.unwrap(),
            AsrEvent::Segment { text, .. } if text == "尾句"
        ));
    }

    #[test]
    fn maps_server_error_codes() {
        assert!(matches!(map_server_error(4002, "bad"), AsrError::Auth(_)));
        assert!(matches!(map_server_error(4004, "quota"), AsrError::Quota));
        assert!(matches!(map_server_error(4008, "idle"), AsrError::Timeout));
        assert!(matches!(
            map_server_error(4009, "network"),
            AsrError::Network(_)
        ));
        assert!(matches!(
            map_server_error(4010, "text"),
            AsrError::Protocol(_)
        ));
        assert!(matches!(
            map_server_error(5000, "load"),
            AsrError::Server(_)
        ));
    }

    #[test]
    fn constants_match_tencent_pcm_contract() {
        assert_eq!(VOICE_FORMAT_PCM, 1);
        assert_eq!(CHUNK_SAMPLES * 2, CHUNK_BYTES);
    }
}
