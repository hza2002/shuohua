//! 一次录音的完整生命周期：录音 → ASR → pipeline → dispatch。
//!
//! M2.5 流程（单 ASR session）：
//!
//!   1. 生成 recording_id (ULID)
//!   2. 开 cpal streaming recorder（可选 wav 留存）
//!   3. 开 ASR session（DoubaoProvider）
//!   4. 主循环 select!：
//!        - stop_rx 收到 → 进 Finishing
//!        - recorder 帧 → 转发到 ASR
//!        - ASR 事件 → 累积 Segment（不再用 partial 兜底）
//!   5. Finishing：drain stop_delay_ms → stop recorder → is_last → 等 Done
//!   6. 拼文本 → filler pipeline → dispatch
//!
//! 设计上"一次用户会话可能包含多次 ASR session"（见 DESIGN §2.9），但 v1
//! 暂不实现客户端 VAD。webrtc-vad 在真实声学环境里误判率高，不适合生产。
//! 后续用更好的模型（如 Silero VAD ONNX）时再启用多 session，控制开关是
//! DESIGN §2.9 的 `voice.pause_asr_silence_ms` 等配置字段。

use std::time::{Duration, Instant};

use crate::asr::types::{AsrEvent, AsrProvider, AsrSession, LanguageMode, SessionCtx};
use crate::post::{self, PipelineText, RuleBasedFiller};
use crate::voice::{dispatch, recorder};
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep_until;

pub struct SessionParams {
    pub auto_paste: bool,
    pub record_audio: bool,
    pub stop_delay_ms: u32,
    pub hotwords: Vec<String>,
    pub segment_separator: String,
}

pub async fn run_recording(
    provider: &dyn AsrProvider,
    params: SessionParams,
    stop_rx: oneshot::Receiver<()>,
) {
    let recording_id = ulid::Ulid::new().to_string();
    eprintln!("[shuo] ▶ recording id={recording_id}");

    let audio_path = if params.record_audio {
        match prepare_audio_path(&recording_id) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("[shuo] record_audio 开启但准备路径失败: {e:#}");
                None
            }
        }
    } else {
        None
    };
    if let Some(p) = &audio_path {
        eprintln!("[shuo] 留存 wav → {}", p.display());
    }

    let mut rec = match recorder::start(audio_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[shuo] ❌ 录音启动失败: {e:#}");
            return;
        }
    };

    let ctx = SessionCtx {
        language: LanguageMode::Multilingual { hint: vec!["zh-CN".into(), "en-US".into()] },
        hotwords: params.hotwords,
    };
    let (mut session, mut events) = match provider.open(ctx).await {
        Ok(t) => t,
        Err(err) => {
            eprintln!("[shuo] ❌ ASR open failed: {err}");
            rec.stop();
            return;
        }
    };

    let mut stop_rx = stop_rx;
    let mut pending_segments: Vec<String> = Vec::new();
    let mut stop_requested = false;

    loop {
        tokio::select! {
            biased;
            _ = &mut stop_rx, if !stop_requested => {
                stop_requested = true;
            }
            pcm = rec.recv(), if !stop_requested => {
                match pcm {
                    Some(samples) => {
                        if let Err(e) = session.send_pcm(&samples, false).await {
                            eprintln!("[shuo] ❌ ASR send_pcm failed: {e}");
                            break;
                        }
                    }
                    None => {
                        eprintln!("[shuo] recorder ended unexpectedly");
                        stop_requested = true;
                    }
                }
            }
            ev = events.recv() => {
                match ev {
                    None => break,
                    Some(AsrEvent::Partial { text, seq }) => {
                        eprintln!("[shuo]   partial#{seq}: {text}");
                    }
                    Some(AsrEvent::Segment { text, .. }) => {
                        eprintln!("[shuo]   segment: {text}");
                        pending_segments.push(text);
                    }
                    Some(AsrEvent::Error { err }) => {
                        eprintln!("[shuo] ❌ ASR error: {err}");
                        rec.stop();
                        let _ = session.close().await;
                        return;
                    }
                    Some(AsrEvent::Done) => break,
                }
            }
        }

        if stop_requested {
            finish(
                &mut rec,
                &mut session,
                &mut events,
                &mut pending_segments,
                params.stop_delay_ms,
            )
            .await;
            break;
        }
    }

    let _ = session.close().await;
    dispatch_with_filler(pending_segments, &params.segment_separator, params.auto_paste).await;
}

