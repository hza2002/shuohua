//! 一次录音的完整生命周期：VAD 驱动多段 ASR session + stop drain + dispatch。
//!
//! M2.5 流程：
//!
//!   1. 生成 recording_id (ULID)
//!   2. 开 cpal streaming recorder + PcmConsumer（500ms pre-roll + VAD）
//!   3. 开首个 ASR session，dump pre-roll 入 session（补辅音/弱起）
//!   4. 主循环：PCM → consumer.feed() → VAD 判定 voiced/unvoiced
//!        - Voiced + 无 session → 开新 session + dump pre-roll
//!        - Unvoiced ≥ pause_asr_silence_ms → 关 session（send is_last →
//!          wait Done → close），段文本追加到 pending_segments
//!        - Unvoiced ≥ auto_stop_silence_ms → auto stop（防忘按 toggle）
//!        - PCM 转发到 session（Active/WindingDown 时）
//!   5. toggle OFF / auto stop → Finishing：
//!        - 若 Active → send is_last，drain stop_delay_ms 尾音
//!        - 等 Done → close session → 追加最后一段文本
//!        - stop recorder
//!   6. 拼最终文本：pending_segments join segment_separator
//!   7. 非空时 dispatch（写剪贴板 + 可选 Cmd+V）
//!
//! M2.5 变更（相对 M2）：
//!   - 不再用 last_partial 兜底（partial 是中间态，最终上屏应只来自 Segment）
//!   - segments 在录音全周期内跨 session 累积（不是只在 finish 收尾）

use std::time::{Duration, Instant};

use crate::asr::types::{AsrEvent, AsrProvider, AsrSession, LanguageMode, SessionCtx};
use crate::voice::consumer::PcmConsumer;
use crate::voice::vad::{VadEvent, VadState};
use crate::voice::{dispatch, recorder};
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep_until;

pub struct SessionParams {
    pub auto_paste: bool,
    pub record_audio: bool,
    pub stop_delay_ms: u32,
    pub hotwords: Vec<String>,
    pub pause_asr_silence_ms: u32,
    pub auto_stop_silence_ms: u32,
    pub segment_separator: String,
}

/// 跑一次完整录音。`stop_rx` 触发 = 用户 toggle OFF（或 auto stop）。函数返回时
/// 整次录音已结束。
pub async fn run_recording(
    provider: &dyn AsrProvider,
    params: SessionParams,
    stop_rx: oneshot::Receiver<()>,
) {
    let recording_id = ulid::Ulid::new().to_string();
    let pause_dur = Duration::from_millis(params.pause_asr_silence_ms as u64);
    let auto_stop_dur = Duration::from_millis(params.auto_stop_silence_ms as u64);
    eprintln!("[shuo] ▶ recording id={recording_id}");

    // 1. wav 路径
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

    // 2. 开录音
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

    // 3. 开首个 ASR session
    let (session0, events0) = match open_session(provider, &ctx, &mut PcmConsumer::new()).await {
        Ok(t) => t,
        Err(err) => {
            eprintln!("[shuo] ❌ ASR open failed: {err}");
            rec.stop();
            return;
        }
    };
    let mut session: Option<Box<dyn AsrSession>> = Some(session0);
    let mut events: Option<mpsc::Receiver<AsrEvent>> = Some(events0);

    // 4. 主循环
    let mut stop_rx = stop_rx;
    let mut consumer = PcmConsumer::new();
    let mut pending_segments: Vec<String> = Vec::new();
    let mut unvoiced_since: Option<Instant> = None;
    let mut winding_down = false;
    let mut stop_requested = false;
    let mut session_active = true; // true if session is Some + not winding_down

    // Start assumed voiced — first few seconds of silence won't close session
    // because user needs time to start speaking
    let first_voiced_reset = Instant::now();

    loop {
        tokio::select! {
            biased;
            // 用户 toggle OFF / auto stop
            _ = &mut stop_rx, if !stop_requested => {
                stop_requested = true;
            }

            // PCM 帧
            pcm = rec.recv(), if !stop_requested => {
                let samples = match pcm {
                    Some(s) => s,
                    None => {
                        eprintln!("[shuo] recorder ended unexpectedly");
                        stop_requested = true;
                        continue;
                    }
                };
                let vad_ev = consumer.feed(&samples);

                // VAD 去抖后的状态切换
                match vad_ev {
                    Some(VadEvent::Switched(VadState::Voiced)) => {
                        eprintln!("[vad] state → Voiced");
                        unvoiced_since = None;
                        if !session_active && !winding_down {
                            // Idle → Active：开新 session + dump pre-roll
                            match open_session(provider, &ctx, &mut consumer).await {
                                Ok((s, e)) => {
                                    session = Some(s);
                                    events = Some(e);
                                    session_active = true;
                                }
                                Err(err) => {
                                    eprintln!("[shuo] ❌ session open failed (idle→active): {err}");
                                }
                            }
                        }
                    }
                    Some(VadEvent::Switched(VadState::Unvoiced)) => {
                        eprintln!("[vad] state → Unvoiced");
                        unvoiced_since = Some(Instant::now());
                    }
                    None => {}
                }

                // Forward PCM 到 session（Active 或 WindingDown）
                if let Some(ref mut s) = session {
                    if let Err(e) = s.send_pcm(&samples, false).await {
                        eprintln!("[shuo] ❌ ASR send_pcm failed: {e}");
                        // 罕见但可能：session 还在但连接断开。回 idle 让下次
                        // VAD voiced 触发重开。
                        let _ = session.take().unwrap().close().await;
                        session = None;
                        events = None;
                        session_active = false;
                        winding_down = false;
                    }
                }

                // Deadline checks
                if let Some(since) = unvoiced_since {
                    // 给一个 guard：录音开始后前 1s 不关 session（用户可能还没来得及开口）。
                    // 这是针对 pause_asr_silence_ms 设得很短（<3s）的边缘保护。
                    if since >= first_voiced_reset && since.elapsed() >= pause_dur
                        && session.is_some() && !winding_down
                    {
                        eprintln!("[session] unvoiced ≥ {}ms, closing session", params.pause_asr_silence_ms);
                        if let Some(ref mut s) = session {
                            // is_last 通知 ASR 这段说完了。等 events 臂收 Segment+Done。
                            let _ = s.send_pcm(&[], true).await;
                        }
                        winding_down = true;
                        session_active = false;
                    }
                    if since.elapsed() >= auto_stop_dur && !stop_requested {
                        eprintln!("[shuo] auto_stop: 静音 ≥ {}ms，自动停", params.auto_stop_silence_ms);
                        stop_requested = true;
                    }
                }
            }

            // ASR 事件
            ev = async {
                match events.as_mut() {
                    Some(e) => e.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match ev {
                    None => {
                        eprintln!("[shuo] ASR events channel closed");
                        events = None;
                        session = None;
                        session_active = false;
                        winding_down = false;
                    }
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
                        if let Some(s) = session.take() { let _ = s.close().await; }
                        events = None;
                        session_active = false;
                        winding_down = false;
                        // Error → return early，不 dispatch
                        return;
                    }
                    Some(AsrEvent::Done) => {
                        eprintln!("[shuo]   done");
                        if let Some(s) = session.take() {
                            let _ = session.take().unwrap().close().await;
                        }
                        events = None;
                        winding_down = false;
                        session_active = false;
                        eprintln!("[session] session closed → idle");
                        // VAD 当前是 Voiced 的话立刻重开（用户在关闭期间恢复了说话）
                        if consumer.vad_state() == VadState::Voiced {
                            match open_session(provider, &ctx, &mut consumer).await {
                                Ok((s, e)) => {
                                    eprintln!("[session] immediate reopen (VAD still voiced)");
                                    session = Some(s);
                                    events = Some(e);
                                    session_active = true;
                                }
                                Err(err) => {
                                    eprintln!("[shuo] ❌ reopen failed: {err}");
                                }
                            }
                        }
                    }
                }
            }
        }

        if stop_requested {
            // 进 Finishing：drain + 关 session + dispatch
            let has_active = session_active || winding_down;
            let to_close = if has_active { session.take() } else { None };
            let mut to_events = if has_active { events.take() } else { None };
            finish_stop(
                &mut rec,
                to_close,
                &mut to_events,
                &mut pending_segments,
                params.stop_delay_ms,
            )
            .await;
            break;
        }
    }

    // 5. 拼最终文本
    let final_text = pending_segments.join(&params.segment_separator);

    // 6. dispatch
    if final_text.is_empty() {
        eprintln!("[shuo] (空识别结果，跳过 dispatch)");
    } else {
        eprintln!("[shuo] ✓ 最终: {final_text}");
        if let Err(e) = dispatch::dispatch(&final_text, params.auto_paste) {
            eprintln!("[shuo] ❌ 剪贴板写入失败: {e:#}");
        }
    }
}

