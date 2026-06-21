//! 直接驱动 [`engine::run_with_recorder`] 的生命周期测试。
//!
//! 用 `RecordingStream::for_test` 注入受控 PCM，用脚本化 `AsrProvider` /
//! `AsrSession` 控制事件流。覆盖 Continuous / VadPause 双模式的 stop、cancel、
//! 主动 Done、ASR stream close、PCM 发送失败、initial open 失败、no-audio
//! watchdog、multi-session 不变量等真实路径。
//!
//! Resume open 失败本身是 `SessionOpener::open_resume` 的纯函数行为，在
//! `voice::engine::tests::vad_pause_resume_propagates_provider_open_error` 中
//! 用确定性单测覆盖；这里不再尝试用 Silero 触发，避免依赖 ML 模型对合成
//! PCM 的行为而产生假阳性。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::{mpsc, watch};
use tokio::time::{timeout, Duration};

use crate::asr::types::{AsrError, AsrEvent, AsrProvider, AsrSession, Caps, SessionCtx};
use crate::config::{RecordAudioMode, VoiceVadBackend, VoiceVadCfg};
use crate::overlay::OverlayHandle;
use crate::post;
use crate::post::PostChain;
use crate::state::StateStore;
use crate::voice::engine::{self, EngineOutcome, RecordingMode, SessionParams};
use crate::voice::observer::{RecordingObserver, TraceStart};
use crate::voice::recorder::{FinishMode, RecordingStream};
use crate::voice::SessionControl;

// ---------- 测试用 provider / session ----------

type SentLog = Arc<Mutex<Vec<(Vec<i16>, bool)>>>;

enum OpenScript {
    /// 成功 open：提供一个事件 receiver 和一个 session。
    Ok(mpsc::Receiver<AsrEvent>, Box<dyn AsrSession>),
    /// open 失败，返回指定 AsrError。
    Err(AsrError),
}

struct ScriptedProvider {
    next: Mutex<VecDeque<OpenScript>>,
    opens: AtomicUsize,
}

impl ScriptedProvider {
    fn new(scripts: Vec<OpenScript>) -> Self {
        Self {
            next: Mutex::new(scripts.into()),
            opens: AtomicUsize::new(0),
        }
    }

    fn opens(&self) -> usize {
        self.opens.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl AsrProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    fn caps(&self) -> Caps {
        Caps {
            hotwords: false,
            max_session_secs: None,
            multilingual: true,
        }
    }

    async fn open(
        &self,
        _ctx: SessionCtx,
    ) -> Result<(Box<dyn AsrSession>, mpsc::Receiver<AsrEvent>), AsrError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        match self.next.lock().unwrap().pop_front() {
            Some(OpenScript::Ok(rx, session)) => Ok((session, rx)),
            Some(OpenScript::Err(err)) => Err(err),
            None => panic!("ScriptedProvider received unexpected open() call"),
        }
    }
}

/// 总是成功的 session，记录每次 send_pcm。
#[derive(Clone, Default)]
struct RecordingSession {
    sent: SentLog,
}

#[async_trait]
impl AsrSession for RecordingSession {
    async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<(), AsrError> {
        self.sent.lock().unwrap().push((pcm.to_vec(), is_last));
        Ok(())
    }

    async fn close(self: Box<Self>) -> Result<(), AsrError> {
        Ok(())
    }
}

/// 接收第 `done_after_n` 次 send_pcm 后主动向 event 通道发送 Done，
/// 模拟 provider 自发结束当前 session。
struct AutoDoneSession {
    sent: SentLog,
    event_tx: mpsc::Sender<AsrEvent>,
    done_after_n: usize,
}

#[async_trait]
impl AsrSession for AutoDoneSession {
    async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<(), AsrError> {
        let count = {
            let mut sent = self.sent.lock().unwrap();
            sent.push((pcm.to_vec(), is_last));
            sent.len()
        };
        if count == self.done_after_n {
            let _ = self.event_tx.send(AsrEvent::Done).await;
        }
        Ok(())
    }

