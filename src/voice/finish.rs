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
use crate::overlay::{OverlayCmd, OverlayHandle, OverlayState, TextKind, ToastLevel};
use crate::post::{self, PipelineStepStatus, PipelineText, RuleBasedFiller};
use crate::state::StateStore;
use crate::state::history::{
    self, AsrHistory, AsrSessionHistory, HistoryError, HistoryRecord, HistoryStatus,
    PipelineStepHistory, PipelineStepStatus as HistoryPipelineStepStatus,
};
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
    pub overlay: Option<OverlayHandle>,
    pub state: StateStore,
}

pub async fn run_recording(
    provider: &dyn AsrProvider,
    params: SessionParams,
    stop_rx: oneshot::Receiver<()>,
) {
    let recording_id = ulid::Ulid::new().to_string();
    let recording_started_at = time::OffsetDateTime::now_utc();
    let recording_started_instant = Instant::now();
    eprintln!("[shuo] ▶ recording id={recording_id}");
    params.state.set_recording(recording_id.clone());
    overlay_send(&params, OverlayCmd::SetState { state: OverlayState::Connecting });
    overlay_send(
        &params,
        OverlayCmd::SetApp {
            bundle_id: None,
            app_name: None,
            chain_summary: "filler".to_string(),
        },
    );

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
        hotwords: params.hotwords.clone(),
    };
    let (mut session, mut events) = match provider.open(ctx).await {
        Ok(t) => t,
        Err(err) => {
            eprintln!("[shuo] ❌ ASR open failed: {err}");
            rec.stop();
            params.state.set_error(Some(recording_id));
            overlay_send(&params, OverlayCmd::SetState { state: OverlayState::Error });
            overlay_send(
                &params,
                OverlayCmd::Toast {
                    text: err.to_string(),
                    level: ToastLevel::Error,
                    ttl_ms: 1500,
                },
            );
            return;
        }
    };
    overlay_send(&params, OverlayCmd::SetState { state: OverlayState::Recording });

    let mut stop_rx = stop_rx;
    let mut pending_segments: Vec<SegmentCapture> = Vec::new();
    let mut audio_samples_sent: u64 = 0;
    let mut stop_requested = false;
    let mut terminal_error: Option<HistoryError> = None;

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
                            terminal_error = Some(HistoryError {
                                kind: "asr_send".to_string(),
                                msg: e.to_string(),
                            });
                            break;
                        }
                        audio_samples_sent += samples.len() as u64;
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
                        params.state.partial(recording_id.clone(), text.clone());
                        overlay_send(
                            &params,
                            OverlayCmd::SetStats {
                                dur_ms: recording_started_instant.elapsed().as_millis() as u64,
                                chars: text.chars().count() as u32,
                            },
                        );
                        overlay_send(
                            &params,
                            OverlayCmd::SetText { text, kind: TextKind::Partial },
                        );
                    }
                    Some(AsrEvent::Segment { text, started_at, ended_at }) => {
                        eprintln!("[shuo]   segment: {text}");
                        overlay_send(&params, OverlayCmd::AppendSegment { text: text.clone() });
                        pending_segments.push(SegmentCapture { text, started_at, ended_at });
                    }
                    Some(AsrEvent::Error { err }) => {
                        eprintln!("[shuo] ❌ ASR error: {err}");
                        params.state.set_error(Some(recording_id.clone()));
                        overlay_send(&params, OverlayCmd::SetState { state: OverlayState::Error });
                        overlay_send(
                            &params,
                            OverlayCmd::Toast {
                                text: err.to_string(),
                                level: ToastLevel::Error,
                                ttl_ms: 1500,
                            },
                        );
                        rec.stop();
                        let _ = session.close().await;
                        let ended_at = time::OffsetDateTime::now_utc();
                        if let Some(record) = append_history(HistoryInput {
                            id: recording_id,
                            provider: provider.name().to_string(),
                            started_at: recording_started_at,
                            ended_at,
                            started_instant: recording_started_instant,
                            raw_text: pending_segments
                                .iter()
                                .map(|s| s.text.as_str())
                                .collect::<Vec<_>>()
                                .join(&params.segment_separator),
                            segments: pending_segments,
                            pipeline: Vec::new(),
                            audio_ms: samples_to_ms(audio_samples_sent),
                            status: HistoryStatus::Error,
                            error: Some(HistoryError {
                                kind: "asr_error".to_string(),
                                msg: err.to_string(),
                            }),
                        }) {
                            params.state.history_appended(record);
                        }
                        return;
                    }
                    Some(AsrEvent::Done) => break,
                }
            }
        }

        if stop_requested {
            overlay_send(&params, OverlayCmd::SetState { state: OverlayState::Stopping });
            params.state.set_stopping(recording_id.clone());
            finish(
                &mut rec,
                &mut session,
                &mut events,
                &mut pending_segments,
                params.stop_delay_ms,
                &mut audio_samples_sent,
                &mut terminal_error,
            )
            .await;
            break;
        }
    }

    let _ = session.close().await;
    let provider_name = provider.name().to_string();
    let raw_text = pending_segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(&params.segment_separator);
    let (pipeline, status, error) =
        dispatch_with_filler(&pending_segments, &params.segment_separator, params.auto_paste)
            .await
            .unwrap_or_else(|err| (Vec::new(), HistoryStatus::Error, Some(err)));
    if let Some(last_text) = pipeline.iter().rev().find_map(|step| step.text.clone()) {
        overlay_send(&params, OverlayCmd::SetText { text: last_text, kind: TextKind::Final });
    }
    for step in &pipeline {
        params.state.pipeline_step(recording_id.clone(), step.clone());
    }
    if let Some(err) = terminal_error.as_ref().or(error.as_ref()) {
        params.state.set_error(Some(recording_id.clone()));
        overlay_send(&params, OverlayCmd::SetState { state: OverlayState::Error });
        overlay_send(
            &params,
            OverlayCmd::Toast { text: err.msg.clone(), level: ToastLevel::Error, ttl_ms: 1500 },
        );
    } else {
        params.state.set_idle();
        overlay_send(&params, OverlayCmd::SetState { state: OverlayState::Idle });
        overlay_send(&params, OverlayCmd::Hide);
    }
    let ended_at = time::OffsetDateTime::now_utc();
    if let Some(record) = append_history(HistoryInput {
        id: recording_id,
        provider: provider_name,
        started_at: recording_started_at,
        ended_at,
        started_instant: recording_started_instant,
        raw_text,
        segments: pending_segments,
        pipeline,
        audio_ms: samples_to_ms(audio_samples_sent),
        status: terminal_error
            .as_ref()
            .map(|_| HistoryStatus::Error)
            .unwrap_or(status),
        error: terminal_error.or(error),
    }) {
        params.state.history_appended(record);
    }
}

