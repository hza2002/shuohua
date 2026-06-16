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
use crate::voice::observer::{RecordingObserver, SessionPhase, TraceStart};
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

fn observe_asr_event(trace: &mut RecordingObserver, started_at: Instant, event: &AsrEvent) {
    trace.on_asr_event(instant_elapsed_ms(started_at), event);
}

fn observe_asr_error(
    trace: &mut RecordingObserver,
    started_at: Instant,
    err: crate::asr::types::AsrError,
) {
    observe_asr_event(trace, started_at, &AsrEvent::Error { err });
}

fn observe_pcm(trace: &mut RecordingObserver, samples: &[i16]) {
    trace.on_pcm(samples);
}

fn observe_provider_opened(trace: &mut RecordingObserver, started_at: Instant) {
    trace.on_provider_opened(instant_elapsed_ms(started_at));
}

fn observe_session(trace: &mut RecordingObserver, phase: SessionPhase) {
    trace.on_session(phase);
}

fn observe_finish(trace: &mut RecordingObserver, status: &str, audio_samples: u64) {
    trace.on_finish(status, samples_to_ms(audio_samples));
}

fn observe_finish_ms(trace: &mut RecordingObserver, status: &str, audio_ms: u64) {
    trace.on_finish(status, audio_ms);
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
    control_rx: watch::Receiver<SessionControl>,
) {
    let multi_session_enabled =
        params.idle_pause && matches!(params.vad.backend, crate::config::VoiceVadBackend::Silero);
    if multi_session_enabled {
        run_multi_session_recording(provider, params, control_rx).await;
        return;
    }
    run_single_session_recording(provider, params, control_rx).await;
}

async fn run_single_session_recording(
    provider: &dyn AsrProvider,
    params: SessionParams,
    mut control_rx: watch::Receiver<SessionControl>,
) {
    let recording_id = ulid::Ulid::new().to_string();
    let recording_started_at = time::OffsetDateTime::now_utc();
    let recording_started_instant = Instant::now();
    tracing::info!(
        recording_id = %recording_id,
        provider = %provider.name(),
        app = ?params.start_app_context.bundle_id,
        multi_session = false,
        "recording started"
    );
    let mut trace = RecordingObserver::start(TraceStart {
        enabled: params.vad_trace,
        recording_id: recording_id.clone(),
        provider: provider.name().to_string(),
        started_at: recording_started_at.to_string(),
        started_instant: recording_started_instant,
    });
    if params.vad_trace {
        tracing::info!(recording_id = %recording_id, "dev voice trace enabled");
    }
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
                tracing::warn!(
                    recording_id = %recording_id,
                    error = ?e,
                    "record_audio enabled but audio path preparation failed"
                );
                None
            }
        }
    } else {
        None
    };
    if let Some(p) = &audio_path {
        tracing::debug!(recording_id = %recording_id, path = %p.display(), "record audio wav enabled");
    }

    let mut rec = match recorder::start(audio_path) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(recording_id = %recording_id, error = ?e, "recorder start failed");
            observe_finish_ms(&mut trace, "recorder_start_error", 0);
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
            tracing::error!(recording_id = %recording_id, error = %err, "ASR open failed");
            rec.stop();
            observe_asr_error(&mut trace, recording_started_instant, err.clone());
            observe_finish_ms(&mut trace, "asr_open_error", 0);
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
    observe_provider_opened(&mut trace, recording_started_instant);

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
                        observe_pcm(&mut trace, &samples);
                        if !first_audio_seen && frame_has_signal(&samples) {
                            first_audio_seen = true;
                        }
                        if let Err(e) = session.send_pcm(&samples, false).await {
                            tracing::error!(recording_id = %recording_id, error = %e, "ASR send_pcm failed");
                            terminal_error = Some(HistoryError {
                                kind: "asr_send".to_string(),
                                msg: e.to_string(),
                            });
                            break;
                        }
                        audio_samples_sent += samples.len() as u64;
                    }
                    None => {
                        tracing::warn!(recording_id = %recording_id, "recorder ended unexpectedly");
                        stop_requested = true;
                    }
                }
            }
            _ = sleep_until(first_audio_deadline), if !first_audio_seen => {
                tracing::error!(
                    recording_id = %recording_id,
                    timeout_ms = FIRST_AUDIO_TIMEOUT_MS,
                    "no microphone audio received before timeout"
                );
                rec.stop();
                let _ = session.close().await;
                observe_finish(&mut trace, "no_audio", audio_samples_sent);
                params.state.set_error(Some(recording_id));
                send_error_overlay(&params, crate::t!("error.no_audio"));
                return;
            }
            ev = events.recv() => {
                match ev {
                    None => break,
                    Some(AsrEvent::Partial { text, seq }) => {
                        tracing::debug!(
                            recording_id = %recording_id,
                            seq,
                            chars = text.chars().count(),
                            "ASR partial received"
                        );
                        observe_asr_event(&mut trace, recording_started_instant, &AsrEvent::Partial { text: text.clone(), seq });
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
                        tracing::debug!(
                            recording_id = %recording_id,
                            chars = text.chars().count(),
                            "ASR segment received"
                        );
                        observe_asr_event(&mut trace, recording_started_instant, &AsrEvent::Segment { text: text.clone(), started_at, ended_at });
                        params.state.segment(recording_id.clone(), text.clone());
                        overlay_send(&params, OverlayCmd::AppendSegment { text: text.clone() });
                        pending_segments.push(SegmentCapture { text, started_at, ended_at });
                    }
                    Some(AsrEvent::Error { err }) => {
                        tracing::error!(recording_id = %recording_id, error = %err, "ASR event error");
                        observe_asr_error(&mut trace, recording_started_instant, err.clone());
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
                            sessions: wrap_single_session(
                                recording_started_instant,
                                audio_samples_sent,
                                pending_segments,
                            ),
                            pipeline: Vec::new(),
                            app: app_context.bundle_id.clone(),
                            status: HistoryStatus::Error,
                            error: Some(HistoryError {
                                kind: "asr_error".to_string(),
                                msg: err.to_string(),
                            }),
                        }) {
                            params.state.history_appended(record);
                        }
                        observe_finish(&mut trace, "asr_error", audio_samples_sent);
                        return;
                    }
                    Some(AsrEvent::Done) => {
                        observe_asr_event(&mut trace, recording_started_instant, &AsrEvent::Done);
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
                params.overlay.as_ref(),
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
        tracing::info!(recording_id = %recording_id, "recording canceled");
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
            sessions: wrap_single_session(
                recording_started_instant,
                audio_samples_sent,
                pending_segments,
            ),
            pipeline: Vec::new(),
            app: app_context.bundle_id,
            status: HistoryStatus::Canceled,
            error: None,
        }) {
            params.state.history_appended(record);
        }
        observe_finish(&mut trace, "canceled", audio_samples_sent);
        return;
    }
    // terminal_error 路径：录音 / ASR 中途崩溃 → 不跑 post chain、不写剪贴板。
    // 理由（M7 决策）：半成品上屏会误导（用户以为成功），auto_paste 还可能粘到
    // 已经切走的应用；history 保留所有 segments，需要的用户从 TUI 回捞。
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
        sessions: wrap_single_session(
            recording_started_instant,
            audio_samples_sent,
            pending_segments,
        ),
        pipeline,
        app: app_context.bundle_id,
        status: history_status,
        error: terminal_error.or(error),
    }) {
        params.state.history_appended(record);
    }
    observe_finish(&mut trace, trace_status, audio_samples_sent);
}