    async fn close(self: Box<Self>) -> Result<(), AsrError> {
        Ok(())
    }
}

/// 每次 send_pcm 都返回 AsrError::Network，模拟 ASR 发送失败。
struct AlwaysFailSession;

#[async_trait]
impl AsrSession for AlwaysFailSession {
    async fn send_pcm(&mut self, _pcm: &[i16], _is_last: bool) -> Result<(), AsrError> {
        Err(AsrError::Network("scripted send failure".into()))
    }

    async fn close(self: Box<Self>) -> Result<(), AsrError> {
        Ok(())
    }
}

// ---------- 共用工具 ----------

fn signal_frame(len: usize) -> Vec<i16> {
    // amplitude 略高于 first-audio watchdog 阈值，避免被判定为 no_audio
    vec![1_000; len]
}

fn make_recorder() -> (RecordingStream, mpsc::UnboundedSender<Vec<i16>>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (RecordingStream::for_test(rx), tx)
}

fn make_params(
    state: StateStore,
    overlay: Option<OverlayHandle>,
    idle_pause: bool,
    finalize_timeout_ms: u64,
) -> SessionParams {
    SessionParams {
        auto_paste: false,
        record_audio: RecordAudioMode::Off,
        vad_trace: false,
        idle_pause,
        finalize_timeout_ms,
        vad: VoiceVadCfg {
            backend: if idle_pause {
                VoiceVadBackend::Silero
            } else {
                VoiceVadBackend::Off
            },
            threshold: 0.5,
            pause_silence_ms: 1_500,
            pre_roll_ms: 0,
            max_overlap_ms: 0,
            min_start_voiced_frames: 1,
        },
        stop_delay_ms: 0,
        hotwords: vec![],
        start_app_context: post::AppContext::default(),
        post_chain: PostChain {
            name: "test".into(),
            processors: vec![],
        },
        post_timeout_ms: 100,
        overlay,
        state,
    }
}

struct RunHandles {
    recording_id: String,
    recording_started_at: time::OffsetDateTime,
    recording_started_instant: Instant,
    trace: RecordingObserver,
}

fn fresh_handles() -> RunHandles {
    let recording_id = format!("test-{}", ulid::Ulid::new());
    let recording_started_at = time::OffsetDateTime::now_utc();
    let recording_started_instant = Instant::now();
    let trace = RecordingObserver::start(TraceStart {
        enabled: false,
        recording_id: recording_id.clone(),
        provider: "scripted".into(),
        started_at: recording_started_at.to_string(),
        started_instant: recording_started_instant,
    });
    RunHandles {
        recording_id,
        recording_started_at,
        recording_started_instant,
        trace,
    }
}

async fn drive_engine(
    provider: Arc<ScriptedProvider>,
    params: SessionParams,
    control_rx: watch::Receiver<SessionControl>,
    rec: RecordingStream,
    mode: RecordingMode,
) -> Option<EngineOutcome> {
    let handles = fresh_handles();
    // 5 秒上限保护，防止 bug 让测试挂死。
    timeout(
        Duration::from_secs(5),
        run_owned(provider, params, control_rx, rec, mode, handles),
    )
    .await
    .expect("engine.run_with_recorder did not return within 5s")
}

async fn run_owned(
    provider: Arc<ScriptedProvider>,
    params: SessionParams,
    control_rx: watch::Receiver<SessionControl>,
    rec: RecordingStream,
    mode: RecordingMode,
    handles: RunHandles,
) -> Option<EngineOutcome> {
    engine::run_with_recorder(
        provider.as_ref(),
        params,
        control_rx,
        rec,
        handles.recording_id,
        handles.recording_started_at,
        handles.recording_started_instant,
        mode,
        handles.trace,
    )
    .await
}

// ---------- 实际测试 ----------