async fn finish(
    rec: &mut recorder::RecordingStream,
    session: &mut Box<dyn AsrSession>,
    events: &mut mpsc::Receiver<AsrEvent>,
    pending_segments: &mut Vec<SegmentCapture>,
    stop_delay_ms: u32,
    audio_samples_sent: &mut u64,
    terminal_error: &mut Option<HistoryError>,
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
                        *audio_samples_sent += samples.len() as u64;
                    }
                    None => break,
                }
            }
            ev = events.recv() => {
                match ev {
                    None => break,
                    Some(AsrEvent::Segment { text, started_at, ended_at }) => {
                        eprintln!("[shuo]   segment (drain): {text}");
                        pending_segments.push(SegmentCapture { text, started_at, ended_at });
                    }
                    Some(AsrEvent::Done) => break,
                    Some(AsrEvent::Partial { text, seq }) => {
                        eprintln!("[shuo]   partial#{seq} (drain): {text}");
                    }
                    Some(AsrEvent::Error { err }) => {
                        eprintln!("[shuo] ❌ ASR error during drain: {err}");
                        *terminal_error = Some(HistoryError {
                            kind: "asr_error".to_string(),
                            msg: err.to_string(),
                        });
                    }
                }
            }
        }
    }

    rec.stop();
    while let Some(samples) = rec.try_recv() {
        let _ = session.send_pcm(&samples, false).await;
        *audio_samples_sent += samples.len() as u64;
    }

    if let Err(e) = session.send_pcm(&[], true).await {
        eprintln!("[shuo] ❌ send is_last failed: {e}");
        *terminal_error = Some(HistoryError {
            kind: "asr_send_last".to_string(),
            msg: e.to_string(),
        });
        return;
    }

    let timeout = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            biased;
            _ = &mut timeout => {
                eprintln!("[shuo] ⚠ 识别 final 超时 5s");
                *terminal_error = Some(HistoryError {
                    kind: "asr_timeout".to_string(),
                    msg: "timeout waiting final".to_string(),
                });
                return;
            }
            ev = events.recv() => {
                match ev {
                    None => return,
                    Some(AsrEvent::Done) => return,
                    Some(AsrEvent::Segment { text, started_at, ended_at }) => {
                        eprintln!("[shuo]   segment (final): {text}");
                        pending_segments.push(SegmentCapture { text, started_at, ended_at });
                    }
                    Some(AsrEvent::Partial { text, seq }) => {
                        eprintln!("[shuo]   partial#{seq} (final): {text}");
                    }
                    Some(AsrEvent::Error { err }) => {
                        eprintln!("[shuo] ❌ ASR error during final: {err}");
                        *terminal_error = Some(HistoryError {
                            kind: "asr_error".to_string(),
                            msg: err.to_string(),
                        });
                    }
                }
            }
        }
    }
}

