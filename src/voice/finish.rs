//! 一次录音的完整生命周期：录音 → ASR → pipeline → dispatch。
//!
//! 两条路径，由 config `voice.vad_enabled` 控制：
//!
//! **单 session（默认，vad_enabled=false）**：
//!   跟 M2 行为一样：cpal stream → 单一 Doubao session → toggle OFF →
//!   drain stop_delay → finalize → filler pipeline → dispatch。简单可靠。
//!
//! **多 session（vad_enabled=true，实验性）**：
//!   客户端 VAD + 多段 ASR session（"思考不计费"）。DESIGN §2.9 完整实现。
//!   VAD 代码（vad.rs / consumer.rs）保留，切换开关即可启用。
//!
//! M2.5 真正交付的改进（两条路径共用）：
//!   - RuleBasedFiller 去口语词（嗯/啊/呃/那个/就是）
//!   - segment_separator 拼接多段 ASR segment
//!   - 不再用 last_partial 兜底（只信任 Segment）

use std::time::{Duration, Instant};

use crate::asr::types::{AsrEvent, AsrProvider, AsrSession, LanguageMode, SessionCtx};
use crate::post::{self, PipelineText, RuleBasedFiller};
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
    pub vad_enabled: bool,
}

pub async fn run_recording(
    provider: &dyn AsrProvider,
    params: SessionParams,
    stop_rx: oneshot::Receiver<()>,
) {
    if params.vad_enabled {
        run_multi_session(provider, params, stop_rx).await;
    } else {
        run_single_session(provider, params, stop_rx).await;
    }
}

// ── single session ────────────────────────────────────────────────────────

async fn run_single_session(
    provider: &dyn AsrProvider,
    params: SessionParams,
    stop_rx: oneshot::Receiver<()>,
) {
    let recording_id = ulid::Ulid::new().to_string();
    eprintln!("[shuo] ▶ recording id={recording_id} (single session)");

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
            single_finish(
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

async fn single_finish(
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

// ── multi session (VAD, 实验性) ───────────────────────────────────────────

async fn run_multi_session(
    provider: &dyn AsrProvider,
    params: SessionParams,
    stop_rx: oneshot::Receiver<()>,
) {
    let recording_id = ulid::Ulid::new().to_string();
    let pause_dur = Duration::from_millis(params.pause_asr_silence_ms as u64);
    let auto_stop_dur = Duration::from_millis(params.auto_stop_silence_ms as u64);
    eprintln!("[shuo] ▶ recording id={recording_id} (VAD multi-session)");

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

    let mut stop_rx = stop_rx;
    let mut consumer = PcmConsumer::new();
    let mut pending_segments: Vec<String> = Vec::new();
    let mut unvoiced_since: Option<Instant> = None;
    let mut voiced_since: Option<Instant> = None;
    let mut winding_down = false;
    let mut stop_requested = false;
    let mut session_active = true;
    let mut sess_segment_count: usize = 0;
    let mut voiced_confirm_ms: u64 = 500;
    const NOISE_VOICED_CONFIRM_MS: u64 = 2000;

    loop {
        tokio::select! {
            biased;
            _ = &mut stop_rx, if !stop_requested => {
                stop_requested = true;
            }

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

                match vad_ev {
                    Some(VadEvent::Switched(VadState::Voiced)) => {
                        eprintln!("[vad] state → Voiced");
                        voiced_since = Some(Instant::now());
                    }
                    Some(VadEvent::Switched(VadState::Unvoiced)) => {
                        eprintln!("[vad] state → Unvoiced");
                        voiced_since = None;
                        if unvoiced_since.is_none() {
                            unvoiced_since = Some(Instant::now());
                        }
                    }
                    None => {}
                }

                if let Some(vs) = voiced_since {
                    if vs.elapsed() >= Duration::from_millis(voiced_confirm_ms) {
                        voiced_since = None;
                        if !session_active && !winding_down {
                            match open_session(provider, &ctx, &mut consumer).await {
                                Ok((s, e)) => {
                                    eprintln!("[vad] voiced confirmed (≥{voiced_confirm_ms}ms), session opened");
                                    session = Some(s);
                                    events = Some(e);
                                    session_active = true;
                                    sess_segment_count = 0;
                                    unvoiced_since = None;
                                }
                                Err(err) => {
                                    eprintln!("[shuo] ❌ session open failed (idle→active): {err}");
                                }
                            }
                        }
                    }
                }

                if let Some(ref mut s) = session {
                    if let Err(e) = s.send_pcm(&samples, false).await {
                        eprintln!("[shuo] ❌ ASR send_pcm failed: {e}");
                        let _ = session.take().unwrap().close().await;
                        session = None;
                        events = None;
                        session_active = false;
                        winding_down = false;
                    }
                }

                if let Some(since) = unvoiced_since {
                    if since.elapsed() >= pause_dur && session.is_some() && !winding_down {
                        eprintln!("[session] unvoiced ≥ {}ms, closing session", params.pause_asr_silence_ms);
                        if let Some(ref mut s) = session {
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
                        unvoiced_since = None;
                    }
                    Some(AsrEvent::Segment { text, .. }) => {
                        eprintln!("[shuo]   segment: {text}");
                        pending_segments.push(text);
                        sess_segment_count += 1;
                        unvoiced_since = None;
                    }
                    Some(AsrEvent::Error { err }) => {
                        eprintln!("[shuo] ❌ ASR error: {err}");
                        rec.stop();
                        if let Some(s) = session.take() { let _ = s.close().await; }
                        events = None;
                        session_active = false;
                        winding_down = false;
                        return;
                    }
                    Some(AsrEvent::Done) => {
                        eprintln!("[shuo]   done");
                        let produced = sess_segment_count;
                        if let Some(s) = session.take() {
                            let _ = s.close().await;
                        }
                        events = None;
                        winding_down = false;
                        session_active = false;
                        unvoiced_since = None;
                        voiced_confirm_ms = if produced == 0 {
                            eprintln!("[session] noise session, next confirm → 2s");
                            NOISE_VOICED_CONFIRM_MS
                        } else {
                            500
                        };
                        eprintln!("[session] session closed → idle (segments={produced})");
                    }
                }
            }
        }

        if stop_requested {
            let has_active = session_active || winding_down;
            let to_close = if has_active { session.take() } else { None };
            let mut to_events = if has_active { events.take() } else { None };
            multi_finish(&mut rec, to_close, &mut to_events, &mut pending_segments, params.stop_delay_ms).await;
            break;
        }
    }

    dispatch_with_filler(pending_segments, &params.segment_separator, params.auto_paste).await;
}

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

async fn multi_finish(
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
                    Some(samples) => { let _ = sess.send_pcm(&samples, false).await; }
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

    rec.stop();
    while let Some(samples) = rec.try_recv() {
        let _ = sess.send_pcm(&samples, false).await;
    }

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
                    None => { let _ = sess.close().await; return; }
                    Some(AsrEvent::Done) => { let _ = sess.close().await; return; }
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

// ── shared ────────────────────────────────────────────────────────────────

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
