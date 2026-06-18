//! Doubao SAUC bigmodel_async provider.
//!
//! 协议: https://www.volcengine.com/docs/6561/1354869
//! Endpoint: wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async
//!
//! 协议要点（与 Hypnus-Yuan/doubao-speech、chaitin/MonkeyCode、openless、
//! yyyzl/push-2-talk 四份独立实现交叉验证一致）：
//!
//! - 鉴权走 HTTP upgrade header：X-Api-App-Key / X-Api-Access-Key /
//!   X-Api-Resource-Id / X-Api-Request-Id / X-Api-Connect-Id / X-Api-Sequence
//! - 客户端二进制帧 = [4 字节 header][4 字节 sequence BE][4 字节 size BE][payload]
//!
//!   ```text
//!   byte0 = 0x11 (proto v1 << 4 | header_size=1)
//!   byte1 = msg_type << 4 | flags
//!           msg_type: 0x1=full client req, 0x2=audio-only
//!           flags:    0x1=PositiveSeq（普通帧）, 0x3=NegativeSeq（末帧）
//!   byte2 = serialize << 4 | compress
//!           serialize: 0x1=JSON, 0x0=raw bytes
//!           compress:  0x0=none (raw PCM 高熵 gzip 仅 ~30%，DESIGN §2.8 不压)
//!   byte3 = 0x00 reserved
//!   ```
//!
//!   **所有客户端帧都带 sequence**：FullClientRequest=1，audio 帧从 2 起单调递增，
//!   末帧 flag 改为 NegativeSeq + 取负序号（如已发到 5，末帧 seq=-6）。
//!   不带 sequence 的"裸末帧"（flags=0x02）会让 Doubao 走 server 自分配 fallback
//!   路径，偶发 finalize 慢到 5s+。
//! - 服务端帧 = [4 字节 header][4 字节 sequence (跳过)][4 字节 size BE][payload]
//!   payload 是 result/utterances/audio_info JSON。flag bit 0x02 = 末帧
//!   （server 也可能用 NegativeSeq=0x03，has_seq + is_last 两位都会置上）
//! - `enable_nonstream=true` + `show_utterances=true` 是定型 (`definite=true`)
//!   的必要条件，DESIGN §2.8 表里 Doubao 行依赖这两个 flag
//! - 音频 codec 写死 raw PCM 16kHz s16le mono；gzip 收益小不做

use crate::asr::types::*;
use crate::config::asr::doubao::{load_config_with_overrides, DoubaoConfig};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::time::Instant;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;
use toml::value::Table;

const ENDPOINT: &str = "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async";

pub use crate::config::asr::doubao::config_path;

#[cfg(test)]
use crate::config::asr::doubao::{default_finalize_timeout_ms, default_resource_id};

// ============================================================
// 2. Provider
// ============================================================

pub struct DoubaoProvider {
    config: DoubaoConfig,
}

impl DoubaoProvider {
    pub fn new_with_overrides(overrides: Option<&Table>) -> anyhow::Result<Self> {
        Ok(Self {
            config: load_config_with_overrides(overrides)?,
        })
    }

    pub fn idle_pause(&self) -> bool {
        self.config.idle_pause
    }

    pub fn finalize_timeout_ms(&self) -> u64 {
        self.config.finalize_timeout_ms
    }

    pub async fn check_runtime(&self, ctx: SessionCtx) -> Result<(), AsrError> {
        let (session, _events) = self.open(ctx).await?;
        session.close().await
    }
}

#[async_trait]
impl AsrProvider for DoubaoProvider {
    fn name(&self) -> &str {
        "doubao"
    }

    fn caps(&self) -> Caps {
        Caps {
            hotwords: true,
            max_session_secs: None,
            multilingual: true,
        }
    }

    async fn open(
        &self,
        ctx: SessionCtx,
    ) -> Result<(Box<dyn AsrSession>, mpsc::Receiver<AsrEvent>), AsrError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let connect_id = uuid::Uuid::new_v4().to_string();

