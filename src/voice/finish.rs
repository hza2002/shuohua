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
use crate::overlay::{OverlayCmd, OverlayHandle, OverlayState, TextKind};
use crate::post::{self, PipelineStepStatus, PipelineText};
use crate::state::history::{
    self, AsrHistory, AsrSessionHistory, HistoryError, HistoryRecord, HistoryStatus,
    PipelineStepHistory, PipelineStepStatus as HistoryPipelineStepStatus,
};
use crate::state::StateStore;
use crate::voice::trace::{TraceRecorder, TraceStart};
use crate::voice::{dispatch, recorder, SessionControl};
use std::path::PathBuf;
use tokio::sync::{mpsc, watch};
use tokio::time::{sleep_until, Instant as TokioInstant};

/// 录音开始后等"非零样本"出现的硬上限。
///
/// macOS 没有可靠的"麦克风一定能产生数据"的预检（合盖、被其他进程独占、
/// 设备名匹配 builtin 但其实是别的，都会假阳/假阴）。所以放弃事前探测，
/// 改成运行时 watchdog：开了 cpal 之后 1s 内没看见任何**非零** PCM 样本
/// 就当作不可用。
///
/// 为什么不是"任意一帧"：合盖时 macOS 仍然驱动 AudioQueue 推 callback，
/// 只是塞 0 帧（默认输入设备被屏蔽）。所以 has-any-frame 检测不到合盖，
/// 必须看样本值本身。
///
/// 阈值故意不进配置：超过这个时间几乎可以确定是设备问题，不是用户该调的旋钮。
const FIRST_AUDIO_TIMEOUT_MS: u64 = 1000;

/// 区分"真静音"和"真信号"的样本振幅门槛。约 -72 dBFS，远低于消费级麦克风
/// 在安静房间的本底噪声（约 -50 ~ -60 dBFS，对应 i16 ≈ 32~100），同时严格
/// 大于"完美零"（合盖 / 设备被屏蔽时的 silence buffer 是精确 0）。
const MIN_NONZERO_AMPLITUDE: i16 = 8;

/// 当前 PCM 帧里是否至少有一个样本超过静音阈值。
fn frame_has_signal(samples: &[i16]) -> bool {
    samples
        .iter()
        .any(|s| s.unsigned_abs() > MIN_NONZERO_AMPLITUDE as u16)
}

/// 非阻断 warn 在 meta 行上显示多久。跟 overlay 的 ERROR_TTL_MS 对齐，
/// 心智一致：错误/警告都 3s。
const NOTICE_TTL_MS: u32 = 3000;

/// 把 overlay 切到 Error 终态：状态字红色 icon + text 区显示错误文案（红字）。
/// view 的 tick 会在 ERROR_TTL_MS 后自动 hide。所有 error 路径走这一条。
fn send_error_overlay(params: &SessionParams, message: String) {
    overlay_send(
        params,
        OverlayCmd::SetState {
            state: OverlayState::Error,
        },
    );
    overlay_send(
        params,
        OverlayCmd::SetText {
            text: message,
            kind: TextKind::Error,
        },
    );
}

pub struct SessionParams {
    pub auto_paste: bool,
    pub record_audio: bool,
    pub vad_trace: bool,
    /// M10：provider 私有，源于 `asr/<provider>.toml.idle_pause`。
    /// false 时 voice 不做多 session 切分，保持 M9 单 session 行为。
    pub idle_pause: bool,
    /// M10：provider 私有，源于 `asr/<provider>.toml.finalize_timeout_ms`。
    /// voice 发出 `is_last=true` 后等 final segment / Done 的最大毫秒。
    pub finalize_timeout_ms: u64,
    /// M10：全局 voice VAD 配置；`backend = Off` 时 voice 不启用本地 VAD。
    pub vad: crate::config::VoiceVadCfg,
    pub stop_delay_ms: u32,
    pub hotwords: Vec<String>,
    pub start_app_context: post::AppContext,
    pub post_chain: post::config::PostChain,
    pub post_timeout_ms: u64,
    pub overlay: Option<OverlayHandle>,
    pub state: StateStore,
}