async fn finish(
    rec: &mut recorder::RecordingStream,
    session: &mut Box<dyn AsrSession>,
    events: &mut mpsc::Receiver<AsrEvent>,
    pending_segments: &mut Vec<String>,
    stop_delay_ms: u32,
) {
    let drain_until = Instant::now() + Duration::from_millis(stop_delay_ms as u64);
    loop {
        let now = Instant::now();
        if now >= drain_until {
            break;
        }
        let deadline = tokio::time::Instant::from_std(drain_until);
        tokio::select! {
            biased;
            _ = sleep_until(deadline) => break,
            pcm = rec.recv() => {
                match pcm {
                    Some(samples) => {
                        let _ = session.send_pcm(&samples, false).await;
                    }
                    None => break,
                }
            }
            ev = events.recv() => {
                match ev {
                    None => break,
                    Some(AsrEvent::Segment { text, .. }) => {
                        eprintln!("[shuo]   segment (drain): {text}");
                        pending_segments.push(text);
                    }
                    Some(AsrEvent::Done) => break,
                    Some(AsrEvent::Partial { text, seq }) => {
                        eprintln!("[shuo]   partial#{seq} (drain): {text}");
                    }
                    Some(AsrEvent::Error { err }) => {
                        eprintln!("[shuo] ❌ ASR error during drain: {err}");
                    }
                }
            }
        }
    }

    rec.stop();
    while let Some(samples) = rec.try_recv() {
        let _ = session.send_pcm(&samples, false).await;
    }

    if let Err(e) = session.send_pcm(&[], true).await {
        eprintln!("[shuo] ❌ send is_last failed: {e}");
        return;
    }

    let timeout = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            biased;
            _ = &mut timeout => {
                eprintln!("[shuo] ⚠ 识别 final 超时 5s");
                return;
            }
            ev = events.recv() => {
                match ev {
                    None => return,
                    Some(AsrEvent::Done) => return,
                    Some(AsrEvent::Segment { text, .. }) => {
                        eprintln!("[shuo]   segment (final): {text}");
                        pending_segments.push(text);
                    }
                    Some(AsrEvent::Partial { text, seq }) => {
                        eprintln!("[shuo]   partial#{seq} (final): {text}");
                    }
                    Some(AsrEvent::Error { err }) => {
                        eprintln!("[shuo] ❌ ASR error during final: {err}");
                    }
                }
            }
        }
    }
}

async fn dispatch_with_filler(mut segments: Vec<String>, sep: &str, auto_paste: bool) {
    let raw_text = segments.join(sep);
    if raw_text.is_empty() {
        eprintln!("[shuo] (空识别结果，跳过 dispatch)");
        return;
    }
    let chain: Vec<Box<dyn post::PostProcessor>> =
        vec![Box::new(RuleBasedFiller::default_patterns())];
    let initial = PipelineText::new(raw_text, std::mem::take(&mut segments));
    let out = post::run_chain(&chain, initial, &post::AppContext, Duration::from_secs(2)).await;
    eprintln!("[shuo] ✓ 最终: {}", out.text);
    if let Err(e) = dispatch::dispatch(&out.text, auto_paste) {
        eprintln!("[shuo] ❌ 剪贴板写入失败: {e:#}");
    }
}

fn prepare_audio_path(recording_id: &str) -> anyhow::Result<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        PathBuf::from(xdg).join("shuohua/audio")
    } else {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".local/state/shuohua/audio")
    };
    std::fs::create_dir_all(&base)?;
    Ok(base.join(format!("{recording_id}.wav")))
}