        let mut req = ENDPOINT
            .into_client_request()
            .map_err(|e| AsrError::Protocol(format!("build request: {e}")))?;
        {
            let headers = req.headers_mut();
            for (k, v) in [
                ("X-Api-App-Key", self.config.app_key.as_str()),
                ("X-Api-Access-Key", self.config.access_key.as_str()),
                ("X-Api-Resource-Id", self.config.resource_id.as_str()),
                ("X-Api-Request-Id", &request_id),
                ("X-Api-Connect-Id", &connect_id),
                ("X-Api-Sequence", "-1"),
            ] {
                let val = HeaderValue::from_str(v)
                    .map_err(|e| AsrError::Protocol(format!("header {k}: {e}")))?;
                headers.insert(k, val);
            }
        }

        let (mut ws, resp) = tokio_tungstenite::connect_async(req)
            .await
            .map_err(connect_err)?;

        // X-Tt-Logid: 服务端日志对账 id，断网/识别异常时拿这个去问火山
        let logid = resp
            .headers()
            .get("X-Tt-Logid")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-")
            .to_string();
        tracing::debug!(logid = %logid, "doubao connected");

        // 首条 full client request；占 seq=1，后续 audio 帧从 seq=2 开始。
        let payload = build_full_client_request_payload(&self.config, &ctx);
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| AsrError::Protocol(format!("encode init payload: {e}")))?;
        let frame = encode_full_client_request(&payload_bytes, 1);
        ws.send(Message::Binary(frame.into()))
            .await
            .map_err(send_err)?;

        // 启动 session task
        let (cmd_tx, cmd_rx) = mpsc::channel::<PcmCmd>(64);
        let (evt_tx, evt_rx) = mpsc::channel::<AsrEvent>(64);
        let cancel = CancellationToken::new();
        let cancel_for_task = cancel.clone();
        tokio::spawn(async move {
            session_task(ws, cmd_rx, evt_tx, cancel_for_task).await;
        });

        Ok((Box::new(DoubaoSession { cmd_tx, cancel }), evt_rx))
    }
}

fn connect_err(e: tokio_tungstenite::tungstenite::Error) -> AsrError {
    use tokio_tungstenite::tungstenite::Error::*;
    match &e {
        Http(resp) => {
            let code = resp.status().as_u16();
            match code {
                401 | 403 => AsrError::Auth(format!("HTTP {code}")),
                429 => AsrError::Quota,
                _ => AsrError::Network(format!("HTTP {code}")),
            }
        }
        _ => AsrError::Network(e.to_string()),
    }
}

fn send_err(e: tokio_tungstenite::tungstenite::Error) -> AsrError {
    AsrError::Network(format!("ws send: {e}"))
}

// ============================================================
// 3. Session
// ============================================================

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

enum PcmCmd {
    Audio { bytes: Vec<u8>, is_last: bool },
}

pub struct DoubaoSession {
    cmd_tx: mpsc::Sender<PcmCmd>,
    cancel: CancellationToken,
}

#[async_trait]
impl AsrSession for DoubaoSession {
    async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<(), AsrError> {
        // i16 → u8 BE? No — Doubao bigmodel_async expects pcm_s16le (little endian)
        // per audio.format=pcm + bits=16 + ASR convention. Same as cpal native.
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for &s in pcm {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        self.cmd_tx
            .send(PcmCmd::Audio { bytes, is_last })
            .await
            .map_err(|_| AsrError::Network("session task ended".into()))
    }

    async fn close(self: Box<Self>) -> Result<(), AsrError> {
        self.cancel.cancel();
        // dropping cmd_tx 也会让 task 退出；这里靠 cancel 提前打断 stream.next()
        Ok(())
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
    let mut definite_emitted: usize = 0;
    let mut drift = DriftProbe::new();
    let mut partial_seq: u64 = 0;
    // 协议层 audio 帧 sequence number。FullClientRequest 在 open() 里占 seq=1，
    // 这里从 2 起。末帧 (is_last=true) 用 NegativeSeq + 下一个 seq 取负——
    // 先递增再取负，避免末帧 seq 跟最后一帧 audio seq 撞号触发服务端
    // "autoAssignedSequence mismatch" 报错（chaitin 实测过这条坑）。
    let mut wire_seq: i32 = 1;
    let mut last_sent = false; // 是否已发出 is_last 帧；之后只等服务端 Done

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                let _ = sink.close().await;
                return;
            }
            cmd = cmd_rx.recv(), if !last_sent => {
                match cmd {
                    None => {
                        let _ = sink.close().await;
                        return;
                    }
                    Some(PcmCmd::Audio { bytes, is_last }) => {
                        wire_seq += 1;
                        let frame_seq = if is_last { -wire_seq } else { wire_seq };
                        let frame = encode_audio_frame(&bytes, frame_seq, is_last);
                        if let Err(e) = sink.send(Message::Binary(frame.into())).await {
                            let _ = evt_tx.send(AsrEvent::Error { err: AsrError::Network(e.to_string()) }).await;
                            return;
                        }
                        if is_last {
                            last_sent = true;
                        }
                    }
                }
            }
            msg = stream.next() => {
                let Some(msg) = msg else {
                    let _ = evt_tx.send(AsrEvent::Done).await;
                    return;
                };
                match msg {
                    Err(e) => {
                        let _ = evt_tx.send(AsrEvent::Error { err: AsrError::Network(e.to_string()) }).await;
                        return;
                    }
                    Ok(Message::Binary(data)) => {
                        match handle_response(
                            &data,
                            started_at,
                            &mut definite_emitted,
                            &mut drift,
                            &mut partial_seq,
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
                        let _ = evt_tx.send(AsrEvent::Done).await;
                        return;
                    }
                    Ok(_) => {} // ping/pong/text — 忽略
                }
            }
        }
    }
}