async fn dispatch_with_filler(
    segments: &[SegmentCapture],
    sep: &str,
    auto_paste: bool,
) -> Result<(Vec<PipelineStepHistory>, HistoryStatus, Option<HistoryError>), HistoryError> {
    let segment_texts: Vec<String> = segments.iter().map(|s| s.text.clone()).collect();
    let raw_text = segment_texts.join(sep);
    if raw_text.is_empty() {
        eprintln!("[shuo] (空识别结果，跳过 dispatch)");
        return Ok((Vec::new(), HistoryStatus::Canceled, None));
    }
    let chain: Vec<Box<dyn post::PostProcessor>> =
        vec![Box::new(RuleBasedFiller::default_patterns())];
    let initial = PipelineText::new(raw_text, segment_texts);
    let (out, steps) =
        post::run_chain(&chain, initial, &post::AppContext, Duration::from_secs(2)).await;
    eprintln!("[shuo] ✓ 最终: {}", out.text);
    let pipeline = steps.into_iter().map(PipelineStepHistory::from).collect();
    if let Err(e) = dispatch::dispatch(&out.text, auto_paste) {
        eprintln!("[shuo] ❌ 剪贴板写入失败: {e:#}");
        return Ok((
            pipeline,
            HistoryStatus::Error,
            Some(HistoryError { kind: "dispatch".to_string(), msg: format!("{e:#}") }),
        ));
    }
    Ok((pipeline, HistoryStatus::Submitted, None))
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

fn overlay_send(params: &SessionParams, cmd: OverlayCmd) {
    if let Some(overlay) = &params.overlay {
        overlay.send(cmd);
    }
}

#[derive(Debug, Clone)]
struct SegmentCapture {
    text: String,
    started_at: Instant,
    ended_at: Instant,
}

struct HistoryInput {
    id: String,
    provider: String,
    started_at: time::OffsetDateTime,
    ended_at: time::OffsetDateTime,
    started_instant: Instant,
    raw_text: String,
    segments: Vec<SegmentCapture>,
    pipeline: Vec<PipelineStepHistory>,
    audio_ms: u64,
    status: HistoryStatus,
    error: Option<HistoryError>,
}

fn append_history(input: HistoryInput) -> Option<HistoryRecord> {
    let record = HistoryRecord {
        version: 1,
        id: input.id,
        started_at: input.started_at,
        ended_at: input.ended_at,
        duration_ms: (input.ended_at - input.started_at).whole_milliseconds().max(0) as u64,
        status: input.status,
        app: None,
        asr: AsrHistory {
            provider: input.provider,
            raw: input.raw_text,
            audio_ms: input.audio_ms,
            sessions: input
                .segments
                .into_iter()
                .map(|s| AsrSessionHistory {
                    text: s.text,
                    started_at: instant_to_datetime(
                        input.started_at,
                        input.started_instant,
                        s.started_at,
                    ),
                    ended_at: instant_to_datetime(
                        input.started_at,
                        input.started_instant,
                        s.ended_at,
                    ),
                })
                .collect(),
        },
        pipeline: input.pipeline,
        error: input.error,
    };
    if let Err(e) = history::append_default(&record) {
        eprintln!("[shuo] ❌ history append failed: {e:#}");
        return None;
    }
    Some(record)
}

fn instant_to_datetime(
    recording_started_at: time::OffsetDateTime,
    recording_started_instant: Instant,
    instant: Instant,
) -> time::OffsetDateTime {
    let delta = instant.saturating_duration_since(recording_started_instant);
    recording_started_at + time::Duration::milliseconds(delta.as_millis() as i64)
}

fn samples_to_ms(samples: u64) -> u64 {
    samples.saturating_mul(1000) / 16_000
}

impl From<post::PipelineStep> for PipelineStepHistory {
    fn from(step: post::PipelineStep) -> Self {
        Self {
            name: step.name,
            status: match step.status {
                PipelineStepStatus::Ok => HistoryPipelineStepStatus::Ok,
                PipelineStepStatus::Error => HistoryPipelineStepStatus::Error,
                PipelineStepStatus::Timeout => HistoryPipelineStepStatus::Timeout,
                PipelineStepStatus::Skipped => HistoryPipelineStepStatus::Skipped,
            },
            duration_ms: step.duration_ms,
            text: step.text,
            error: step.error,
        }
    }
}