/// Continuous：PCM → stop → finalize → outcome；返回 1 个 session、无错误。
#[tokio::test]
async fn continuous_normal_completion_yields_single_session() {
    let (event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent: sent.clone() });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state.clone(), Some(overlay), false, 200);
    let (control_tx, control_rx) = watch::channel(SessionControl::Idle);
    let (rec, pcm_tx) = make_recorder();

    pcm_tx.send(signal_frame(480)).unwrap();
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider.clone(),
        params,
        control_rx,
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(50)).await;
    control_tx.send(SessionControl::Stop).unwrap();
    // finalize 期间 provider 必须发 Segment + Done 才能让 engine 顺利收尾
    event_tx
        .send(AsrEvent::Segment {
            text: "hello".into(),
            started_at: Instant::now(),
            ended_at: Instant::now(),
        })
        .await
        .unwrap();
    event_tx.send(AsrEvent::Done).await.unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(
        outcome.terminal_error.is_none(),
        "{:?}",
        outcome.terminal_error
    );
    assert!(!outcome.cancel_requested);
    assert_eq!(outcome.sessions.len(), 1);
    assert_eq!(provider.opens(), 1);
    // 至少有一次 PCM 发送 + 收尾 is_last
    let calls = sent.lock().unwrap();
    assert!(calls.iter().any(|(_, last)| !*last));
    assert!(calls.iter().any(|(_, last)| *last));
}

/// Stop drain 期间 provider 主动 Done：不得再向同一 session 发送 is_last。
#[tokio::test]
async fn provider_done_during_stop_drain_skips_finalize() {
    let (event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent: sent.clone() });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let mut params = make_params(state, Some(overlay), false, 200);
    params.stop_delay_ms = 100;
    let (control_tx, control_rx) = watch::channel(SessionControl::Idle);
    let (rec, pcm_tx) = make_recorder();
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control_rx,
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(40)).await;
    control_tx.send(SessionControl::Stop).unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    event_tx.send(AsrEvent::Done).await.unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(
        outcome.terminal_error.is_none(),
        "{:?}",
        outcome.terminal_error
    );
    let calls = sent.lock().unwrap();
    assert_eq!(
        calls.iter().filter(|(_, is_last)| *is_last).count(),
        0,
        "provider already sent Done during drain; engine must not send is_last: {calls:?}"
    );
}

/// VadPause + provider 主动 Done：当前实现重复 finalize 并触发 asr_timeout。
/// 修复后：跳过 finalize，进入 Idle，stop 后 1 个 session、无错误。
#[tokio::test]
async fn vad_pause_provider_done_does_not_double_finalize() {
    let (event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    // 在第 1 次 send_pcm 后 provider 主动发 Done。
    let session = Box::new(AutoDoneSession {
        sent: sent.clone(),
        event_tx: event_tx.clone(),
        done_after_n: 1,
    });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state.clone(), Some(overlay), true, 100);
    let (control_tx, control_rx) = watch::channel(SessionControl::Idle);
    let (rec, pcm_tx) = make_recorder();

    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider.clone(),
        params,
        control_rx,
        rec,
        RecordingMode::VadPause,
    ));

    // 给 engine 时间处理 PCM → session.send_pcm → AutoDoneSession 触发 Done。
    tokio::time::sleep(Duration::from_millis(80)).await;
    // 此时 engine 应当已经离开 Active，进入 Idle。
    control_tx.send(SessionControl::Stop).unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(
        outcome.terminal_error.is_none(),
        "provider-initiated Done in VadPause should not surface terminal_error, got {:?}",
        outcome.terminal_error
    );
    assert!(!outcome.cancel_requested);
    assert_eq!(outcome.sessions.len(), 1);
    assert_eq!(provider.opens(), 1);
    // 关键：engine 不应在 provider 已 Done 后再发 is_last。
    let calls = sent.lock().unwrap();
    let is_last_calls = calls.iter().filter(|(_, last)| *last).count();
    assert_eq!(
        is_last_calls, 0,
        "provider already sent Done; engine must not send another is_last (calls: {calls:?})"
    );
}