enum ResponseAction {
    Continue,
    Done,
    Errored,
}

/// Drift 探针 — debug-only。release build 里方法体空、struct zero-sized，编译器消除得干净。
///
/// 监测两类豆包行为偏离我们的假设：
/// 1. 已 emit 的 definite utterance 在后续帧里 text 被改（[`Self::check_drift`]）
/// 2. session 末尾 `snapshots.concat()` 与豆包 cumulative `result.text` 不一致（[`Self::check_final`]）
///
/// 任何一条 `⚠` 出现就该回头看豆包协议是不是变了 / 我们的 segment-concat 假设是不是破了。
/// 日志门禁原则见 `docs/DESIGN.md` §2.13。
///
/// 用两份 `cfg` impl 分支，避免在表达式 / 语句位置写 `#[cfg]`（仍是 nightly）。
#[cfg(debug_assertions)]
struct DriftProbe {
    snapshots: Vec<String>,
}

#[cfg(not(debug_assertions))]
struct DriftProbe;

#[cfg(debug_assertions)]
impl DriftProbe {
    fn new() -> Self {
        Self {
            snapshots: Vec::new(),
        }
    }

    fn check_drift(&self, i: usize, current: &str) {
        if let Some(prev) = self.snapshots.get(i) {
            if prev != current {
                tracing::warn!(
                    utterance_index = i,
                    previous_chars = prev.chars().count(),
                    current_chars = current.chars().count(),
                    "doubao utterance drift detected"
                );
            }
        }
    }

    fn record(&mut self, text: String) {
        self.snapshots.push(text);
    }

    fn check_final(&self, doubao_text: &str) {
        let ours: String = self.snapshots.concat();
        if ours != doubao_text {
            tracing::debug!(
                ours_chars = ours.chars().count(),
                doubao_chars = doubao_text.chars().count(),
                "doubao final text mismatch"
            );
        }
    }
}

#[cfg(not(debug_assertions))]
impl DriftProbe {
    fn new() -> Self {
        Self
    }
    fn check_drift(&self, _i: usize, _current: &str) {}
    fn record(&mut self, _text: String) {}
    fn check_final(&self, _doubao_text: &str) {}
}