/// 给定 VAD 检测到的 speech 起点，按 pre-roll / overlap 上限算出该从 timeline
/// 哪个样本开始往新 ASR session 发送。
///
/// 不变量：
///   desired = speech_start - pre_roll
///   bounded = max(desired, last_sent - max_overlap)
///   result  = max(bounded, oldest)   // 不要回到 ring buffer 之外
fn compute_resume_start_sample(
    speech_start_sample: u64,
    pre_roll_samples: u64,
    last_sent_sample: u64,
    max_overlap_samples: u64,
    oldest_sample: u64,
) -> u64 {
    let desired = speech_start_sample.saturating_sub(pre_roll_samples);
    let bounded = desired.max(last_sent_sample.saturating_sub(max_overlap_samples));
    bounded.max(oldest_sample)
}

#[allow(clippy::too_many_arguments)]
async fn run_multi_session_recording(
    provider: &dyn AsrProvider,
    params: SessionParams,
    mut control_rx: watch::Receiver<SessionControl>,
) {
    use crate::voice::silero::{SileroConfig, SileroVad};
    use crate::voice::timeline::{ms_to_samples, PcmTimeline};
    use crate::voice::vad::{VadController, VadFrame, VadPolicy, VadTransition};

    let recording_id = ulid::Ulid::new().to_string();
    let recording_started_at = time::OffsetDateTime::now_utc();
    let recording_started_instant = Instant::now();
    tracing::info!(
        recording_id = %recording_id,
        provider = %provider.name(),
        app = ?params.start_app_context.bundle_id,
        multi_session = true,
        "recording started"
    );
    let mut trace = RecordingObserver::start(TraceStart {
        enabled: params.vad_trace,
        recording_id: recording_id.clone(),
        provider: provider.name().to_string(),
        started_at: recording_started_at.to_string(),
        started_instant: recording_started_instant,
    });
    if params.vad_trace {
        tracing::info!(recording_id = %recording_id, "dev voice trace enabled");
    }
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
                tracing::warn!(
                    recording_id = %recording_id,
                    error = ?e,
                    "record_audio enabled but audio path preparation failed"
                );
                None
            }
        }
    } else {
        None
    };

    let mut rec = match recorder::start(audio_path) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(recording_id = %recording_id, error = ?e, "recorder start failed");
            observe_finish_ms(&mut trace, "recorder_start_error", 0);
            params.state.set_error(Some(recording_id));
            send_error_overlay(&params, crate::t!("error.recorder_start"));
            return;
        }
    };
    let first_audio_deadline = TokioInstant::now() + Duration::from_millis(FIRST_AUDIO_TIMEOUT_MS);
    let mut first_audio_seen = false;

    let mut silero = match SileroVad::new(SileroConfig {
        threshold: params.vad.threshold,
    }) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(recording_id = %recording_id, error = ?e, "Silero VAD init failed");
            rec.stop();
            observe_finish_ms(&mut trace, "vad_init_error", 0);
            params.state.set_error(Some(recording_id));
            send_error_overlay(&params, crate::t!("error.asr_runtime"));
            return;
        }
    };
    let mut controller = VadController::new(VadPolicy {
        min_start_voiced_frames: params.vad.min_start_voiced_frames,
        pause_silence_ms: params.vad.pause_silence_ms,
        frame_ms: SileroConfig::frame_ms(),
    });
    let retention_ms = params.vad.pre_roll_ms + params.vad.max_overlap_ms + 100;
    let mut timeline = PcmTimeline::new(retention_ms);
    let pre_roll_samples = ms_to_samples(params.vad.pre_roll_ms);
    let max_overlap_samples = ms_to_samples(params.vad.max_overlap_ms);

    let ctx = SessionCtx {
        language: LanguageMode::Multilingual {
            hint: vec!["zh-CN".into(), "en-US".into()],
        },
        hotwords: params.hotwords.clone(),
    };

    // 开第一个 provider session。
    let (initial_session, initial_events) = match provider.open(ctx.clone()).await {
        Ok(t) => t,
        Err(err) => {
            tracing::error!(recording_id = %recording_id, error = %err, "ASR open failed");
            rec.stop();
            observe_asr_error(&mut trace, recording_started_instant, err.clone());
            observe_finish_ms(&mut trace, "asr_open_error", 0);
            params.state.set_error(Some(recording_id));
            send_error_overlay(&params, crate::t!("error.asr_open"));
            return;
        }
    };
    // `close()` 消费 Box，所以用 Option 持有以便在状态切换时 take 出来。
    let mut session: Option<Box<dyn AsrSession>> = Some(initial_session);
    let mut events = initial_events;
    overlay_send(
        &params,
        OverlayCmd::SetState {
            state: OverlayState::Recording,
        },
    );
    observe_provider_opened(&mut trace, recording_started_instant);
    let mut session_index: u32 = 0;
    observe_session(
        &mut trace,
        SessionPhase::Start {
            index: session_index,
            start_ms: 0,
        },
    );

    let mut sessions: Vec<SessionCapture> = Vec::new();
    // session 边界严格用 timeline 上的样本索引；started_at / ended_at
    // 都从这两个数字派生，保证 audio_ms == ended_at - started_at。
    let mut current_session_start_sample: u64 = 0;
    let mut current_session_samples: u64 = 0;
    let mut current_pending_segments: Vec<SegmentCapture> = Vec::new();
    let mut last_sent_sample: u64 = 0;
    let mut total_audio_samples: u64 = 0;
    let mut stop_requested = false;
    let mut cancel_requested = false;
    let mut terminal_error: Option<HistoryError> = None;
    let mut active = true;

    'outer: loop {
        if active {
            // ----- Active state -----
            let mut pause_triggered = false;
            'active: loop {
                tokio::select! {
                    biased;
                    _ = control_rx.changed() => {
                        match *control_rx.borrow_and_update() {
                            SessionControl::Stop => { stop_requested = true; break 'active; }
                            SessionControl::Cancel => { cancel_requested = true; break 'outer; }
                            SessionControl::Idle => {}
                        }
                    }
                    pcm = rec.recv() => {
                        match pcm {
                            None => { stop_requested = true; break 'active; }
                            Some(samples) => {
                                observe_pcm(&mut trace, &samples);
                                if !first_audio_seen && frame_has_signal(&samples) {
                                    first_audio_seen = true;
                                }
                                let chunk = timeline.push(&samples);
                                // 发送到当前 session
                                let send_res = match session.as_mut() {
                                    Some(s) => s.send_pcm(&samples, false).await,
                                    None => Err(crate::asr::types::AsrError::Network("no active session".into())),
                                };
                                if let Err(e) = send_res {
                                    tracing::error!(recording_id = %recording_id, error = %e, "ASR send_pcm failed");
                                    terminal_error = Some(HistoryError {
                                        kind: "asr_send".to_string(),
                                        msg: e.to_string(),
                                    });
                                    break 'outer;
                                }
                                current_session_samples += samples.len() as u64;
                                total_audio_samples += samples.len() as u64;
                                last_sent_sample = chunk.end_sample();
                                // 喂 Silero
                                for frame in silero.accept(&samples) {
                                    match controller.accept(frame.frame) {
                                        VadTransition::SilenceStarted => {
                                            pause_triggered = true;
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                                if pause_triggered { break 'active; }
                            }
                        }
                    }
                    _ = sleep_until(first_audio_deadline), if !first_audio_seen => {
                        tracing::error!(
                            recording_id = %recording_id,
                            timeout_ms = FIRST_AUDIO_TIMEOUT_MS,
                            "no microphone audio received before timeout"
                        );
                        rec.stop();
                        if let Some(s) = session.take() { let _ = s.close().await; }
                        observe_finish(&mut trace, "no_audio", total_audio_samples);
                        params.state.set_error(Some(recording_id));
                        send_error_overlay(&params, crate::t!("error.no_audio"));
                        return;
                    }
                    ev = events.recv() => {
                        match ev {
                            None => { stop_requested = true; break 'active; }
                            Some(AsrEvent::Partial { text, seq }) => {
                                tracing::debug!(
                                    recording_id = %recording_id,
                                    seq,
                                    chars = text.chars().count(),
                                    "ASR partial received"
                                );
                                observe_asr_event(
                                    &mut trace,
                                    recording_started_instant,
                                    &AsrEvent::Partial { text: text.clone(), seq },
                                );
                                params.state.partial(recording_id.clone(), text.clone());
                                let live_text = format!(
                                    "{}{}",
                                    current_pending_segments.iter().map(|s| s.text.as_str()).collect::<String>(),
                                    text
                                );
                                let words = crate::text_stats::compute(&live_text).words as u32;
                                let dur_ms = recording_started_instant.elapsed().as_millis() as u64;
                                params.state.stats(dur_ms, words);
                                overlay_send(&params, OverlayCmd::SetStats { dur_ms, words });
                                overlay_send(&params, OverlayCmd::SetText { text, kind: TextKind::Partial });
                            }
                            Some(AsrEvent::Segment { text, started_at, ended_at }) => {
                                tracing::debug!(
                                    recording_id = %recording_id,
                                    chars = text.chars().count(),
                                    "ASR segment received"
                                );
                                observe_asr_event(
                                    &mut trace,
                                    recording_started_instant,
                                    &AsrEvent::Segment { text: text.clone(), started_at, ended_at },
                                );
                                params.state.segment(recording_id.clone(), text.clone());
                                overlay_send(&params, OverlayCmd::AppendSegment { text: text.clone() });
                                current_pending_segments.push(SegmentCapture { text, started_at, ended_at });
                            }
                            Some(AsrEvent::Error { err }) => {
                                tracing::error!(recording_id = %recording_id, error = %err, "ASR event error");
                                observe_asr_event(
                                    &mut trace,
                                    recording_started_instant,
                                    &AsrEvent::Error { err: err.clone() },
                                );
                                terminal_error = Some(HistoryError {
                                    kind: "asr_error".to_string(),
                                    msg: err.to_string(),
                                });
                                break 'outer;
                            }
                            Some(AsrEvent::Done) => {
                                observe_asr_event(
                                    &mut trace,
                                    recording_started_instant,
                                    &AsrEvent::Done,
                                );
                                // provider 主动 Done：把当前 session 落账并进 Idle，等待重启。
                                pause_triggered = true;
                                break 'active;
                            }
                        }
                    }
                }
            }

            // ----- Pausing -----
            // 关键场景：stop_requested 时先 drain stop_delay_ms 再 finalize。
            if stop_requested && !pause_triggered {
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
                let drain_until =
                    Instant::now() + Duration::from_millis(params.stop_delay_ms as u64);
                while Instant::now() < drain_until {
                    let deadline = TokioInstant::from_std(drain_until);
                    tokio::select! {
                        biased;
                        _ = control_rx.changed() => {
                            if matches!(*control_rx.borrow_and_update(), SessionControl::Cancel) {
                                cancel_requested = true;
                                break;
                            }
                        }
                        _ = sleep_until(deadline) => break,
                        pcm = rec.recv() => {
                            match pcm {
                                None => break,
                                Some(samples) => {
                                    observe_pcm(&mut trace, &samples);
                                    let chunk = timeline.push(&samples);
                                    if let Some(s) = session.as_mut() {
                                        let _ = s.send_pcm(&samples, false).await;
                                    }
                                    current_session_samples += samples.len() as u64;
                                    total_audio_samples += samples.len() as u64;
                                    last_sent_sample = chunk.end_sample();
                                }
                            }
                        }
                    }
                }
                if cancel_requested {
                    break 'outer;
                }
                rec.stop();
                while let Some(samples) = rec.try_recv() {
                    observe_pcm(&mut trace, &samples);
                    if let Some(s) = session.as_mut() {
                        let _ = s.send_pcm(&samples, false).await;
                    }
                    let chunk = timeline.push(&samples);
                    current_session_samples += samples.len() as u64;
                    total_audio_samples += samples.len() as u64;
                    last_sent_sample = chunk.end_sample();
                }
            }

            // finalize 当前 session
            observe_session(
                &mut trace,
                SessionPhase::FinalizeStart {
                    index: session_index,
                    t_ms: instant_elapsed_ms(recording_started_instant),
                },
            );
            let Some(active_session_ref) = session.as_mut() else {
                break 'outer;
            };
            match finalize_provider_session(
                active_session_ref,
                &mut events,
                &mut current_pending_segments,
                params.finalize_timeout_ms,
                &mut control_rx,
                &mut terminal_error,
                &mut trace,
                recording_started_instant,
                &params.state,
                &recording_id,
                params.overlay.as_ref(),
            )
            .await
            {
                Ok(FinalizeOutcome::Done) => {}
                Ok(FinalizeOutcome::Canceled) => {
                    cancel_requested = true;
                    break 'outer;
                }
                Err(e) => {
                    tracing::warn!(
                        recording_id = %recording_id,
                        error_kind = %e.kind,
                        error = %e.msg,
                        "ASR finalize error"
                    );
                    terminal_error = Some(e);
                    break 'outer;
                }
            }
            if let Some(s) = session.take() {
                let _ = s.close().await;
            }

            // 记录当前 session 的 SessionCapture。
            // 边界用 sample 索引派生：started_at = 首发样本 timeline 位置，
            // ended_at = 末发样本 timeline 位置；audio_ms = ended − started。
            let session_audio_ms = samples_to_ms(current_session_samples);
            let session_start_ms = samples_to_ms(current_session_start_sample);
            let session_end_sample = current_session_start_sample + current_session_samples;
            let session_end_ms = samples_to_ms(session_end_sample);
            let started_at = recording_started_instant + Duration::from_millis(session_start_ms);
            let ended_at = recording_started_instant + Duration::from_millis(session_end_ms);
            observe_session(
                &mut trace,
                SessionPhase::Done {
                    index: session_index,
                    start_ms: session_start_ms,
                    end_ms: session_end_ms,
                    audio_ms: session_audio_ms,
                },
            );
            sessions.push(SessionCapture {
                started_at,
                ended_at,
                audio_samples: current_session_samples,
                segments: std::mem::take(&mut current_pending_segments),
            });
            current_session_samples = 0;

            if stop_requested {
                break 'outer;
            }
            // VAD-triggered pause -> 进入 Idle 子状态。overlay 切到 Idle
            // 让用户看到"麦克风还在听，ASR 已暂停"。
            overlay_send(
                &params,
                OverlayCmd::SetState {
                    state: OverlayState::Idle,
                },
            );
            active = false;
            controller.reset();
            controller.accept(VadFrame::Silence);
        } else {
            // ----- Idle state -----
            // 找到下一次 speech 起点对应的样本索引
            let mut resume_speech_start: Option<u64> = None;
            'idle: loop {
                tokio::select! {
                    biased;
                    _ = control_rx.changed() => {
                        match *control_rx.borrow_and_update() {
                            SessionControl::Stop => { stop_requested = true; break 'idle; }
                            SessionControl::Cancel => { cancel_requested = true; break 'outer; }
                            SessionControl::Idle => {}
                        }
                    }
                    pcm = rec.recv() => {
                        match pcm {
                            None => { stop_requested = true; break 'idle; }
                            Some(samples) => {
                                observe_pcm(&mut trace, &samples);
                                let chunk = timeline.push(&samples);
                                // 不发 ASR；只跑 VAD
                                for frame in silero.accept(&samples) {
                                    if controller.accept(frame.frame) == VadTransition::SpeechStarted {
                                        resume_speech_start = Some(frame.start_sample);
                                        break;
                                    }
                                }
                                if resume_speech_start.is_some() { break 'idle; }
                                // chunk consumed (push) — 不发到 ASR
                                let _ = chunk;
                            }
                        }
                    }
                }
            }

            if stop_requested {
                // Idle 状态下用户按停止：直接落账，不开新 session。
                break 'outer;
            }
            let Some(speech_start_sample) = resume_speech_start else {
                // 不可能到这里（除非 stop/cancel 已 break 'outer）
                break 'outer;
            };

            // ----- Opening -----
            let send_start = compute_resume_start_sample(
                speech_start_sample,
                pre_roll_samples,
                last_sent_sample,
                max_overlap_samples,
                timeline.oldest_sample(),
            );

            let next_index = session_index + 1;
            let (new_session, new_events) = match provider.open(ctx.clone()).await {
                Ok(t) => t,
                Err(err) => {
                    tracing::error!(recording_id = %recording_id, session_index = next_index, error = %err, "ASR resume open failed");
                    observe_session(
                        &mut trace,
                        SessionPhase::OpenError {
                            index: next_index,
                            t_ms: instant_elapsed_ms(recording_started_instant),
                            message: err.to_string(),
                        },
                    );
                    terminal_error = Some(HistoryError {
                        kind: "asr_resume_open".to_string(),
                        msg: err.to_string(),
                    });
                    break 'outer;
                }
            };
            session = Some(new_session);
            events = new_events;
            session_index = next_index;
            observe_provider_opened(&mut trace, recording_started_instant);
            observe_session(
                &mut trace,
                SessionPhase::Start {
                    index: session_index,
                    start_ms: samples_to_ms(send_start),
                },
            );

            let replay = timeline.slice_from(send_start);
            current_session_start_sample = replay.start_sample;
            current_session_samples = replay.samples.len() as u64;
            total_audio_samples += replay.samples.len() as u64;
            last_sent_sample = replay.end_sample();
            if !replay.samples.is_empty() {
                if let Some(s) = session.as_mut() {
                    if let Err(e) = s.send_pcm(&replay.samples, false).await {
                        tracing::error!(recording_id = %recording_id, session_index, error = %e, "ASR resume send_pcm failed");
                        terminal_error = Some(HistoryError {
                            kind: "asr_send".to_string(),
                            msg: e.to_string(),
                        });
                        break 'outer;
                    }
                }
            }
            controller.reset();
            // 重新设置成 Speech，防止 controller 立刻判 silence。
            controller.accept(VadFrame::Speech);
            // overlay 切回 Recording。复用 SetState 不会重置 view 的录音时钟，
            // 多 session 录音的 dur_ms 是连续的。
            overlay_send(
                &params,
                OverlayCmd::SetState {
                    state: OverlayState::Recording,
                },
            );
            active = true;
        }
    }

    // ----- Teardown -----
    // 异常退出（cancel / send 失败 / open 失败）也走严格 sample-window 边界，
    // ended_at = first_sent + samples_to_ms(audio_samples)。
    if !current_pending_segments.is_empty() || current_session_samples > 0 {
        let session_start_ms = samples_to_ms(current_session_start_sample);
        let session_end_ms = samples_to_ms(current_session_start_sample + current_session_samples);
        sessions.push(SessionCapture {
            started_at: recording_started_instant + Duration::from_millis(session_start_ms),
            ended_at: recording_started_instant + Duration::from_millis(session_end_ms),
            audio_samples: current_session_samples,
            segments: std::mem::take(&mut current_pending_segments),
        });
    }
    if cancel_requested {
        rec.stop();
    }
    if let Some(s) = session.take() {
        let _ = s.close().await;
    }

    let provider_name = provider.name().to_string();
    let raw_text: String = sessions
        .iter()
        .flat_map(|s| s.segments.iter())
        .map(|s| s.text.as_str())
        .collect();

    if cancel_requested {
        tracing::info!(recording_id = %recording_id, "recording canceled");
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
            sessions,
            pipeline: Vec::new(),
            app: app_context.bundle_id,
            status: HistoryStatus::Canceled,
            error: None,
        }) {
            params.state.history_appended(record);
        }
        observe_finish(&mut trace, "canceled", total_audio_samples);
        return;
    }

    let all_segments: Vec<SegmentCapture> = sessions
        .iter()
        .flat_map(|s| s.segments.iter().cloned())
        .collect();
    let (final_text, pipeline, status, error) = if terminal_error.is_some() {
        (raw_text.clone(), Vec::new(), HistoryStatus::Error, None)
    } else {
        dispatch_with_post_chain(&all_segments, params.auto_paste, &app_context, &params)
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
        sessions,
        pipeline,
        app: app_context.bundle_id,
        status: history_status,
        error: terminal_error.or(error),
    }) {
        params.state.history_appended(record);
    }
    observe_finish(&mut trace, trace_status, total_audio_samples);
}

#[allow(clippy::too_many_arguments)]
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
    trace: &mut RecordingObserver,
    recording_started_instant: Instant,
    overlay: Option<&OverlayHandle>,
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
                        observe_pcm(trace, &samples);
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
                        tracing::debug!(
                            recording_id = %recording_id,
                            chars = text.chars().count(),
                            "ASR segment received during drain"
                        );
                        observe_asr_event(
                            trace,
                            recording_started_instant,
                            &AsrEvent::Segment { text: text.clone(), started_at, ended_at },
                        );
                        state.segment(recording_id.to_string(), text.clone());
                        if let Some(overlay) = overlay {
                            overlay.send(OverlayCmd::AppendSegment { text: text.clone() });
                        }
                        pending_segments.push(SegmentCapture { text, started_at, ended_at });
                    }
                    Some(AsrEvent::Done) => {
                        observe_asr_event(trace, recording_started_instant, &AsrEvent::Done);
                        break;
                    }
                    Some(AsrEvent::Partial { text, seq }) => {
                        tracing::debug!(
                            recording_id = %recording_id,
                            seq,
                            chars = text.chars().count(),
                            "ASR partial received during drain"
                        );
                        observe_asr_event(
                            trace,
                            recording_started_instant,
                            &AsrEvent::Partial { text, seq },
                        );
                    }
                    Some(AsrEvent::Error { err }) => {
                        tracing::error!(recording_id = %recording_id, error = %err, "ASR event error during drain");
                        observe_asr_event(
                            trace,
                            recording_started_instant,
                            &AsrEvent::Error { err: err.clone() },
                        );
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
        observe_pcm(trace, &samples);
        let _ = session.send_pcm(&samples, false).await;
        *audio_samples_sent += samples.len() as u64;
    }

    match finalize_provider_session(
        session,
        events,
        pending_segments,
        finalize_timeout_ms,
        control_rx,
        terminal_error,
        trace,
        recording_started_instant,
        state,
        recording_id,
        overlay,
    )
    .await
    {
        Ok(FinalizeOutcome::Done) => false,
        Ok(FinalizeOutcome::Canceled) => {
            cancel_session(rec, session, audio_samples_sent).await;
            true
        }
        Err(e) => {
            match e.kind.as_str() {
                "asr_send_last" => {
                    tracing::error!(
                        recording_id = %recording_id,
                        error = %e.msg,
                        "ASR send is_last failed"
                    )
                }
                "asr_timeout" => {
                    tracing::warn!(
                        recording_id = %recording_id,
                        finalize_timeout_ms,
                        "ASR final timed out"
                    )
                }
                _ => {
                    tracing::warn!(
                        recording_id = %recording_id,
                        error_kind = %e.kind,
                        error = %e.msg,
                        "ASR finalize error"
                    )
                }
            }
            *terminal_error = Some(e);
            false
        }
    }
}

/// 把 voice 视角的 ASR session 收尾原子化：发 `is_last=true`，等 final segment
/// / `Done`、超时或 cancel。期间累计 segment、partial、ASR error（不中断收尾）。
///
/// 返回值语义：
///
/// - `Ok(Done)`：provider 已 finalize（收到 `Done` 或事件通道关闭）。
/// - `Ok(Canceled)`：等待期间收到 `SessionControl::Cancel`；调用方负责后续清理。
/// - `Err(asr_send_last)`：`send_pcm(&[], true)` 失败。
/// - `Err(asr_timeout)`：`finalize_timeout_ms` 内未收到 `Done`。
///
/// 期间出现的 `AsrEvent::Error` 不中断等待（保持 M9 行为），但会写入
/// `terminal_error`，调用方据此决定 history status。
#[allow(clippy::too_many_arguments)]
async fn finalize_provider_session(
    session: &mut Box<dyn AsrSession>,
    events: &mut mpsc::Receiver<AsrEvent>,
    pending_segments: &mut Vec<SegmentCapture>,
    finalize_timeout_ms: u64,
    control_rx: &mut watch::Receiver<SessionControl>,
    terminal_error: &mut Option<HistoryError>,
    trace: &mut RecordingObserver,
    recording_started_instant: Instant,
    state: &crate::state::StateStore,
    recording_id: &str,
    overlay: Option<&OverlayHandle>,
) -> Result<FinalizeOutcome, HistoryError> {
    if let Err(e) = session.send_pcm(&[], true).await {
        return Err(HistoryError {
            kind: "asr_send_last".to_string(),
            msg: e.to_string(),
        });
    }

    let timeout = tokio::time::sleep(Duration::from_millis(finalize_timeout_ms));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            biased;
            _ = control_rx.changed() => {
                if matches!(*control_rx.borrow_and_update(), SessionControl::Cancel) {
                    return Ok(FinalizeOutcome::Canceled);
                }
            }
            _ = &mut timeout => {
                return Err(HistoryError {
                    kind: "asr_timeout".to_string(),
                    msg: "timeout waiting final".to_string(),
                });
            }
            ev = events.recv() => {
                match ev {
                    None => return Ok(FinalizeOutcome::Done),
                    Some(AsrEvent::Done) => {
                        observe_asr_event(trace, recording_started_instant, &AsrEvent::Done);
                        return Ok(FinalizeOutcome::Done);
                    }
                    Some(AsrEvent::Segment { text, started_at, ended_at }) => {
                        tracing::debug!(
                            recording_id = %recording_id,
                            chars = text.chars().count(),
                            "ASR segment received during final"
                        );
                        observe_asr_event(
                            trace,
                            recording_started_instant,
                            &AsrEvent::Segment { text: text.clone(), started_at, ended_at },
                        );
                        state.segment(recording_id.to_string(), text.clone());
                        // finalize 阶段拿到的 definite segment 也要喂 overlay，
                        // 否则 Doubao 在 is_last 之后才"升级"出来的尾段全丢。
                        if let Some(overlay) = overlay {
                            overlay.send(OverlayCmd::AppendSegment { text: text.clone() });
                        }
                        pending_segments.push(SegmentCapture { text, started_at, ended_at });
                    }
                    Some(AsrEvent::Partial { text, seq }) => {
                        tracing::debug!(
                            recording_id = %recording_id,
                            seq,
                            chars = text.chars().count(),
                            "ASR partial received during final"
                        );
                        observe_asr_event(
                            trace,
                            recording_started_instant,
                            &AsrEvent::Partial { text, seq },
                        );
                    }
                    Some(AsrEvent::Error { err }) => {
                        tracing::error!(recording_id = %recording_id, error = %err, "ASR event error during final");
                        observe_asr_event(
                            trace,
                            recording_started_instant,
                            &AsrEvent::Error { err: err.clone() },
                        );
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalizeOutcome {
    Done,
    Canceled,
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
    let dispatched = out.text.clone();
    let pipeline = steps.into_iter().map(PipelineStepHistory::from).collect();
    if let Err(e) = dispatch::dispatch(&out.text, auto_paste) {
        tracing::error!(error = ?e, "dispatch failed");
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

/// 一次 ASR provider session 在 recording timeline 上的捕获。
///
/// M10 之前每条 recording 只产生 1 个 `SessionCapture`；启用 idle_pause +
/// Silero 后，一条 recording 可携带 1..N 个，保持 `sessions[]` 的多 session 语义。
#[derive(Debug, Clone)]
struct SessionCapture {
    /// 该 session 第一帧 PCM 发送时刻（recording timeline 上的 instant）。
    started_at: Instant,
    /// 该 session finalize 完成（或被放弃）时刻。
    ended_at: Instant,
    /// 该 session 已发送的 PCM 样本数。`audio_ms = samples_to_ms(audio_samples)`。
    audio_samples: u64,
    /// 该 session 收到的 ASR segments，按 emit 顺序。
    segments: Vec<SegmentCapture>,
}

/// 当前单 session 路径把整段 PCM 视作一个 `SessionCapture`。
/// 空录音（无 segment 且未发送样本）返回空 Vec，保留 M9 "no sessions" 语义。
///
/// session 边界严格 = "首/末发送样本的 timeline ms"，即
/// `ended_at − started_at == samples_to_ms(audio_samples)`。不变量见 SCHEMA §2.2。
fn wrap_single_session(
    started_at: Instant,
    audio_samples: u64,
    segments: Vec<SegmentCapture>,
) -> Vec<SessionCapture> {
    if segments.is_empty() && audio_samples == 0 {
        return Vec::new();
    }
    let ended_at = started_at + Duration::from_millis(samples_to_ms(audio_samples));
    vec![SessionCapture {
        started_at,
        ended_at,
        audio_samples,
        segments,
    }]
}

fn session_text(segments: &[SegmentCapture]) -> String {
    segments.iter().map(|s| s.text.as_str()).collect()
}

fn build_asr_sessions(
    sessions: &[SessionCapture],
    recording_started_at: time::OffsetDateTime,
    recording_started_instant: Instant,
) -> Vec<AsrSessionHistory> {
    sessions
        .iter()
        .map(|s| AsrSessionHistory {
            text: session_text(&s.segments),
            started_at: instant_to_datetime(
                recording_started_at,
                recording_started_instant,
                s.started_at,
            ),
            ended_at: instant_to_datetime(
                recording_started_at,
                recording_started_instant,
                s.ended_at,
            ),
            audio_ms: samples_to_ms(s.audio_samples),
        })
        .collect()
}

struct HistoryInput {
    id: String,
    provider: String,
    started_at: time::OffsetDateTime,
    ended_at: time::OffsetDateTime,
    started_instant: Instant,
    asr_text: String,
    final_text: String,
    sessions: Vec<SessionCapture>,
    pipeline: Vec<PipelineStepHistory>,
    app: Option<String>,
    status: HistoryStatus,
    error: Option<HistoryError>,
}

fn build_record(input: HistoryInput) -> HistoryRecord {
    let audio_ms: u64 = input
        .sessions
        .iter()
        .map(|s| samples_to_ms(s.audio_samples))
        .sum();
    let all_sessions_empty = input
        .sessions
        .iter()
        .all(|s| s.segments.is_empty() && s.audio_samples == 0);
    let sessions = if all_sessions_empty && input.asr_text.is_empty() {
        Vec::new()
    } else {
        build_asr_sessions(&input.sessions, input.started_at, input.started_instant)
    };

    // ASR 工作窗口（首段 started_at → 末段 ended_at）。空 sessions 直接 0。
    let asr_duration_ms = match (sessions.first(), sessions.last()) {
        (Some(first), Some(last)) => (last.ended_at - first.started_at)
            .whole_milliseconds()
            .max(0) as u64,
        _ => 0,
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
            duration_ms: asr_duration_ms,
            audio_ms,
            sessions,
        },
        pipeline: input.pipeline,
        error: input.error,
    }
}

fn append_history(input: HistoryInput) -> Option<HistoryRecord> {
    let record = build_record(input);
    if let Err(e) = history::append_default(&record) {
        tracing::error!(
            recording_id = %record.id,
            error = ?e,
            "history append failed"
        );
        return None;
    }
    tracing::info!(
        recording_id = %record.id,
        status = ?record.status,
        provider = %record.asr.provider,
        audio_ms = record.asr.audio_ms,
        session_count = record.asr.sessions.len(),
        pipeline_steps = record.pipeline.len(),
        "recording ended"
    );
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
    fn resume_start_uses_pre_roll_when_buffer_has_headroom() {
        // speech at sample 16000 (1s), pre-roll 300ms (4800), no overlap concern.
        let s = compute_resume_start_sample(16_000, 4_800, /*last_sent*/ 0, 3_200, 0);
        assert_eq!(s, 16_000 - 4_800);
    }

    #[test]
    fn resume_start_bounded_by_last_sent_minus_max_overlap() {
        // pre-roll wants to go back further than max_overlap allows.
        // last_sent=20_000, overlap_cap=3_200 -> floor at 16_800.
        // speech_start=18_000, pre_roll=8_000 -> desired=10_000, bounded=max(10_000, 16_800)=16_800.
        let s = compute_resume_start_sample(18_000, 8_000, 20_000, 3_200, 0);
        assert_eq!(s, 16_800);
    }

    #[test]
    fn resume_start_clamped_to_oldest_retained_sample() {
        // 想回到 200，但 ring buffer 最旧只剩 800。
        let s = compute_resume_start_sample(2_000, 4_000, 0, 200, 800);
        assert_eq!(s, 800);
    }

    #[test]
    fn resume_start_overlap_within_max_overlap_ms() {
        // 200ms max overlap @16kHz = 3200 samples.
        let max_overlap = ms_to_samples_helper(200);
        let s = compute_resume_start_sample(10_000, 0, 16_000, max_overlap, 0);
        assert!(
            16_000 - s <= max_overlap,
            "overlap {} > cap {}",
            16_000 - s,
            max_overlap
        );
    }

    fn ms_to_samples_helper(ms: u32) -> u64 {
        (ms as u64) * 16_000 / 1_000
    }

    fn segment(base: Instant, text: &str, start_ms: u64, end_ms: u64) -> SegmentCapture {
        SegmentCapture {
            text: text.to_string(),
            started_at: base + std::time::Duration::from_millis(start_ms),
            ended_at: base + std::time::Duration::from_millis(end_ms),
        }
    }

    #[test]
    fn single_session_collapses_multiple_segments_into_one_entry() {
        let recording_start = OffsetDateTime::from_unix_timestamp(1_750_000_000).unwrap();
        let base = Instant::now();
        let segments = vec![
            segment(base, "alpha ", 0, 500),
            segment(base, "beta ", 600, 1_000),
            segment(base, "gamma", 1_100, 1_500),
        ];
        let sessions = vec![SessionCapture {
            started_at: base,
            ended_at: base + std::time::Duration::from_millis(1_500),
            audio_samples: 16_000 * 1_500 / 1_000,
            segments,
        }];
        let input = HistoryInput {
            id: "01HXYZABCDEF0123456789ABCD".to_string(),
            provider: "fake".to_string(),
            started_at: recording_start,
            ended_at: recording_start + time::Duration::milliseconds(2_000),
            started_instant: base,
            asr_text: "alpha beta gamma".to_string(),
            final_text: "alpha beta gamma.".to_string(),
            sessions,
            pipeline: Vec::new(),
            app: None,
            status: HistoryStatus::Submitted,
            error: None,
        };
        let record = build_record(input);
        assert_eq!(record.version, 2);
        assert_eq!(record.text, "alpha beta gamma.");
        assert_eq!(record.asr.text, "alpha beta gamma");
        assert_eq!(record.asr.audio_ms, 1_500);
        assert_eq!(record.asr.sessions.len(), 1);
        let session = &record.asr.sessions[0];
        assert_eq!(session.text, "alpha beta gamma");
        assert_eq!(session.audio_ms, 1_500);
        assert!(session.started_at <= session.ended_at);
        assert!(session.started_at >= record.started_at);
        assert!(session.ended_at <= record.ended_at);
    }

    #[test]
    fn empty_sessions_and_empty_asr_text_produce_no_sessions() {
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
            sessions: Vec::new(),
            pipeline: Vec::new(),
            app: None,
            status: HistoryStatus::Canceled,
            error: None,
        };
        let record = build_record(input);
        assert!(record.asr.sessions.is_empty());
        assert_eq!(record.asr.text, "");
    }

    #[test]
    fn multi_session_history_sums_audio_ms_and_preserves_session_count() {
        let recording_start = OffsetDateTime::from_unix_timestamp(1_750_000_000).unwrap();
        let base = Instant::now();
        let sessions = vec![
            SessionCapture {
                started_at: base,
                ended_at: base + std::time::Duration::from_millis(800),
                audio_samples: 16_000 * 800 / 1_000,
                segments: vec![segment(base, "hello ", 0, 600)],
            },
            SessionCapture {
                started_at: base + std::time::Duration::from_millis(2_000),
                ended_at: base + std::time::Duration::from_millis(2_900),
                audio_samples: 16_000 * 900 / 1_000,
                segments: vec![segment(base, "world", 2_100, 2_800)],
            },
        ];
        let input = HistoryInput {
            id: "01HXYZABCDEF0123456789ABCD".to_string(),
            provider: "fake".to_string(),
            started_at: recording_start,
            ended_at: recording_start + time::Duration::milliseconds(3_000),
            started_instant: base,
            asr_text: "hello world".to_string(),
            final_text: "hello world.".to_string(),
            sessions,
            pipeline: Vec::new(),
            app: None,
            status: HistoryStatus::Submitted,
            error: None,
        };
        let record = build_record(input);
        assert_eq!(record.asr.sessions.len(), 2);
        assert_eq!(record.asr.sessions[0].text, "hello ");
        assert_eq!(record.asr.sessions[1].text, "world");
        assert_eq!(record.asr.sessions[0].audio_ms, 800);
        assert_eq!(record.asr.sessions[1].audio_ms, 900);
        assert_eq!(record.asr.audio_ms, 800 + 900);

        // ASR multi-session duration = last_session.ended_at - first_session.started_at
        // 同时也写到 asr.duration_ms top-level 字段。
        let asr_duration_ms = (record.asr.sessions.last().unwrap().ended_at
            - record.asr.sessions.first().unwrap().started_at)
            .whole_milliseconds() as u64;
        assert_eq!(asr_duration_ms, 2_900);
        assert_eq!(record.asr.duration_ms, 2_900);
        // M10 跳过的纯静音时长 = duration_ms - audio_ms
        assert_eq!(
            record.asr.duration_ms - record.asr.audio_ms,
            2_900 - (800 + 900)
        );
    }

    #[test]
    fn asr_duration_ms_is_zero_when_no_sessions() {
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
            sessions: Vec::new(),
            pipeline: Vec::new(),
            app: None,
            status: HistoryStatus::Canceled,
            error: None,
        };
        let record = build_record(input);
        assert!(record.asr.sessions.is_empty());
        assert_eq!(record.asr.duration_ms, 0);
        assert_eq!(record.asr.audio_ms, 0);
    }

    #[test]
    fn wrap_single_session_audio_ms_matches_ended_minus_started() {
        // 不变量：sessions[].audio_ms == ended_at - started_at（recording timeline）
        let base = Instant::now();
        let segments = vec![segment(base, "hello", 0, 800)];
        let audio_samples = 16_000 * 1_500 / 1_000; // 1500ms
        let sessions = wrap_single_session(base, audio_samples, segments);
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        let span_ms = s
            .ended_at
            .saturating_duration_since(s.started_at)
            .as_millis() as u64;
        let audio_ms = (s.audio_samples * 1000) / 16_000;
        assert_eq!(span_ms, audio_ms);
        assert_eq!(span_ms, 1_500);
    }

    #[test]
    fn empty_single_session_wrap_returns_empty_vec() {
        let base = Instant::now();
        let sessions = wrap_single_session(base, 0, Vec::new());
        assert!(sessions.is_empty());
    }

    #[test]
    fn overlapping_session_instants_are_preserved() {
        let recording_start = OffsetDateTime::from_unix_timestamp(1_750_000_000).unwrap();
        let base = Instant::now();
        // 第二个 session 从 700ms 开始（在第一个 800ms 结束之前 100ms 重叠）。
        let sessions = vec![
            SessionCapture {
                started_at: base,
                ended_at: base + std::time::Duration::from_millis(800),
                audio_samples: 16_000 * 800 / 1_000,
                segments: vec![segment(base, "a", 0, 700)],
            },
            SessionCapture {
                started_at: base + std::time::Duration::from_millis(700),
                ended_at: base + std::time::Duration::from_millis(1_500),
                audio_samples: 16_000 * 800 / 1_000,
                segments: vec![segment(base, "b", 800, 1_400)],
            },
        ];
        let input = HistoryInput {
            id: "01HXYZABCDEF0123456789ABCD".to_string(),
            provider: "fake".to_string(),
            started_at: recording_start,
            ended_at: recording_start + time::Duration::milliseconds(2_000),
            started_instant: base,
            asr_text: "ab".to_string(),
            final_text: "ab".to_string(),
            sessions,
            pipeline: Vec::new(),
            app: None,
            status: HistoryStatus::Submitted,
            error: None,
        };
        let record = build_record(input);
        assert_eq!(record.asr.sessions.len(), 2);
        // 第二个 session 的 started_at 应早于第一个的 ended_at（允许 overlap）。
        assert!(record.asr.sessions[1].started_at < record.asr.sessions[0].ended_at);
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