pub async fn run_recording(
    provider: &dyn AsrProvider,
    params: SessionParams,
    mut control_rx: watch::Receiver<SessionControl>,
) {
    let recording_id = ulid::Ulid::new().to_string();
    let recording_started_at = time::OffsetDateTime::now_utc();
    let recording_started_instant = Instant::now();
    crate::debug_println!("[shuo] ▶ recording id={recording_id}");
    let mut trace = TraceRecorder::start(TraceStart {
        enabled: params.vad_trace,
        recording_id: recording_id.clone(),
        provider: provider.name().to_string(),
        started_at: recording_started_at.to_string(),
    });
    let mut app_context = params.start_app_context.clone();
    params
        .state
        .set_recording(recording_id.clone(), recording_started_at);
    params
        .state
        .app(app_context.bundle_id.clone(), app_context.app_name.clone());
    overlay_send(
        &params,
        OverlayCmd::SetState {
            state: OverlayState::Connecting,
        },
    );
    overlay_send(
        &params,
        OverlayCmd::SetApp {
            bundle_id: app_context.bundle_id.clone(),
            app_name: app_context.app_name.clone(),
            chain_summary: params.post_chain.name.clone(),
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
        crate::debug_println!("[shuo] 留存 wav → {}", p.display());
    }

    let mut rec = match recorder::start(audio_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[shuo] ❌ 录音启动失败: {e:#}");
            trace.finish("recorder_start_error", 0);
            params.state.set_error(Some(recording_id));
            send_error_overlay(&params, crate::t!("error.recorder_start"));
            return;
        }
    };
    // deadline 从 cpal stream play 成功这一刻起算。ASR open 期间 cpal callback
    // 已经在底层线程往 mpsc 推帧，所以这个倒计时跟 provider.open() 真正并行：
    // 正常麦克风的话，进 select 时帧早就在 channel 里，pcm 分支会先 ready；
    // 没声音的话，1s 后 sleep_until 分支 fire 并退出。
    let first_audio_deadline = TokioInstant::now() + Duration::from_millis(FIRST_AUDIO_TIMEOUT_MS);
    let mut first_audio_seen = false;

    let ctx = SessionCtx {
        language: LanguageMode::Multilingual {
            hint: vec!["zh-CN".into(), "en-US".into()],
        },
        hotwords: params.hotwords.clone(),
    };
    let (mut session, mut events) = match provider.open(ctx).await {
        Ok(t) => t,
        Err(err) => {
            eprintln!("[shuo] ❌ ASR open failed: {err}");
            rec.stop();
            trace.asr_error(
                instant_elapsed_ms(recording_started_instant),
                &err.to_string(),
            );
            trace.finish("asr_open_error", 0);
            params.state.set_error(Some(recording_id));
            send_error_overlay(&params, crate::t!("error.asr_open"));
            return;
        }
    };
    overlay_send(
        &params,
        OverlayCmd::SetState {
            state: OverlayState::Recording,
        },
    );
    trace.provider_opened(instant_elapsed_ms(recording_started_instant));

    let mut pending_segments: Vec<SegmentCapture> = Vec::new();
    let mut audio_samples_sent: u64 = 0;
    let mut stop_requested = false;
    let mut cancel_requested = false;
    let mut terminal_error: Option<HistoryError> = None;

    loop {
        tokio::select! {
            biased;
            _ = control_rx.changed() => {
                match *control_rx.borrow_and_update() {
                    SessionControl::Stop => {
                        stop_requested = true;
                    }
                    SessionControl::Cancel => {
                        cancel_requested = true;
                        break;
                    }
                    SessionControl::Idle => {}
                }
            }
            pcm = rec.recv(), if !stop_requested => {
                match pcm {
                    Some(samples) => {
                        trace.pcm_frame(&samples);
                        if !first_audio_seen && frame_has_signal(&samples) {
                            first_audio_seen = true;
                        }
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
            _ = sleep_until(first_audio_deadline), if !first_audio_seen => {
                eprintln!(
                    "[shuo] ❌ {FIRST_AUDIO_TIMEOUT_MS}ms 内未收到麦克风音频，疑似设备不可用"
                );
                rec.stop();
                let _ = session.close().await;
                trace.finish("no_audio", samples_to_ms(audio_samples_sent));
                params.state.set_error(Some(recording_id));
                send_error_overlay(&params, crate::t!("error.no_audio"));
                return;
            }
            ev = events.recv() => {
                match ev {
                    None => break,
                    Some(AsrEvent::Partial { text, seq }) => {
                        crate::debug_println!("[shuo]   partial#{seq}: {text}");
                        trace.asr_partial(instant_elapsed_ms(recording_started_instant), seq, &text);
                        params.state.partial(recording_id.clone(), text.clone());
                        let live_text = format!(
                            "{}{}",
                            pending_segments
                                .iter()
                                .map(|segment| segment.text.as_str())
                                .collect::<String>(),
                            text
                        );
                        let words = crate::text_stats::compute(&live_text).words as u32;
                        let dur_ms = recording_started_instant.elapsed().as_millis() as u64;
                        params.state.stats(dur_ms, words);
                        overlay_send(&params, OverlayCmd::SetStats { dur_ms, words });
                        overlay_send(
                            &params,
                            OverlayCmd::SetText { text, kind: TextKind::Partial },
                        );
                    }
                    Some(AsrEvent::Segment { text, started_at, ended_at }) => {
                        crate::debug_println!("[shuo]   segment: {text}");
                        trace.asr_segment(
                            instant_elapsed_ms(recording_started_instant),
                            &text,
                            instant_to_ms(recording_started_instant, started_at),
                            instant_to_ms(recording_started_instant, ended_at),
                        );
                        params.state.segment(recording_id.clone(), text.clone());
                        overlay_send(&params, OverlayCmd::AppendSegment { text: text.clone() });
                        pending_segments.push(SegmentCapture { text, started_at, ended_at });
                    }
                    Some(AsrEvent::Error { err }) => {
                        eprintln!("[shuo] ❌ ASR error: {err}");
                        trace.asr_error(instant_elapsed_ms(recording_started_instant), &err.to_string());
                        params.state.set_error(Some(recording_id.clone()));
                        send_error_overlay(&params, crate::t!("error.asr_runtime"));
                        rec.stop();
                        let _ = session.close().await;
                        let ended_at = time::OffsetDateTime::now_utc();
                        let asr_text: String = pending_segments
                            .iter()
                            .map(|s| s.text.as_str())
                            .collect();
                        if let Some(record) = append_history(HistoryInput {
                            id: recording_id,
                            provider: provider.name().to_string(),
                            started_at: recording_started_at,
                            ended_at,
                            started_instant: recording_started_instant,
                            asr_text: asr_text.clone(),
                            final_text: asr_text,
                            segments: pending_segments,
                            pipeline: Vec::new(),
                            audio_ms: samples_to_ms(audio_samples_sent),
                            app: app_context.bundle_id.clone(),
                            status: HistoryStatus::Error,
                            error: Some(HistoryError {
                                kind: "asr_error".to_string(),
                                msg: err.to_string(),
                            }),
                        }) {
                            params.state.history_appended(record);
                        }
                        trace.finish("asr_error", samples_to_ms(audio_samples_sent));
                        return;
                    }
                    Some(AsrEvent::Done) => {
                        trace.asr_done(instant_elapsed_ms(recording_started_instant));
                        break;
                    }
                }
            }
        }

        if cancel_requested {
            break;
        }
        if stop_requested {
            app_context = post::app_context::frontmost_app();
            params
                .state
                .app(app_context.bundle_id.clone(), app_context.app_name.clone());
            overlay_send(
                &params,
                OverlayCmd::SetApp {
                    bundle_id: app_context.bundle_id.clone(),
                    app_name: app_context.app_name.clone(),
                    chain_summary: params.post_chain.name.clone(),
                },
            );
            overlay_send(
                &params,
                OverlayCmd::SetState {
                    state: OverlayState::Stopping,
                },
            );
            params.state.set_stopping(recording_id.clone());
            finish(
                &mut rec,
                &mut session,
                &mut events,
                &mut pending_segments,
                &params.state,
                &recording_id,
                params.stop_delay_ms,
                params.finalize_timeout_ms,
                &mut control_rx,
                &mut audio_samples_sent,
                &mut terminal_error,
                &mut trace,
                recording_started_instant,
            )
            .await;
            if terminal_error.is_none() && control_rx.borrow().eq(&SessionControl::Cancel) {
                cancel_requested = true;
            }
            break;
        }
    }

    if cancel_requested {
        cancel_session(&mut rec, &mut session, &mut audio_samples_sent).await;
    }

    let _ = session.close().await;
    let provider_name = provider.name().to_string();
    let raw_text = pending_segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<String>();
    if cancel_requested {
        crate::debug_println!("[shuo] ✖ recording canceled");
        params.state.set_idle();
        overlay_send(&params, OverlayCmd::Hide);
        if let Some(record) = append_history(HistoryInput {
            id: recording_id,
            provider: provider_name,
            started_at: recording_started_at,
            ended_at: time::OffsetDateTime::now_utc(),
            started_instant: recording_started_instant,
            asr_text: raw_text.clone(),
            final_text: raw_text,
            segments: pending_segments,
            pipeline: Vec::new(),
            audio_ms: samples_to_ms(audio_samples_sent),
            app: app_context.bundle_id,
            status: HistoryStatus::Canceled,
            error: None,
        }) {
            params.state.history_appended(record);
        }
        trace.finish("canceled", samples_to_ms(audio_samples_sent));
        return;
    }
    // terminal_error 路径：录音 / ASR 中途崩溃 → 不跑 post chain、不写剪贴板。
    // 理由（M7 决策）：半成品上屏会误导（用户以为成功），auto_paste 还可能粘到
    // 已经切走的应用；history.jsonl 保留所有 segments，需要的用户从 TUI 回捞。
    let (final_text, pipeline, status, error) = if terminal_error.is_some() {
        (raw_text.clone(), Vec::new(), HistoryStatus::Error, None)
    } else {
        dispatch_with_post_chain(&pending_segments, params.auto_paste, &app_context, &params)
            .await
            .unwrap_or_else(|err| {
                (
                    raw_text.clone(),
                    Vec::new(),
                    HistoryStatus::Error,
                    Some(err),
                )
            })
    };
    for step in &pipeline {
        params
            .state
            .pipeline_step(recording_id.clone(), step.clone());
    }
    if terminal_error.is_some() || error.is_some() {
        // dispatch 失败 vs 录音中途失败用不同文案；剪贴板失败明确告知用户。
        let msg = if terminal_error.is_some() {
            crate::t!("error.asr_runtime")
        } else {
            crate::t!("error.dispatch")
        };
        params.state.set_error(Some(recording_id.clone()));
        send_error_overlay(&params, msg);
    } else {
        if let Some(last_text) = pipeline.iter().rev().find_map(|step| step.text.clone()) {
            overlay_send(
                &params,
                OverlayCmd::SetText {
                    text: last_text,
                    kind: TextKind::Final,
                },
            );
        }
        params.state.set_idle();
        // 不发 SetState{Idle}：它会让 model.visible=false，渲染时立刻 orderOut
        // panel，把 notice 在显示前就关掉。Hide 内部已经把 state 切回 Idle，
        // 而且 notice 活着时会延期到 ttl 到点再真隐藏。
        overlay_send(&params, OverlayCmd::Hide);
    }
    let ended_at = time::OffsetDateTime::now_utc();
    let history_status = terminal_error
        .as_ref()
        .map(|_| HistoryStatus::Error)
        .unwrap_or(status);
    let trace_status = if terminal_error.is_some() || error.is_some() {
        "error"
    } else {
        match status {
            HistoryStatus::Submitted => "submitted",
            HistoryStatus::Canceled => "canceled",
            HistoryStatus::Error => "error",
            HistoryStatus::Timeout => "timeout",
        }
    };
    if let Some(record) = append_history(HistoryInput {
        id: recording_id,
        provider: provider_name,
        started_at: recording_started_at,
        ended_at,
        started_instant: recording_started_instant,
        asr_text: raw_text,
        final_text,
        segments: pending_segments,
        pipeline,
        audio_ms: samples_to_ms(audio_samples_sent),
        app: app_context.bundle_id,
        status: history_status,
        error: terminal_error.or(error),
    }) {
        params.state.history_appended(record);
    }
    trace.finish(trace_status, samples_to_ms(audio_samples_sent));
}

async fn finish(
    rec: &mut recorder::RecordingStream,
    session: &mut Box<dyn AsrSession>,
    events: &mut mpsc::Receiver<AsrEvent>,
    pending_segments: &mut Vec<SegmentCapture>,
    state: &crate::state::StateStore,
    recording_id: &str,
    stop_delay_ms: u32,
    finalize_timeout_ms: u64,
    control_rx: &mut watch::Receiver<SessionControl>,
    audio_samples_sent: &mut u64,
    terminal_error: &mut Option<HistoryError>,
    trace: &mut TraceRecorder,
    recording_started_instant: Instant,
) -> bool {
    let drain_until = Instant::now() + Duration::from_millis(stop_delay_ms as u64);
    loop {
        let now = Instant::now();
        if now >= drain_until {
            break;
        }
        let deadline = tokio::time::Instant::from_std(drain_until);
        tokio::select! {
            biased;
            _ = control_rx.changed() => {
                if matches!(*control_rx.borrow_and_update(), SessionControl::Cancel) {
                    cancel_session(rec, session, audio_samples_sent).await;
                    return true;
                }
            }
            _ = sleep_until(deadline) => break,
            pcm = rec.recv() => {
                match pcm {
                    Some(samples) => {
                        trace.pcm_frame(&samples);
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
                        crate::debug_println!("[shuo]   segment (drain): {text}");
                        trace.asr_segment(
                            instant_elapsed_ms(recording_started_instant),
                            &text,
                            instant_to_ms(recording_started_instant, started_at),
                            instant_to_ms(recording_started_instant, ended_at),
                        );
                        state.segment(recording_id.to_string(), text.clone());
                        pending_segments.push(SegmentCapture { text, started_at, ended_at });
                    }
                    Some(AsrEvent::Done) => {
                        trace.asr_done(instant_elapsed_ms(recording_started_instant));
                        break;
                    }
                    Some(AsrEvent::Partial { text, seq }) => {
                        crate::debug_println!("[shuo]   partial#{seq} (drain): {text}");
                        trace.asr_partial(instant_elapsed_ms(recording_started_instant), seq, &text);
                    }
                    Some(AsrEvent::Error { err }) => {
                        eprintln!("[shuo] ❌ ASR error during drain: {err}");
                        trace.asr_error(instant_elapsed_ms(recording_started_instant), &err.to_string());
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
        trace.pcm_frame(&samples);
        let _ = session.send_pcm(&samples, false).await;
        *audio_samples_sent += samples.len() as u64;
    }

    if let Err(e) = session.send_pcm(&[], true).await {
        eprintln!("[shuo] ❌ send is_last failed: {e}");
        *terminal_error = Some(HistoryError {
            kind: "asr_send_last".to_string(),
            msg: e.to_string(),
        });
        return false;
    }

    let timeout = tokio::time::sleep(Duration::from_millis(finalize_timeout_ms));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            biased;
            _ = control_rx.changed() => {
                if matches!(*control_rx.borrow_and_update(), SessionControl::Cancel) {
                    cancel_session(rec, session, audio_samples_sent).await;
                    return true;
                }
            }
            _ = &mut timeout => {
                eprintln!("[shuo] ⚠ 识别 final 超时 {finalize_timeout_ms}ms");
                *terminal_error = Some(HistoryError {
                    kind: "asr_timeout".to_string(),
                    msg: "timeout waiting final".to_string(),
                });
                return false;
            }
            ev = events.recv() => {
                match ev {
                    None => return false,
                    Some(AsrEvent::Done) => {
                        trace.asr_done(instant_elapsed_ms(recording_started_instant));
                        return false;
                    }
                    Some(AsrEvent::Segment { text, started_at, ended_at }) => {
                        crate::debug_println!("[shuo]   segment (final): {text}");
                        trace.asr_segment(
                            instant_elapsed_ms(recording_started_instant),
                            &text,
                            instant_to_ms(recording_started_instant, started_at),
                            instant_to_ms(recording_started_instant, ended_at),
                        );
                        state.segment(recording_id.to_string(), text.clone());
                        pending_segments.push(SegmentCapture { text, started_at, ended_at });
                    }
                    Some(AsrEvent::Partial { text, seq }) => {
                        crate::debug_println!("[shuo]   partial#{seq} (final): {text}");
                        trace.asr_partial(instant_elapsed_ms(recording_started_instant), seq, &text);
                    }
                    Some(AsrEvent::Error { err }) => {
                        eprintln!("[shuo] ❌ ASR error during final: {err}");
                        trace.asr_error(instant_elapsed_ms(recording_started_instant), &err.to_string());
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

async fn cancel_session(
    rec: &mut recorder::RecordingStream,
    session: &mut Box<dyn AsrSession>,
    audio_samples_sent: &mut u64,
) {
    rec.stop();
    while let Some(samples) = rec.try_recv() {
        let _ = session.send_pcm(&samples, false).await;
        *audio_samples_sent += samples.len() as u64;
    }
}

async fn dispatch_with_post_chain(
    segments: &[SegmentCapture],
    auto_paste: bool,
    app_context: &post::AppContext,
    params: &SessionParams,
) -> Result<
    (
        String,
        Vec<PipelineStepHistory>,
        HistoryStatus,
        Option<HistoryError>,
    ),
    HistoryError,
> {
    let segment_texts: Vec<String> = segments.iter().map(|s| s.text.clone()).collect();
    let raw_text: String = segment_texts.concat();
    if raw_text.is_empty() {
        crate::debug_println!("[shuo] (空识别结果，跳过 dispatch)");
        return Ok((String::new(), Vec::new(), HistoryStatus::Canceled, None));
    }
    let initial = PipelineText::new(raw_text, segment_texts);
    overlay_send(
        params,
        OverlayCmd::SetState {
            state: OverlayState::Thinking,
        },
    );
    let (out, steps) = post::run_chain(
        &params.post_chain.processors,
        initial,
        app_context,
        Duration::from_millis(params.post_timeout_ms),
    )
    .await;
    for step in &steps {
        match step.status {
            PipelineStepStatus::Error | PipelineStepStatus::Timeout => {
                let text = match step.status {
                    PipelineStepStatus::Timeout => {
                        crate::t!("notice.step_timeout", name = step.name)
                    }
                    _ => crate::t!("notice.step_failed", name = step.name),
                };
                overlay_send(
                    params,
                    OverlayCmd::Notice {
                        text,
                        ttl_ms: NOTICE_TTL_MS,
                    },
                );
            }
            PipelineStepStatus::Ok | PipelineStepStatus::Skipped => {}
        }
    }
    crate::debug_println!("[shuo] ✓ 最终: {}", out.text);
    let dispatched = out.text.clone();
    let pipeline = steps.into_iter().map(PipelineStepHistory::from).collect();
    if let Err(e) = dispatch::dispatch(&out.text, auto_paste) {
        eprintln!("[shuo] ❌ 剪贴板写入失败: {e:#}");
        return Ok((
            dispatched,
            pipeline,
            HistoryStatus::Error,
            Some(HistoryError {
                kind: "dispatch".to_string(),
                msg: format!("{e:#}"),
            }),
        ));
    }
    Ok((dispatched, pipeline, HistoryStatus::Submitted, None))
}

fn prepare_audio_path(recording_id: &str) -> anyhow::Result<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        PathBuf::from(xdg).join("shuohua/audio")
    } else {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state/shuohua/audio")
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
    asr_text: String,
    final_text: String,
    segments: Vec<SegmentCapture>,
    pipeline: Vec<PipelineStepHistory>,
    audio_ms: u64,
    app: Option<String>,
    status: HistoryStatus,
    error: Option<HistoryError>,
}

fn build_record(input: HistoryInput) -> HistoryRecord {
    let session_started_at = input
        .segments
        .first()
        .map(|s| instant_to_datetime(input.started_at, input.started_instant, s.started_at))
        .unwrap_or(input.started_at);
    let session_ended_at = input
        .segments
        .last()
        .map(|s| instant_to_datetime(input.started_at, input.started_instant, s.ended_at))
        .unwrap_or(input.ended_at);

    let sessions = if input.segments.is_empty() && input.asr_text.is_empty() {
        Vec::new()
    } else {
        vec![AsrSessionHistory {
            text: input.asr_text.clone(),
            started_at: session_started_at,
            ended_at: session_ended_at,
            audio_ms: input.audio_ms,
        }]
    };

    HistoryRecord {
        version: 2,
        id: input.id,
        started_at: input.started_at,
        ended_at: input.ended_at,
        duration_ms: (input.ended_at - input.started_at)
            .whole_milliseconds()
            .max(0) as u64,
        status: input.status,
        app: input.app,
        text: input.final_text.clone(),
        text_stats: crate::text_stats::compute(&input.final_text),
        asr: AsrHistory {
            provider: input.provider,
            text: input.asr_text,
            audio_ms: input.audio_ms,
            sessions,
        },
        pipeline: input.pipeline,
        error: input.error,
    }
}

fn append_history(input: HistoryInput) -> Option<HistoryRecord> {
    let record = build_record(input);
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

fn instant_to_ms(recording_started_instant: Instant, instant: Instant) -> u64 {
    instant
        .saturating_duration_since(recording_started_instant)
        .as_millis() as u64
}

fn instant_elapsed_ms(recording_started_instant: Instant) -> u64 {
    recording_started_instant.elapsed().as_millis() as u64
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

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use time::OffsetDateTime;

    use super::*;

    #[test]
    fn single_session_collapses_multiple_segments_into_one_entry() {
        let recording_start = OffsetDateTime::from_unix_timestamp(1_750_000_000).unwrap();
        let base = Instant::now();
        let segments = vec![
            SegmentCapture {
                text: "alpha ".to_string(),
                started_at: base,
                ended_at: base + std::time::Duration::from_millis(500),
            },
            SegmentCapture {
                text: "beta ".to_string(),
                started_at: base + std::time::Duration::from_millis(600),
                ended_at: base + std::time::Duration::from_millis(1_000),
            },
            SegmentCapture {
                text: "gamma".to_string(),
                started_at: base + std::time::Duration::from_millis(1_100),
                ended_at: base + std::time::Duration::from_millis(1_500),
            },
        ];
        let input = HistoryInput {
            id: "01HXYZABCDEF0123456789ABCD".to_string(),
            provider: "fake".to_string(),
            started_at: recording_start,
            ended_at: recording_start + time::Duration::milliseconds(2_000),
            started_instant: base,
            asr_text: "alpha beta gamma".to_string(),
            final_text: "alpha beta gamma.".to_string(),
            segments,
            pipeline: Vec::new(),
            audio_ms: 1_500,
            app: None,
            status: HistoryStatus::Submitted,
            error: None,
        };
        let record = build_record(input);
        assert_eq!(record.version, 2);
        assert_eq!(record.text, "alpha beta gamma.");
        assert_eq!(record.asr.text, "alpha beta gamma");
        assert_eq!(record.asr.audio_ms, 1_500);
        assert_eq!(
            record.asr.sessions.len(),
            1,
            "all segments collapse into a single session entry in current phase"
        );
        let session = &record.asr.sessions[0];
        assert_eq!(session.text, "alpha beta gamma");
        assert_eq!(session.audio_ms, 1_500);
        assert!(session.started_at < session.ended_at);
        assert!(session.started_at >= record.started_at);
        assert!(session.ended_at <= record.ended_at);
    }

    #[test]
    fn empty_segments_and_empty_asr_text_produce_no_sessions() {
        let recording_start = OffsetDateTime::from_unix_timestamp(1_750_000_000).unwrap();
        let base = Instant::now();
        let input = HistoryInput {
            id: "01HXYZABCDEF0123456789ABCD".to_string(),
            provider: "fake".to_string(),
            started_at: recording_start,
            ended_at: recording_start + time::Duration::milliseconds(500),
            started_instant: base,
            asr_text: String::new(),
            final_text: String::new(),
            segments: Vec::new(),
            pipeline: Vec::new(),
            audio_ms: 0,
            app: None,
            status: HistoryStatus::Canceled,
            error: None,
        };
        let record = build_record(input);
        assert!(record.asr.sessions.is_empty());
        assert_eq!(record.asr.text, "");
    }

    #[test]
    fn all_zero_frame_is_silence() {
        assert!(!frame_has_signal(&[0; 480]));
    }

    #[test]
    fn frame_with_sub_threshold_samples_is_silence() {
        // 全部样本 |s| <= MIN_NONZERO_AMPLITUDE 视为静音
        assert!(!frame_has_signal(&[1, -2, 3, -4, 5, -6, 7, -8, 8, -8]));
    }

    #[test]
    fn frame_with_one_above_threshold_sample_has_signal() {
        let mut samples = vec![0i16; 480];
        samples[100] = MIN_NONZERO_AMPLITUDE + 1;
        assert!(frame_has_signal(&samples));
    }

    #[test]
    fn realistic_noise_floor_passes() {
        // 真实安静房间本底噪声约 -50 dBFS，对应 i16 ≈ 100。务必不能误判为静音。
        let samples: Vec<i16> = (0..480)
            .map(|i| if i % 7 == 0 { 80 } else { -50 })
            .collect();
        assert!(frame_has_signal(&samples));
    }
}