async fn handle_response(
    data: &[u8],
    started_at: Instant,
    definite_emitted: &mut usize,
    drift: &mut DriftProbe,
    seq: &mut u64,
    evt_tx: &mpsc::Sender<AsrEvent>,
) -> ResponseAction {
    let frame = match parse_response_frame(data) {
        Ok(f) => f,
        Err(e) => {
            let _ = evt_tx.send(AsrEvent::Error { err: e }).await;
            return ResponseAction::Errored;
        }
    };

    if frame.msg_type == SRV_MSG_ERROR {
        let msg = std::str::from_utf8(&frame.payload)
            .unwrap_or("<non-utf8>")
            .to_string();
        let _ = evt_tx
            .send(AsrEvent::Error {
                err: AsrError::Server(msg),
            })
            .await;
        return ResponseAction::Errored;
    }
    if frame.msg_type != SRV_MSG_FULL_RESPONSE {
        // 未知消息类型，跳过不致命
        return ResponseAction::Continue;
    }

    let parsed: DoubaoResponseJson = match serde_json::from_slice(&frame.payload) {
        Ok(p) => p,
        Err(e) => {
            let _ = evt_tx
                .send(AsrEvent::Error {
                    err: AsrError::Protocol(format!("decode JSON: {e}")),
                })
                .await;
            return ResponseAction::Errored;
        }
    };

    if let Some(result) = parsed.result {
        // 新出现的 definite=true utterance 各推一条 Segment；已 emit 的过快照对比。
        for (i, u) in result.utterances.iter().enumerate() {
            if i < *definite_emitted {
                drift.check_drift(i, &u.text);
                continue;
            }
            if u.definite {
                if u.text.is_empty() {
                    // 豆包偶发：空 utterance 标 definite。不发 Segment 给上层
                    // （overlay / history / dispatch 都没意义），但 drift 快照得占位推进，
                    // 否则索引和 utterances[] 错位。
                    drift.record(String::new());
                    *definite_emitted = i + 1;
                    continue;
                }
                let _ = evt_tx
                    .send(AsrEvent::Segment {
                        text: u.text.clone(),
                        started_at: u
                            .start_time_ms
                            .map(|ms| started_at + std::time::Duration::from_millis(ms))
                            .unwrap_or(started_at),
                        ended_at: u
                            .end_time_ms
                            .map(|ms| started_at + std::time::Duration::from_millis(ms))
                            .unwrap_or_else(Instant::now),
                    })
                    .await;
                drift.record(u.text.clone());
                *definite_emitted = i + 1;
            }
        }
        if frame.is_last {
            drift.check_final(&result.text);
            if !result.text.is_empty() {
                let _ = evt_tx
                    .send(AsrEvent::Final {
                        text: result.text.clone(),
                    })
                    .await;
            }
        }
        // Partial 只取尾巴：result.text 是 cumulative 全文，会与已 emit 的
        // Segment 前缀重叠，导致 overlay (segments + partial) 复读。
        // AsrEvent::Partial 契约要求只发"当前 utterance 尾巴"，即非 definite
        // utterance 的拼接。
        let partial = compute_partial_text(&result.utterances);
        if !partial.is_empty() {
            *seq += 1;
            let _ = evt_tx
                .send(AsrEvent::Partial {
                    text: partial,
                    seq: *seq,
                })
                .await;
        }
    }

    if frame.is_last {
        ResponseAction::Done
    } else {
        ResponseAction::Continue
    }
}

// ============================================================
// 4. 二进制协议
// ============================================================

const HDR_BYTE0: u8 = 0x11; // proto v1 (0b0001 << 4) | header_size=1 (0b0001)
                            // byte1 高 4 bit = msg_type
const MSG_TYPE_FULL_CLIENT_REQ: u8 = 0x10; // 0b0001 << 4
const MSG_TYPE_AUDIO_ONLY: u8 = 0x20; // 0b0010 << 4
                                      // byte1 低 4 bit = flags（与 msg_type 按位或拼成 byte1）
const FLAG_POS_SEQ: u8 = 0x01; // 普通帧：带正序号
const FLAG_NEG_SEQ: u8 = 0x03; // 末帧：带负序号 + is_last
const SERIALIZE_JSON_NO_COMPRESS: u8 = 0x10; // serialize=JSON | compress=none
const SERIALIZE_RAW_NO_COMPRESS: u8 = 0x00; // serialize=raw | compress=none

const SRV_MSG_FULL_RESPONSE: u8 = 0x09; // 0b1001
const SRV_MSG_ERROR: u8 = 0x0F; // 0b1111