/// Cancel 信号：engine 设 cancel_requested、无 terminal_error、不 dispatch。
#[tokio::test]
async fn cancel_during_active_marks_cancel_requested() {
    let (_event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent: sent.clone() });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), false, 200);
    let (control_tx, control_rx) = watch::channel(SessionControl::Idle);
    let (rec, pcm_tx) = make_recorder();
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider.clone(),
        params,
        control_rx,
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(50)).await;
    control_tx.send(SessionControl::Cancel).unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(outcome.cancel_requested);
    assert!(outcome.terminal_error.is_none());
}

/// 有内容的 cancel（喂过音频，可能误触）必须保留 retained audio（Publish），
/// 供用户从 TUI 找回。
#[tokio::test]
async fn content_bearing_cancel_keeps_retained_audio() {
    let (_event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), false, 200);
    let (control_tx, control_rx) = watch::channel(SessionControl::Idle);
    let (pcm_tx, pcm_rx) = mpsc::unbounded_channel();
    let (rec, finish_rx) = RecordingStream::for_test_observe(pcm_rx);
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control_rx,
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(50)).await;
    control_tx.send(SessionControl::Cancel).unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(outcome.cancel_requested);
    let modes: Vec<_> = std::iter::from_fn(|| finish_rx.try_recv().ok()).collect();
    assert!(
        modes.contains(&FinishMode::Publish),
        "content-bearing cancel must keep retained audio, got {modes:?}"
    );
    assert!(
        !modes.contains(&FinishMode::Discard),
        "content-bearing cancel must not discard retained audio, got {modes:?}"
    );
}

/// 无内容的 cancel（toggle 后立即取消、没喂任何音频）必须丢弃 retained audio，
/// 避免产生 TUI 无法关联的孤儿音频文件。
#[tokio::test]
async fn contentless_cancel_discards_retained_audio() {
    let (_event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), false, 200);
    let (control_tx, control_rx) = watch::channel(SessionControl::Idle);
    let (_pcm_tx, pcm_rx) = mpsc::unbounded_channel();
    let (rec, finish_rx) = RecordingStream::for_test_observe(pcm_rx);
    // 不喂任何 PCM；立即取消（biased select 让 Cancel 抢在 1s watchdog 之前）。

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control_rx,
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(20)).await;
    control_tx.send(SessionControl::Cancel).unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(outcome.cancel_requested);
    assert!(outcome.sessions.is_empty());
    let modes: Vec<_> = std::iter::from_fn(|| finish_rx.try_recv().ok()).collect();
    assert_eq!(
        modes,
        vec![FinishMode::Discard],
        "contentless cancel must discard retained audio, got {modes:?}"
    );
}

/// 正常 stop 完成必须 Publish retained audio（不得 Discard）。
#[tokio::test]
async fn normal_completion_publishes_retained_audio() {
    let (event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), false, 200);
    let (control_tx, control_rx) = watch::channel(SessionControl::Idle);
    let (pcm_tx, pcm_rx) = mpsc::unbounded_channel();
    let (rec, finish_rx) = RecordingStream::for_test_observe(pcm_rx);
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control_rx,
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(50)).await;
    control_tx.send(SessionControl::Stop).unwrap();
    // 模拟真实 recorder：stop 后 cpal stream 关闭，PCM 通道随之 close，
    // 让 drain_after_stop 的阻塞 recv 能够终止。
    drop(pcm_tx);
    event_tx
        .send(AsrEvent::Segment {
            text: "hello".into(),
            started_at: Instant::now(),
            ended_at: Instant::now(),
        })
        .await
        .unwrap();
    event_tx.send(AsrEvent::Done).await.unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(!outcome.cancel_requested);
    let modes: Vec<_> = std::iter::from_fn(|| finish_rx.try_recv().ok()).collect();
    assert!(
        modes.contains(&FinishMode::Publish),
        "normal completion must publish retained audio, got {modes:?}"
    );
    assert!(
        !modes.contains(&FinishMode::Discard),
        "normal completion must not discard retained audio, got {modes:?}"
    );
}