/// 开新 ASR session + 把 pre-roll 历史 dump 进去（避免辅音/弱起被丢，DESIGN §2.9）。
async fn open_session(
    provider: &dyn AsrProvider,
    ctx: &SessionCtx,
    consumer: &mut PcmConsumer,
) -> Result<(Box<dyn AsrSession>, mpsc::Receiver<AsrEvent>), crate::asr::AsrError> {
    let (mut session, events) = provider.open(ctx.clone()).await?;
    let preroll = consumer.drain_preroll();
    if !preroll.is_empty() {
        eprintln!("[session] dump {} samples preroll", preroll.len());
        for chunk in preroll.chunks(8_000) {
            session.send_pcm(chunk, false).await?;
        }
    }
    eprintln!("[session] session opened");
    Ok((session, events))
}

/// Finishing：drain + 关当前 session → 追加末段。session / events 传入时
/// 取 ownership（caller 已 take），函数内 close + drop。
async fn finish_stop(
    rec: &mut recorder::RecordingStream,
    mut session: Option<Box<dyn AsrSession>>,
    events: &mut Option<mpsc::Receiver<AsrEvent>>,
    pending_segments: &mut Vec<String>,
    stop_delay_ms: u32,
) {
    let (Some(mut sess), Some(mut evts)) = (session.take(), events.take()) else {
        rec.stop();
        return;
    };

    let _ = sess.send_pcm(&[], true).await;

    // a. drain stop_delay_ms 内继续读 cpal 帧、推 ASR（防尾字丢）
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
                        let _ = sess.send_pcm(&samples, false).await;
                    }
                    None => break,
                }
            }
            ev = evts.recv() => {
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

    // b. 真停 recorder，吸完剩余帧
    rec.stop();
    while let Some(samples) = rec.try_recv() {
        let _ = sess.send_pcm(&samples, false).await;
    }

    // c. 等 Done 或 5s 超时，然后 close session
    let timeout = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            biased;
            _ = &mut timeout => {
                eprintln!("[shuo] ⚠ 识别 final 超时 5s");
                let _ = sess.close().await;
                return;
            }
            ev = evts.recv() => {
                match ev {
                    None => {
                        let _ = sess.close().await;
                        return;
                    }
                    Some(AsrEvent::Done) => {
                        let _ = sess.close().await;
                        return;
                    }
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