/// FullClientRequest 帧。`seq` 永远是 1（session 起手）。flag=PositiveSeq。
fn encode_full_client_request(payload: &[u8], seq: i32) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + payload.len());
    out.extend_from_slice(&[
        HDR_BYTE0,
        MSG_TYPE_FULL_CLIENT_REQ | FLAG_POS_SEQ,
        SERIALIZE_JSON_NO_COMPRESS,
        0x00,
    ]);
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Audio 帧。普通帧 (`is_last=false`) flag=PositiveSeq，`seq` 是单调递增的正数；
/// 末帧 (`is_last=true`) flag=NegativeSeq，`seq` 应该是负数（调用方负责取负）。
fn encode_audio_frame(pcm: &[u8], seq: i32, is_last: bool) -> Vec<u8> {
    let flag = if is_last { FLAG_NEG_SEQ } else { FLAG_POS_SEQ };
    let mut out = Vec::with_capacity(12 + pcm.len());
    out.extend_from_slice(&[
        HDR_BYTE0,
        MSG_TYPE_AUDIO_ONLY | flag,
        SERIALIZE_RAW_NO_COMPRESS,
        0x00,
    ]);
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&(pcm.len() as u32).to_be_bytes());
    out.extend_from_slice(pcm);
    out
}

#[derive(Debug, PartialEq)]
struct ResponseFrame {
    msg_type: u8,
    is_last: bool,
    payload: Vec<u8>,
}

fn parse_response_frame(data: &[u8]) -> Result<ResponseFrame, AsrError> {
    if data.len() < 4 {
        return Err(AsrError::Protocol(format!(
            "frame too short: {} bytes",
            data.len()
        )));
    }
    let msg_type = (data[1] >> 4) & 0x0F;
    let flags = data[1] & 0x0F;
    let has_seq = flags & 0x01 != 0;
    let is_last = flags & 0x02 != 0;

    let mut offset = 4;
    if has_seq {
        if data.len() < offset + 4 {
            return Err(AsrError::Protocol("missing sequence".into()));
        }
        offset += 4;
    }
    if data.len() < offset + 4 {
        return Err(AsrError::Protocol("missing payload size".into()));
    }
    let size = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
    offset += 4;
    if data.len() < offset + size {
        return Err(AsrError::Protocol(format!(
            "payload truncated: need {size} bytes, have {}",
            data.len() - offset
        )));
    }
    Ok(ResponseFrame {
        msg_type,
        is_last,
        payload: data[offset..offset + size].to_vec(),
    })
}

// ============================================================
// 5. JSON payloads
// ============================================================

fn build_full_client_request_payload(cfg: &DoubaoConfig, ctx: &SessionCtx) -> serde_json::Value {
    let mut audio = json!({
        "format": "pcm",
        "codec": "raw",
        "rate": 16000,
        "bits": 16,
        "channel": 1,
    });
    // language: bigmodel_async 留空时自动多语种识别；用户显式设置才写
    if let Some(lang) = cfg.language.as_deref().filter(|s| !s.is_empty()) {
        audio["language"] = json!(lang);
    } else if let LanguageMode::Single(lang) = &ctx.language {
        // 用户在 config.toml 没写 language，但 voice 模块要求单语
        audio["language"] = json!(lang);
    }

    let mut request = json!({
        "model_name":       "bigmodel",
        "enable_itn":       cfg.enable_itn,
        "enable_punc":      cfg.enable_punc,
        "enable_ddc":       cfg.enable_ddc,
        "enable_word":      false,
        "enable_nonstream": true,
        "result_type":      "full",
        "show_utterances":  true,
    });
    if !ctx.hotwords.is_empty() {
        request["corpus"] = json!({ "context": build_hotwords_context(&ctx.hotwords) });
    }
    if let Some(mode) = cfg.stream_mode {
        request["stream_mode"] = json!(mode);
    }
    if cfg.ai_vad == Some(true) {
        request["vad_config"] = json!({ "ai_vad": true });
    }

    json!({
        "user":    { "uid": "shuohua" },
        "audio":   audio,
        "request": request,
    })
}

/// Doubao 协议里 `corpus.context` 是 **stringified JSON 嵌套字段**：
/// `{"hotwords":[{"word":"Rust"},...]}` 转成字符串塞回去。
/// 这不是 hack，是文档规定形态。
fn build_hotwords_context(words: &[String]) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::with_capacity(words.len());
    for w in words {
        let w = w.trim();
        if w.is_empty() || !seen.insert(w.to_string()) {
            continue;
        }
        items.push(json!({ "word": w }));
    }
    serde_json::to_string(&json!({ "hotwords": items })).unwrap_or_default()
}

#[derive(Debug, Deserialize)]
struct DoubaoResponseJson {
    result: Option<DoubaoResultBody>,
}

#[derive(Debug, Deserialize)]
struct DoubaoResultBody {
    #[serde(default)]
    text: String,
    #[serde(default)]
    utterances: Vec<DoubaoUtterance>,
}