/// 录音中 ASR 事件流被关闭：engine 报 asr_stream_closed terminal error。
#[tokio::test]
async fn asr_stream_close_during_active_yields_terminal_error() {
    let (event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), false, 200);
    let (_control_tx, control_rx) = watch::channel(SessionControl::Idle);
    let (rec, pcm_tx) = make_recorder();
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control_rx,
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(40)).await;
    // 关闭 ASR 事件通道
    drop(event_tx);

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    let err = outcome
        .terminal_error
        .expect("expected terminal_error from closed ASR stream");
    assert_eq!(err.kind, "asr_stream_closed");
    assert!(!outcome.cancel_requested);
}

/// PCM 发送失败：engine 报 asr_send terminal error，不进 dispatch。
#[tokio::test]
async fn pcm_send_failure_yields_terminal_error() {
    let (_event_tx, event_rx) = mpsc::channel(8);
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx,
        Box::new(AlwaysFailSession),
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), false, 200);
    let (_control_tx, control_rx) = watch::channel(SessionControl::Idle);
    let (rec, pcm_tx) = make_recorder();
    pcm_tx.send(signal_frame(480)).unwrap();

    let outcome = drive_engine(provider, params, control_rx, rec, RecordingMode::Continuous)
        .await
        .expect("engine returned None");
    let err = outcome
        .terminal_error
        .expect("expected terminal_error from failed PCM send");
    assert_eq!(err.kind, "asr_send");
}

/// Initial ASR open 失败：engine 返回 None，不产生 EngineOutcome。
#[tokio::test]
async fn initial_asr_open_failure_returns_none() {
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Err(
        AsrError::Auth("denied".into()),
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), false, 200);
    let (_control_tx, control_rx) = watch::channel(SessionControl::Idle);
    let (rec, _pcm_tx) = make_recorder();

    let outcome = drive_engine(
        provider.clone(),
        params,
        control_rx,
        rec,
        RecordingMode::Continuous,
    )
    .await;
    assert!(outcome.is_none(), "initial open failure must return None");
    assert_eq!(provider.opens(), 1);
}

/// First-audio watchdog：1 秒内所有 PCM 样本都低于阈值 → engine 返回 None。
#[tokio::test]
async fn first_audio_watchdog_returns_none_on_silent_input() {
    let (_event_tx, event_rx) = mpsc::channel(8);
    let session = Box::<RecordingSession>::default();
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), false, 200);
    let (_control_tx, control_rx) = watch::channel(SessionControl::Idle);
    let (rec, pcm_tx) = make_recorder();

    // 全零帧低于 MIN_NONZERO_AMPLITUDE，无法触发 first_audio_seen。
    pcm_tx.send(vec![0i16; 480]).unwrap();
    pcm_tx.send(vec![0i16; 480]).unwrap();

    let outcome = drive_engine(
        provider.clone(),
        params,
        control_rx,
        rec,
        RecordingMode::Continuous,
    )
    .await;
    assert!(
        outcome.is_none(),
        "first-audio watchdog must return None on silent input"
    );
    assert_eq!(provider.opens(), 1);
}

/// Continuous outcome：累计 audio_samples 等于唯一 session.audio_samples。
#[tokio::test]
async fn continuous_outcome_preserves_audio_ms_invariant() {
    let (event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent: sent.clone() });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), false, 200);
    let (control_tx, control_rx) = watch::channel(SessionControl::Idle);
    let (rec, pcm_tx) = make_recorder();
    pcm_tx.send(signal_frame(480)).unwrap();
    pcm_tx.send(signal_frame(960)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control_rx,
        rec,
        RecordingMode::Continuous,
    ));
    tokio::time::sleep(Duration::from_millis(60)).await;
    control_tx.send(SessionControl::Stop).unwrap();
    event_tx
        .send(AsrEvent::Segment {
            text: "ok".into(),
            started_at: Instant::now(),
            ended_at: Instant::now(),
        })
        .await
        .unwrap();
    event_tx.send(AsrEvent::Done).await.unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    let sum_samples: u64 = outcome.sessions.iter().map(|s| s.audio_samples).sum();
    assert_eq!(sum_samples, outcome.total_audio_samples);
    assert!(outcome.total_audio_samples >= (480 + 960) as u64);
}
