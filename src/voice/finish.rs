//! 一次录音的完整生命周期：start → stream → stop → finalize → dispatch。
//!
//! M2 不分 Voice 子状态；M2.5 引入 VAD 后这里会拆出 Active/Idle。当前流程：
//!
//!   1. 生成 recording_id (ULID)
//!   2. 开 cpal streaming recorder（可选 wav 留存）
//!   3. 开 ASR session（DoubaoProvider）
//!   4. 主循环 select! 串：
//!        - stop_rx 收到 → 进 Finishing
//!        - recorder 帧 → 转发到 ASR
//!        - ASR 事件 → 累积 partial / segment
//!        - ASR Error → 报错 + 返回
//!   5. Finishing：
//!        - drain stop_delay_ms 内的尾音继续推 ASR（防尾字丢，DESIGN §5 不变量 3）
//!        - stop recorder + 吸完剩余帧推 ASR
//!        - send_pcm(&[], is_last=true) 标末包
//!        - 等 Done 或 5s 超时
//!   6. 拼最终文本：优先 Segment 们拼起来；空则用最后一个 Partial
//!   7. 非空时 dispatch（写剪贴板 + 可选 Cmd+V）
//!
//! 失败语义见 DESIGN §2.8 表（Auth/Network/Quota/...）；M2 一律 stderr 报，
//! 不重试，状态回 Idle。

use std::time::{Duration, Instant};

use crate::asr::types::{AsrEvent, AsrProvider, AsrSession, LanguageMode, SessionCtx};
use crate::voice::{dispatch, recorder};
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep_until;

pub struct SessionParams {
    pub auto_paste: bool,
    pub record_audio: bool,
    pub stop_delay_ms: u32,
    pub hotwords: Vec<String>,
}

/// 跑一次完整录音。`stop_rx` 触发 = 用户 toggle OFF。函数返回时整次录音已结
/// 束（可能成功 dispatch、可能空文本跳过、可能错误中断）。
pub async fn run_recording(
    provider: &dyn AsrProvider,
    params: SessionParams,
    stop_rx: oneshot::Receiver<()>,
) {
    let recording_id = ulid::Ulid::new().to_string();
    eprintln!("[shuo] ▶ recording id={recording_id}");

    // 1. wav 路径（如果 record_audio=true）
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

    // 2. 录音
    let mut rec = match recorder::start(audio_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[shuo] ❌ 录音启动失败: {e:#}");
            return;
        }
    };

    // 3. ASR session
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

    // 4. 主循环
    //
    // stop_rx 必须直接借用进 select 臂（不能用 Option<Receiver> + take 模式）：
    // 任何一次 select 是别的臂赢、wait helper future 被 drop 时，会把 Receiver
    // 一起带走 → 下次 toggle 信号永远收不到。这是 oneshot::Receiver 跟 select!
    // 配合的经典踩坑。
    let mut stop_rx = stop_rx;
    let mut segments: Vec<String> = Vec::new();
    let mut last_partial = String::new();
    let mut stop_requested = false;

    loop {
        tokio::select! {
            biased;
            // 用户 toggle OFF — gating 防止 oneshot 已 ready 后被反复 poll panic
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
                        // 录音流主动结束（罕见：设备拔出等）
                        eprintln!("[shuo] recorder ended unexpectedly");
                        stop_requested = true;
                    }
                }
            }
            ev = events.recv() => {
                match ev {
                    None => break, // ASR 已 Done 并关 channel
                    Some(AsrEvent::Partial { text, seq }) => {
                        eprintln!("[shuo]   partial#{seq}: {text}");
                        last_partial = text;
                    }
                    Some(AsrEvent::Segment { text, .. }) => {
                        eprintln!("[shuo]   segment: {text}");
                        segments.push(text);
                    }
                    Some(AsrEvent::Error { err }) => {
                        eprintln!("[shuo] ❌ ASR error: {err}");
                        // 错误不再尝试 dispatch；快速收尾
                        rec.stop();
                        let _ = session.close().await;
                        return;
                    }
                    Some(AsrEvent::Done) => break,
                }
            }
        }

        if stop_requested {
            // 进 Finishing：drain stop_delay_ms 内的尾音
            finish(
                &mut rec,
                &mut session,
                &mut events,
                &mut segments,
                &mut last_partial,
                params.stop_delay_ms,
            )
            .await;
            break;
        }
    }

    let _ = session.close().await;

    // 5. 拼最终文本
    let final_text = if !segments.is_empty() {
        segments.join("")
    } else {
        last_partial
    };

    // 6. dispatch
    if final_text.is_empty() {
        eprintln!("[shuo] (空识别结果，跳过 dispatch)");
    } else {
        eprintln!("[shuo] ✓ 最终: {final_text}");
        if let Err(e) = dispatch::dispatch(&final_text, params.auto_paste) {
            eprintln!("[shuo] ❌ dispatch failed: {e:#}");
        }
    }
}

/// Finishing 阶段：drain stop_delay 内的尾音 → 真停 recorder → 推末包 → 等 Done。
async fn finish(
    rec: &mut recorder::RecordingStream,
    session: &mut Box<dyn AsrSession>,
    events: &mut mpsc::Receiver<AsrEvent>,
    segments: &mut Vec<String>,
    last_partial: &mut String,
    stop_delay_ms: u32,
) {
    // 5a. drain stop_delay_ms 内继续读 cpal 帧、推 ASR
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
                merge_event(ev, segments, last_partial);
            }
        }
    }

    // 5b. 真停 recorder，吸完剩余帧
    rec.stop();
    while let Some(samples) = rec.try_recv() {
        let _ = session.send_pcm(&samples, false).await;
    }

    // 5c. 末包
    if let Err(e) = session.send_pcm(&[], true).await {
        eprintln!("[shuo] ❌ send is_last failed: {e}");
        return;
    }

    // 5d. 等 Done 或 5s 超时
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
                    other => merge_event(other, segments, last_partial),
                }
            }
        }
    }
}

fn merge_event(
    ev: Option<AsrEvent>,
    segments: &mut Vec<String>,
    last_partial: &mut String,
) {
    match ev {
        Some(AsrEvent::Partial { text, seq }) => {
            eprintln!("[shuo]   partial#{seq}: {text}");
            *last_partial = text;
        }
        Some(AsrEvent::Segment { text, .. }) => {
            eprintln!("[shuo]   segment: {text}");
            segments.push(text);
        }
        Some(AsrEvent::Error { err }) => {
            eprintln!("[shuo] ❌ ASR error during finishing: {err}");
        }
        Some(AsrEvent::Done) | None => {}
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