#[derive(Debug, Deserialize)]
struct DoubaoUtterance {
    #[serde(default)]
    text: String,
    #[serde(default)]
    definite: bool,
    #[serde(default, rename = "start_time")]
    start_time_ms: Option<u64>,
    #[serde(default, rename = "end_time")]
    end_time_ms: Option<u64>,
    // words 字段暂不进入通用 AsrEvent；M10 trace 先用 utterance 区间评估 VAD。
}

/// 从 utterances 里抽出"还在变化的尾巴"：跳过所有 definite=true 的（它们已经
/// 作为 Segment emit 过），把剩下的 text 拼起来。
///
/// 不能直接用 `result.text`：那是包含 definite 段的累计全文，会和已 emit 的
/// Segment 在 overlay (segments + partial) 里重复显示。
fn compute_partial_text(utterances: &[DoubaoUtterance]) -> String {
    utterances
        .iter()
        .filter(|u| !u.definite)
        .map(|u| u.text.as_str())
        .collect()
}

// ============================================================
// 6. tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_idle_pause_and_finalize_timeout_fields() {
        let cfg: DoubaoConfig = toml::from_str(
            r#"
app_key = "ak"
access_key = "sk"
idle_pause = true
finalize_timeout_ms = 7000
"#,
        )
        .unwrap();
        assert!(cfg.idle_pause);
        assert_eq!(cfg.finalize_timeout_ms, 7000);
    }

    #[test]
    fn idle_pause_defaults_off_and_finalize_timeout_default() {
        let cfg: DoubaoConfig = toml::from_str(
            r#"
app_key = "ak"
access_key = "sk"
"#,
        )
        .unwrap();
        assert!(!cfg.idle_pause);
        assert_eq!(cfg.finalize_timeout_ms, 12_000);
    }

    #[test]
    fn encode_full_client_request_uses_positive_seq_one() {
        let payload = b"{\"hello\":\"world\"}";
        let frame = encode_full_client_request(payload, 1);
        assert_eq!(frame[0], 0x11);
        assert_eq!(frame[1], 0x11, "msg_type=FullClientReq | PositiveSeq");
        assert_eq!(frame[2], 0x10);
        assert_eq!(frame[3], 0x00);
        assert_eq!(&frame[4..8], &1i32.to_be_bytes(), "sequence=1");
        assert_eq!(&frame[8..12], &(payload.len() as u32).to_be_bytes());
        assert_eq!(&frame[12..], payload);
    }

    #[test]
    fn encode_audio_frame_normal_carries_positive_seq() {
        let pcm = [1u8, 2, 3, 4];
        let frame = encode_audio_frame(&pcm, 7, false);
        assert_eq!(frame[0], 0x11);
        assert_eq!(frame[1], 0x21, "msg_type=AudioOnly | PositiveSeq");
        assert_eq!(&frame[4..8], &7i32.to_be_bytes(), "sequence=7");
        assert_eq!(&frame[8..12], &(pcm.len() as u32).to_be_bytes());
        assert_eq!(&frame[12..], &pcm);
    }

    #[test]
    fn encode_audio_frame_last_carries_negative_seq_and_empty_payload() {
        let pcm: [u8; 0] = [];
        let frame = encode_audio_frame(&pcm, -8, true);
        assert_eq!(frame[0], 0x11);
        assert_eq!(frame[1], 0x23, "msg_type=AudioOnly | NegativeSeq");
        assert_eq!(&frame[4..8], &(-8i32).to_be_bytes(), "sequence=-8");
        assert_eq!(&frame[8..12], &0u32.to_be_bytes(), "empty payload size");
        assert_eq!(frame.len(), 12);
    }

    #[test]
    fn parse_full_response_no_sequence() {
        let body = br#"{"result":{"text":"hi"}}"#;
        let mut data = vec![0x11, 0x90, 0x10, 0x00]; // server full response (type=0x9), flags=0
        data.extend_from_slice(&(body.len() as u32).to_be_bytes());
        data.extend_from_slice(body);
        let parsed = parse_response_frame(&data).unwrap();
        assert_eq!(parsed.msg_type, SRV_MSG_FULL_RESPONSE);
        assert!(!parsed.is_last);
        assert_eq!(parsed.payload, body);
    }

    #[test]
    fn parse_full_response_with_sequence_and_last_flag() {
        let body = br#"{"result":{"text":"bye"}}"#;
        // flags = 0b0011 = has sequence + last packet
        let mut data = vec![0x11, 0x93, 0x10, 0x00];
        data.extend_from_slice(&0u32.to_be_bytes()); // sequence (any value, ignored)
        data.extend_from_slice(&(body.len() as u32).to_be_bytes());
        data.extend_from_slice(body);
        let parsed = parse_response_frame(&data).unwrap();
        assert_eq!(parsed.msg_type, SRV_MSG_FULL_RESPONSE);
        assert!(parsed.is_last);
        assert_eq!(parsed.payload, body);
    }

    #[test]
    fn parse_server_error_frame() {
        let body = b"quota exceeded";
        let mut data = vec![0x11, 0xF0, 0x00, 0x00];
        data.extend_from_slice(&(body.len() as u32).to_be_bytes());
        data.extend_from_slice(body);
        let parsed = parse_response_frame(&data).unwrap();
        assert_eq!(parsed.msg_type, SRV_MSG_ERROR);
    }

    #[test]
    fn parse_rejects_truncated() {
        assert!(parse_response_frame(&[0x11, 0x90]).is_err()); // too short
                                                               // header OK but payload truncated
        let mut data = vec![0x11, 0x90, 0x10, 0x00];
        data.extend_from_slice(&100u32.to_be_bytes()); // claims 100 bytes
        data.extend_from_slice(b"only-a-few"); // but supplies few
        assert!(parse_response_frame(&data).is_err());
    }

    #[test]
    fn compute_partial_skips_definite_utterances() {
        // segment 定型后，doubao 会再发一帧 utterances=[{definite:true}]，
        // cumulative result.text 也仍然带这段文本。Partial 必须为空，否则
        // overlay 会把同一句显示两遍（见 overlay/mod.rs 的 segments+partial 模型）。
        let utterances = vec![DoubaoUtterance {
            text: "测试一下说话。".into(),
            definite: true,
            start_time_ms: None,
            end_time_ms: None,
        }];
        assert_eq!(compute_partial_text(&utterances), "");
    }

    #[test]
    fn compute_partial_concatenates_only_non_definite_tail() {
        let utterances = vec![
            DoubaoUtterance {
                text: "你好。".into(),
                definite: true,
                start_time_ms: None,
                end_time_ms: None,
            },
            DoubaoUtterance {
                text: "我".into(),
                definite: false,
                start_time_ms: None,
                end_time_ms: None,
            },
            DoubaoUtterance {
                text: "在说话".into(),
                definite: false,
                start_time_ms: None,
                end_time_ms: None,
            },
        ];
        assert_eq!(compute_partial_text(&utterances), "我在说话");
    }

    #[test]
    fn compute_partial_empty_when_no_utterances() {
        assert_eq!(compute_partial_text(&[]), "");
    }

    #[tokio::test]
    async fn last_response_emits_session_final_from_result_text() {
        let body = r#"{"result":{"text":"你好，世界","utterances":[{"text":"你好","definite":true},{"text":"世界","definite":true}]}}"#;
        let mut data = vec![0x11, 0x92, 0x10, 0x00]; // server full response + last flag
        data.extend_from_slice(&(body.len() as u32).to_be_bytes());
        data.extend_from_slice(body.as_bytes());

        let (tx, mut rx) = mpsc::channel(8);
        let mut definite_emitted = 0;
        let mut drift = DriftProbe::new();
        let mut seq = 0;
        let action = handle_response(
            &data,
            Instant::now(),
            &mut definite_emitted,
            &mut drift,
            &mut seq,
            &tx,
        )
        .await;

        assert!(matches!(action, ResponseAction::Done));
        assert!(matches!(rx.recv().await, Some(AsrEvent::Segment { text, .. }) if text == "你好"));
        assert!(matches!(rx.recv().await, Some(AsrEvent::Segment { text, .. }) if text == "世界"));
        assert!(matches!(rx.recv().await, Some(AsrEvent::Final { text }) if text == "你好，世界"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn utterance_deserializes_audio_time_offsets() {
        let utterance: DoubaoUtterance = serde_json::from_str(
            r#"{"text":"测试","definite":true,"start_time":120,"end_time":980}"#,
        )
        .unwrap();

        assert_eq!(utterance.start_time_ms, Some(120));
        assert_eq!(utterance.end_time_ms, Some(980));
    }

    #[test]
    fn hotwords_context_dedup_and_format() {
        let words = vec![
            "Rust".to_string(),
            "tokio".into(),
            "Rust".into(),
            "  ".into(),
        ];
        let s = build_hotwords_context(&words);
        // 期望 dedupe + 跳过空白
        assert!(s.contains(r#""word":"Rust""#));
        assert!(s.contains(r#""word":"tokio""#));
        assert_eq!(s.matches(r#""word":"Rust""#).count(), 1);
        assert!(!s.contains(r#""word":"""#));
    }

    #[test]
    fn full_client_request_payload_includes_hotwords_when_present() {
        let cfg = DoubaoConfig {
            app_key: "k".into(),
            access_key: "a".into(),
            resource_id: default_resource_id(),
            language: None,
            enable_itn: true,
            enable_punc: true,
            enable_ddc: false,
            stream_mode: None,
            ai_vad: None,
            idle_pause: false,
            finalize_timeout_ms: default_finalize_timeout_ms(),
        };
        let ctx = SessionCtx {
            language: LanguageMode::Multilingual { hint: vec![] },
            hotwords: vec!["Rust".into()],
        };
        let v = build_full_client_request_payload(&cfg, &ctx);
        assert_eq!(v["audio"]["codec"], "raw");
        assert_eq!(v["request"]["enable_nonstream"], true);
        assert_eq!(v["request"]["show_utterances"], true);
        assert!(v["request"]["corpus"]["context"]
            .as_str()
            .unwrap()
            .contains("Rust"));
        // 多语模式时不强制写 audio.language
        assert!(v["audio"]["language"].is_null());
    }

    #[test]
    fn full_client_request_payload_injects_experimental_knobs_when_set() {
        let cfg = DoubaoConfig {
            app_key: "k".into(),
            access_key: "a".into(),
            resource_id: default_resource_id(),
            language: None,
            enable_itn: true,
            enable_punc: true,
            enable_ddc: true,
            stream_mode: Some(2),
            ai_vad: Some(true),
            idle_pause: false,
            finalize_timeout_ms: default_finalize_timeout_ms(),
        };
        let ctx = SessionCtx {
            language: LanguageMode::Multilingual { hint: vec![] },
            hotwords: vec![],
        };
        let v = build_full_client_request_payload(&cfg, &ctx);
        assert_eq!(v["request"]["stream_mode"], 2);
        assert_eq!(v["request"]["vad_config"]["ai_vad"], true);
    }

    #[test]
    fn full_client_request_payload_omits_experimental_knobs_when_none() {
        let cfg = DoubaoConfig {
            app_key: "k".into(),
            access_key: "a".into(),
            resource_id: default_resource_id(),
            language: None,
            enable_itn: true,
            enable_punc: true,
            enable_ddc: true,
            stream_mode: None,
            ai_vad: None,
            idle_pause: false,
            finalize_timeout_ms: default_finalize_timeout_ms(),
        };
        let ctx = SessionCtx {
            language: LanguageMode::Multilingual { hint: vec![] },
            hotwords: vec![],
        };
        let v = build_full_client_request_payload(&cfg, &ctx);
        assert!(v["request"]["stream_mode"].is_null());
        assert!(v["request"]["vad_config"].is_null());
    }

    #[test]
    fn full_client_request_payload_skips_corpus_when_no_hotwords() {
        let cfg = DoubaoConfig {
            app_key: "k".into(),
            access_key: "a".into(),
            resource_id: default_resource_id(),
            language: Some("zh-CN".into()),
            enable_itn: true,
            enable_punc: true,
            enable_ddc: false,
            stream_mode: None,
            ai_vad: None,
            idle_pause: false,
            finalize_timeout_ms: default_finalize_timeout_ms(),
        };
        let ctx = SessionCtx {
            language: LanguageMode::Single("zh-CN".into()),
            hotwords: vec![],
        };
        let v = build_full_client_request_payload(&cfg, &ctx);
        assert!(v["request"]["corpus"].is_null());
        assert_eq!(v["audio"]["language"], "zh-CN");
    }
}
