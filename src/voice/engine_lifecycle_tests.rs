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
use tokio::sync::{mpsc, oneshot};
use tokio::time::{timeout, Duration};

use crate::asr::types::{AsrError, AsrEvent, AsrProvider, AsrSession, Caps, SessionCtx};
use crate::config::{RecordAudioMode, VoiceVadBackend, VoiceVadCfg};
use crate::overlay::{OverlayCmd, OverlayHandle};
use crate::post;
use crate::post::PostChain;
use crate::state::StateStore;
use crate::voice::engine::{self, EngineOutcome, RecordingMode, SessionParams};
use crate::voice::observer::{RecordingObserver, TraceStart};
use crate::voice::recorder::{FinishMode, RecordingStream};
use crate::voice::vad::VadFrame;
use crate::voice::SessionControl;

// ---------- 测试用 provider / session ----------

type SentLog = Arc<Mutex<Vec<(Vec<i16>, bool)>>>;

enum OpenScript {
    /// 成功 open：提供一个事件 receiver 和一个 session。
    Ok(mpsc::Receiver<AsrEvent>, Box<dyn AsrSession>),
    /// 延迟成功 open，用于覆盖 resume open 等待期间的 capture drain。
    DelayOk {
        opened: mpsc::Receiver<AsrEvent>,
        session: Box<dyn AsrSession>,
        open_started: oneshot::Sender<()>,
        release: oneshot::Receiver<()>,
    },
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
        let script = self.next.lock().unwrap().pop_front();
        match script {
            Some(OpenScript::Ok(rx, session)) => Ok((session, rx)),
            Some(OpenScript::DelayOk {
                opened,
                session,
                open_started,
                release,
            }) => {
                let _ = open_started.send(());
                let _ = release.await;
                Ok((session, opened))
            }
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

struct CloseCountingSession {
    sent: SentLog,
    closes: Arc<AtomicUsize>,
}

#[async_trait]
impl AsrSession for CloseCountingSession {
    async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<(), AsrError> {
        self.sent.lock().unwrap().push((pcm.to_vec(), is_last));
        Ok(())
    }

    async fn close(self: Box<Self>) -> Result<(), AsrError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct DelayedCloseSession {
    sent: SentLog,
    close_started: oneshot::Sender<()>,
    close_release: oneshot::Receiver<()>,
}

#[async_trait]
impl AsrSession for DelayedCloseSession {
    async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<(), AsrError> {
        self.sent.lock().unwrap().push((pcm.to_vec(), is_last));
        Ok(())
    }

    async fn close(self: Box<Self>) -> Result<(), AsrError> {
        let _ = self.close_started.send(());
        let _ = self.close_release.await;
        Ok(())
    }
}

struct DelayedSendSession {
    sent: SentLog,
    delay_on_send: usize,
    send_started: Option<oneshot::Sender<()>>,
    send_release: Option<oneshot::Receiver<()>>,
}

#[async_trait]
impl AsrSession for DelayedSendSession {
    async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<(), AsrError> {
        let count = {
            let mut sent = self.sent.lock().unwrap();
            sent.push((pcm.to_vec(), is_last));
            sent.len()
        };
        if count == self.delay_on_send {
            if let Some(started) = self.send_started.take() {
                let _ = started.send(());
            }
            if let Some(release) = self.send_release.take() {
                let _ = release.await;
            }
        }
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

fn tone_frame(len: usize, value: i16) -> Vec<i16> {
    vec![value; len]
}

fn silence_frame(len: usize) -> Vec<i16> {
    vec![0; len]
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
        preprocess: crate::config::VoicePreprocessCfg::default(),
        vad_trace: false,
        apple_backend_trace: false,
        idle_pause,
        open_timeout_ms: 100,
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
        profile_name: "test".into(),
        profile_choices: vec![crate::overlay::ProfileChoice::test("test")],
        post_chain: PostChain {
            name: "test".into(),
            processors: vec![],
        },
        post_timeout_ms: 100,
        start: crate::voice::resume::RecordingStart::Fresh,
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
    let recording_id = format!("test-{}", ulid::Ulid::generate());
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
    control: SessionControl,
    rec: RecordingStream,
    mode: RecordingMode,
) -> Option<EngineOutcome> {
    let handles = fresh_handles();
    // 5 秒上限保护，防止 bug 让测试挂死。
    timeout(
        Duration::from_secs(5),
        run_owned(provider, params, control, rec, mode, handles),
    )
    .await
    .expect("engine.run_with_recorder did not return within 5s")
}

async fn drive_engine_with_vad_frames(
    provider: Arc<ScriptedProvider>,
    params: SessionParams,
    control: SessionControl,
    rec: RecordingStream,
    mode: RecordingMode,
    frames: VecDeque<VadFrame>,
) -> Option<EngineOutcome> {
    let handles = fresh_handles();
    timeout(
        Duration::from_secs(5),
        run_owned_with_vad_frames(provider, params, control, rec, mode, handles, frames),
    )
    .await
    .expect("engine.run_with_recorder did not return within 5s")
}

async fn run_owned(
    provider: Arc<ScriptedProvider>,
    params: SessionParams,
    control: SessionControl,
    rec: RecordingStream,
    mode: RecordingMode,
    handles: RunHandles,
) -> Option<EngineOutcome> {
    engine::run_with_recorder(
        provider.as_ref(),
        params,
        control,
        rec,
        handles.recording_id,
        handles.recording_started_at,
        handles.recording_started_instant,
        mode,
        handles.trace,
    )
    .await
}

async fn run_owned_with_vad_frames(
    provider: Arc<ScriptedProvider>,
    params: SessionParams,
    control: SessionControl,
    rec: RecordingStream,
    mode: RecordingMode,
    handles: RunHandles,
    frames: VecDeque<VadFrame>,
) -> Option<EngineOutcome> {
    engine::run_with_recorder_and_vad_frames(
        provider.as_ref(),
        params,
        control,
        rec,
        handles.recording_id,
        handles.recording_started_at,
        handles.recording_started_instant,
        mode,
        handles.trace,
        frames,
    )
    .await
}

async fn wait_until_last_sent(sent: &SentLog) {
    timeout(Duration::from_secs(1), async {
        loop {
            if sent.lock().unwrap().iter().any(|(_, is_last)| *is_last) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("session did not enter finalize");
}

async fn wait_until_sent_chunks(sent: &SentLog, chunks: usize) {
    timeout(Duration::from_secs(1), async {
        loop {
            if sent.lock().unwrap().len() >= chunks {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("session did not receive {chunks} PCM chunks"));
}

fn drain_state_events(rx: &mut tokio::sync::broadcast::Receiver<crate::state::StateEvent>) {
    while rx.try_recv().is_ok() {}
}

async fn expect_audio_meter(
    rx: &mut tokio::sync::broadcast::Receiver<crate::state::StateEvent>,
    reason: &str,
) -> crate::state::AudioMeter {
    timeout(Duration::from_millis(150), async {
        loop {
            if let crate::state::StateEvent::AudioMeter { meter, .. } =
                rx.recv().await.expect("state event channel closed")
            {
                return meter;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("expected AudioMeter while {reason}"))
}

fn drain_audio_meter_count(
    rx: &mut tokio::sync::broadcast::Receiver<crate::state::StateEvent>,
) -> usize {
    std::iter::from_fn(|| rx.try_recv().ok())
        .filter(|event| matches!(event, crate::state::StateEvent::AudioMeter { .. }))
        .count()
}

fn drain_state_events_seen_active(
    rx: &mut tokio::sync::broadcast::Receiver<crate::state::StateEvent>,
) -> bool {
    std::iter::from_fn(|| rx.try_recv().ok()).any(|event| {
        matches!(
            event,
            crate::state::StateEvent::SessionPhase {
                phase: crate::state::SessionPhase::Active,
                ..
            }
        )
    })
}

// ---------- 实际测试 ----------

#[tokio::test]
async fn active_partial_stats_use_recording_total_words() {
    let (event_tx, event_rx) = mpsc::channel(8);
    let sent = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx,
        Box::new(RecordingSession { sent: sent.clone() }),
    )]));

    let state = StateStore::new();
    let (overlay, mut overlay_rx) = OverlayHandle::channel();
    let params = make_params(state.clone(), Some(overlay), false, 200);
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::Continuous,
    ));

    pcm_tx.send(tone_frame(480, 1_000)).unwrap();
    wait_until_sent_chunks(&sent, 1).await;
    let segment_start = Instant::now();
    event_tx
        .send(AsrEvent::Segment {
            text: "one two ".to_string(),
            started_at: segment_start,
            ended_at: segment_start,
        })
        .await
        .unwrap();
    event_tx
        .send(AsrEvent::Partial {
            text: "three four".to_string(),
            seq: 1,
        })
        .await
        .unwrap();

    timeout(Duration::from_secs(1), async {
        loop {
            let snapshot = state.snapshot();
            if snapshot.partial == "three four" {
                assert_eq!(snapshot.segments, vec!["one two "]);
                assert_eq!(snapshot.words, 4);
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("partial stats did not reach state");

    timeout(Duration::from_secs(1), async {
        loop {
            match overlay_rx.try_recv() {
                Ok(OverlayCmd::SetStats { words, .. }) => {
                    assert_eq!(words, 4);
                    return;
                }
                Ok(_) => {}
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    panic!("overlay command channel closed");
                }
            }
        }
    })
    .await
    .expect("partial stats did not reach overlay");

    control.request_stop();
    drop(pcm_tx);
    event_tx.send(AsrEvent::Done).await.unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(
        outcome.terminal_error.is_none(),
        "{:?}",
        outcome.terminal_error
    );
}

/// VadPause 初始静音应保持 Idle，不应提前打开 provider session。
#[tokio::test]
async fn vad_pause_initial_silence_stays_idle_and_never_opens_provider() {
    let provider = Arc::new(ScriptedProvider::new(vec![]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), true, 200);
    let control = SessionControl::new();
    let (pcm_tx, pcm_rx) = mpsc::unbounded_channel();
    let (rec, finish_rx) = RecordingStream::for_test_observe(pcm_rx);

    pcm_tx.send(silence_frame(480)).unwrap();
    pcm_tx.send(silence_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider.clone(),
        params,
        control.clone(),
        rec,
        RecordingMode::VadPause,
    ));

    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(
        provider.opens(),
        0,
        "initial silence in VadPause must not open an ASR provider session"
    );

    control.request_stop();
    drop(pcm_tx);

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(
        outcome.terminal_error.is_none(),
        "{:?}",
        outcome.terminal_error
    );
    assert!(!outcome.cancel_requested);
    assert!(
        outcome.sessions.is_empty(),
        "initial silence is not an ASR session"
    );
    assert_eq!(
        outcome.total_audio_samples, 0,
        "idle-listening PCM is not provider audio"
    );
    let modes: Vec<_> = std::iter::from_fn(|| finish_rx.try_recv().ok()).collect();
    assert_eq!(
        modes,
        vec![FinishMode::Discard],
        "contentless initial-silence stop must discard retained audio, got {modes:?}"
    );
    assert_eq!(provider.opens(), 0);
}

/// VadPause 初始静音期间取消：无内容丢弃 retained audio，且不打开 provider。
#[tokio::test]
async fn vad_pause_initial_silence_cancel_discards_without_opening_provider() {
    let provider = Arc::new(ScriptedProvider::new(vec![]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), true, 200);
    let control = SessionControl::new();
    let (pcm_tx, pcm_rx) = mpsc::unbounded_channel();
    let (rec, finish_rx) = RecordingStream::for_test_observe(pcm_rx);

    pcm_tx.send(silence_frame(480)).unwrap();
    pcm_tx.send(silence_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider.clone(),
        params,
        control.clone(),
        rec,
        RecordingMode::VadPause,
    ));

    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(
        provider.opens(),
        0,
        "initial silence in VadPause must not open an ASR provider session"
    );

    control.request_cancel();
    drop(pcm_tx);

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(outcome.cancel_requested);
    assert!(outcome.terminal_error.is_none());
    assert!(outcome.sessions.is_empty());
    assert_eq!(
        outcome.total_audio_samples, 0,
        "idle-listening PCM is not provider audio"
    );
    let modes: Vec<_> = std::iter::from_fn(|| finish_rx.try_recv().ok()).collect();
    assert_eq!(
        modes,
        vec![FinishMode::Discard],
        "contentless initial-silence cancel must discard retained audio, got {modes:?}"
    );
    assert_eq!(provider.opens(), 0);
}

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
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();

    pcm_tx.send(signal_frame(480)).unwrap();
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider.clone(),
        params,
        control.clone(),
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(50)).await;
    control.request_stop();
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

#[tokio::test]
async fn normal_completion_does_not_promote_partial_without_final_text() {
    let (event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), false, 200);
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();

    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::Continuous,
    ));

    event_tx
        .send(AsrEvent::Partial {
            text: "tentative".into(),
            seq: 1,
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    control.request_stop();
    event_tx.send(AsrEvent::Done).await.unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(!outcome.cancel_requested);
    assert_eq!(outcome.sessions.len(), 1);
    assert_eq!(
        crate::voice::capture::session_text(&outcome.sessions[0]),
        ""
    );
}

/// Resume：seed 文本在录音一开始就作为已提交 segment 铺到 overlay，并附带 resume
/// notice；但 seed 只用于展示，不进 capture sessions（finish 另行拼接）。
#[tokio::test]
async fn resume_seed_seeds_overlay_but_not_capture() {
    let (event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, mut overlay_rx) = OverlayHandle::channel();
    let mut params = make_params(state, Some(overlay), false, 200);
    params.start = crate::voice::resume::RecordingStart::Seed(crate::voice::resume::ResumeSeed {
        text: "old text ".into(),
    });
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(50)).await;
    control.request_stop();
    event_tx
        .send(AsrEvent::Segment {
            text: "new text".into(),
            started_at: Instant::now(),
            ended_at: Instant::now(),
        })
        .await
        .unwrap();
    event_tx.send(AsrEvent::Done).await.unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");

    // seed 不进 capture：唯一 session 只含新说的话。
    assert_eq!(outcome.sessions.len(), 1);
    assert_eq!(
        crate::voice::capture::session_text(&outcome.sessions[0]),
        "new text"
    );

    let cmds: Vec<_> = std::iter::from_fn(|| overlay_rx.try_recv().ok()).collect();
    let first_segment = cmds
        .iter()
        .find_map(|c| match c {
            OverlayCmd::AppendSegment { text } => Some(text.clone()),
            _ => None,
        })
        .expect("overlay must receive a seed segment before real ASR");
    assert_eq!(
        first_segment, "old text ",
        "first overlay segment must be the resume seed"
    );
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            OverlayCmd::Notice { text, .. } if text == &crate::t!("notice.resume_recording")
        )),
        "resume must surface a notice: {cmds:?}"
    );
}

/// Resume 热键但没有可恢复记录（`NewFromResume`）：照常开新录音，仍然给用户一条
/// 「新录音」notice；不铺任何 seed segment。
#[tokio::test]
async fn resume_new_from_resume_shows_notice_without_seed_segment() {
    let (_event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, mut overlay_rx) = OverlayHandle::channel();
    let mut params = make_params(state, Some(overlay), false, 200);
    params.start = crate::voice::resume::RecordingStart::NewFromResume;
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(50)).await;
    control.request_cancel();
    let _ = engine_task.await.unwrap();

    let cmds: Vec<_> = std::iter::from_fn(|| overlay_rx.try_recv().ok()).collect();
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            OverlayCmd::Notice { text, .. } if text == &crate::t!("notice.new_recording")
        )),
        "resume-new must surface a 'new recording' notice: {cmds:?}"
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, OverlayCmd::AppendSegment { .. })),
        "resume-new must not seed any segment: {cmds:?}"
    );
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
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(40)).await;
    control.request_stop();
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
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();

    pcm_tx.send(signal_frame(512)).unwrap();

    let engine_task = tokio::spawn(drive_engine_with_vad_frames(
        provider.clone(),
        params,
        control.clone(),
        rec,
        RecordingMode::VadPause,
        VecDeque::from([VadFrame::Speech]),
    ));

    // 给 engine 时间处理 PCM → session.send_pcm → AutoDoneSession 触发 Done。
    tokio::time::sleep(Duration::from_millis(80)).await;
    // 此时 engine 应当已经离开 Active，进入 Idle。
    control.request_stop();

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

#[tokio::test]
async fn vad_pause_emits_audio_meter_while_finalize_is_waiting() {
    let (event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent: sent.clone() });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (_, mut state_rx) = state.subscribe_with_snapshot();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let mut params = make_params(state, Some(overlay), true, 500);
    params.vad.pause_silence_ms = 1;
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();

    let engine_task = tokio::spawn(drive_engine_with_vad_frames(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::VadPause,
        VecDeque::from([VadFrame::Speech, VadFrame::Silence]),
    ));

    pcm_tx.send(signal_frame(800)).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    pcm_tx.send(silence_frame(512)).unwrap();
    wait_until_last_sent(&sent).await;
    drain_state_events(&mut state_rx);

    pcm_tx.send(tone_frame(288, 2_000)).unwrap();
    let meter = expect_audio_meter(&mut state_rx, "finalize is waiting for Done").await;
    assert!(meter.rms > 0.0 && meter.peak > 0.0, "{meter:?}");

    event_tx.send(AsrEvent::Done).await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    control.request_stop();
    drop(pcm_tx);

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(
        outcome.terminal_error.is_none(),
        "{:?}",
        outcome.terminal_error
    );
    assert_eq!(
        drain_audio_meter_count(&mut state_rx),
        0,
        "PCM drained during finalize must not emit meter again when replayed through idle VAD"
    );
    let calls = sent.lock().unwrap();
    assert!(
        !calls.iter().any(|(pcm, _)| pcm == &tone_frame(288, 2_000)),
        "PCM drained during finalize must not be sent to the ended ASR session: {calls:?}"
    );
}

#[tokio::test]
async fn vad_pause_emits_audio_meter_while_close_is_waiting() {
    let (event_tx, event_rx) = mpsc::channel(8);
    let (close_started_tx, close_started_rx) = oneshot::channel();
    let (close_release_tx, close_release_rx) = oneshot::channel();
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(DelayedCloseSession {
        sent: sent.clone(),
        close_started: close_started_tx,
        close_release: close_release_rx,
    });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (_, mut state_rx) = state.subscribe_with_snapshot();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let mut params = make_params(state, Some(overlay), true, 500);
    params.vad.pause_silence_ms = 1;
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();

    let engine_task = tokio::spawn(drive_engine_with_vad_frames(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::VadPause,
        VecDeque::from([VadFrame::Speech, VadFrame::Silence]),
    ));

    pcm_tx.send(signal_frame(800)).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    pcm_tx.send(silence_frame(512)).unwrap();
    wait_until_last_sent(&sent).await;
    event_tx.send(AsrEvent::Done).await.unwrap();
    close_started_rx
        .await
        .expect("session close should have started");
    drain_state_events(&mut state_rx);

    pcm_tx.send(tone_frame(288, 3_000)).unwrap();
    let meter = expect_audio_meter(&mut state_rx, "session close is waiting").await;
    assert!(meter.rms > 0.0 && meter.peak > 0.0, "{meter:?}");

    let _ = close_release_tx.send(());
    tokio::time::sleep(Duration::from_millis(30)).await;
    control.request_stop();
    drop(pcm_tx);

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(
        outcome.terminal_error.is_none(),
        "{:?}",
        outcome.terminal_error
    );
    assert_eq!(
        drain_audio_meter_count(&mut state_rx),
        0,
        "PCM drained during close must not emit meter again when replayed through idle VAD"
    );
    let calls = sent.lock().unwrap();
    assert!(
        !calls.iter().any(|(pcm, _)| pcm == &tone_frame(288, 3_000)),
        "PCM drained during close must not be sent to the ended ASR session: {calls:?}"
    );
}

#[tokio::test]
async fn vad_pause_emits_audio_meter_while_resume_open_is_waiting() {
    let (first_event_tx, first_event_rx) = mpsc::channel(8);
    let (second_event_tx, second_event_rx) = mpsc::channel(8);
    let (open_started_tx, open_started_rx) = oneshot::channel();
    let (open_release_tx, open_release_rx) = oneshot::channel();
    let first_sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let second_sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ScriptedProvider::new(vec![
        OpenScript::Ok(
            first_event_rx,
            Box::new(AutoDoneSession {
                sent: first_sent.clone(),
                event_tx: first_event_tx.clone(),
                done_after_n: 1,
            }),
        ),
        OpenScript::DelayOk {
            opened: second_event_rx,
            session: Box::new(RecordingSession {
                sent: second_sent.clone(),
            }),
            open_started: open_started_tx,
            release: open_release_rx,
        },
    ]));

    let state = StateStore::new();
    let (_, mut state_rx) = state.subscribe_with_snapshot();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), true, 500);
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();

    let engine_task = tokio::spawn(drive_engine_with_vad_frames(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::VadPause,
        VecDeque::from([VadFrame::Speech, VadFrame::Speech]),
    ));

    pcm_tx.send(tone_frame(512, 1_000)).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;

    pcm_tx.send(tone_frame(512, 1_500)).unwrap();
    open_started_rx
        .await
        .expect("second resume open should have started after SpeechStarted");
    drain_state_events(&mut state_rx);

    pcm_tx.send(tone_frame(800, 2_500)).unwrap();
    let meter = expect_audio_meter(&mut state_rx, "resume open is waiting").await;
    assert!(meter.rms > 0.0 && meter.peak > 0.0, "{meter:?}");

    let _ = open_release_tx.send(());
    tokio::time::sleep(Duration::from_millis(30)).await;
    control.request_stop();
    drop(pcm_tx);
    second_event_tx.send(AsrEvent::Done).await.unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(
        outcome.terminal_error.is_none(),
        "{:?}",
        outcome.terminal_error
    );
    let first_calls = first_sent.lock().unwrap();
    assert!(
        !first_calls.iter().any(|(pcm, _)| pcm.contains(&2_500)),
        "PCM drained during second resume open must not be sent to first session: {first_calls:?}"
    );
    let calls = second_sent.lock().unwrap();
    assert!(
        calls.iter().any(|(pcm, _)| pcm.contains(&2_500)),
        "PCM drained during resume open must be replayed into the new ASR session: {calls:?}"
    );
}

#[tokio::test]
async fn vad_pause_emits_audio_meter_while_resume_replay_send_is_waiting() {
    let (first_event_tx, first_event_rx) = mpsc::channel(8);
    let (second_event_tx, second_event_rx) = mpsc::channel(8);
    let (send_started_tx, send_started_rx) = oneshot::channel();
    let (send_release_tx, send_release_rx) = oneshot::channel();
    let first_sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let second_sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ScriptedProvider::new(vec![
        OpenScript::Ok(
            first_event_rx,
            Box::new(AutoDoneSession {
                sent: first_sent.clone(),
                event_tx: first_event_tx.clone(),
                done_after_n: 1,
            }),
        ),
        OpenScript::Ok(
            second_event_rx,
            Box::new(DelayedSendSession {
                sent: second_sent.clone(),
                delay_on_send: 1,
                send_started: Some(send_started_tx),
                send_release: Some(send_release_rx),
            }),
        ),
    ]));

    let state = StateStore::new();
    let (_, mut state_rx) = state.subscribe_with_snapshot();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let mut params = make_params(state, Some(overlay), true, 500);
    params.vad.pre_roll_ms = 64;
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();

    let engine_task = tokio::spawn(drive_engine_with_vad_frames(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::VadPause,
        VecDeque::from([VadFrame::Speech, VadFrame::Speech]),
    ));

    pcm_tx.send(tone_frame(512, 1_000)).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    drain_state_events(&mut state_rx);
    pcm_tx.send(tone_frame(512, 1_500)).unwrap();
    send_started_rx
        .await
        .expect("resume replay send should have started");
    assert!(
        drain_state_events_seen_active(&mut state_rx),
        "UI session phase should become Active before resume replay send completes"
    );

    pcm_tx.send(tone_frame(800, 2_500)).unwrap();
    let meter = expect_audio_meter(&mut state_rx, "resume replay send is waiting").await;
    assert!(meter.rms > 0.0 && meter.peak > 0.0, "{meter:?}");

    let _ = send_release_tx.send(());
    tokio::time::sleep(Duration::from_millis(30)).await;
    control.request_stop();
    drop(pcm_tx);
    second_event_tx.send(AsrEvent::Done).await.unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(
        outcome.terminal_error.is_none(),
        "{:?}",
        outcome.terminal_error
    );
    let calls = second_sent.lock().unwrap();
    assert!(
        calls.iter().any(|(pcm, _)| pcm.contains(&1_500)),
        "resume replay must reach the new ASR session: {calls:?}"
    );
    assert!(
        calls.iter().any(|(pcm, _)| pcm.contains(&2_500)),
        "PCM drained during resume replay send must be flushed into the new ASR session: {calls:?}"
    );
    assert!(
        calls.iter().position(|(pcm, _)| pcm.contains(&1_500))
            < calls.iter().position(|(pcm, _)| pcm.contains(&2_500)),
        "resume replay must be sent before live PCM drained during replay: {calls:?}"
    );
}

#[tokio::test]
async fn vad_pause_replays_all_pending_idle_chunks_after_first_queued_speech_start() {
    let (first_event_tx, first_event_rx) = mpsc::channel(8);
    let (second_event_tx, second_event_rx) = mpsc::channel(8);
    let first_sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let second_sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ScriptedProvider::new(vec![
        OpenScript::Ok(
            first_event_rx,
            Box::new(RecordingSession {
                sent: first_sent.clone(),
            }),
        ),
        OpenScript::Ok(
            second_event_rx,
            Box::new(RecordingSession {
                sent: second_sent.clone(),
            }),
        ),
    ]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let mut params = make_params(state, Some(overlay), true, 500);
    params.vad.pause_silence_ms = 1;
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();

    let engine_task = tokio::spawn(drive_engine_with_vad_frames(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::VadPause,
        VecDeque::from([
            VadFrame::Speech,
            VadFrame::Silence,
            VadFrame::Speech,
            VadFrame::Speech,
        ]),
    ));

    pcm_tx.send(tone_frame(800, 1_000)).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    pcm_tx.send(silence_frame(512)).unwrap();
    wait_until_last_sent(&first_sent).await;
    pcm_tx.send(tone_frame(512, 2_000)).unwrap();
    pcm_tx.send(tone_frame(512, 3_000)).unwrap();
    first_event_tx.send(AsrEvent::Done).await.unwrap();

    tokio::time::sleep(Duration::from_millis(80)).await;
    control.request_stop();
    drop(pcm_tx);
    second_event_tx.send(AsrEvent::Done).await.unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(
        outcome.terminal_error.is_none(),
        "{:?}",
        outcome.terminal_error
    );
    let calls = second_sent.lock().unwrap();
    assert!(
        calls.iter().any(|(pcm, _)| pcm.contains(&2_000)),
        "first pending speech chunk must be replayed into new session: {calls:?}"
    );
    assert!(
        calls.iter().any(|(pcm, _)| pcm.contains(&3_000)),
        "later pending chunk after SpeechStarted must not be dropped: {calls:?}"
    );
}

#[tokio::test]
async fn cancel_during_vad_pause_resume_open_does_not_install_new_session() {
    let (first_event_tx, first_event_rx) = mpsc::channel(8);
    let (second_event_tx, second_event_rx) = mpsc::channel(8);
    let (open_started_tx, open_started_rx) = oneshot::channel();
    let (open_release_tx, open_release_rx) = oneshot::channel();
    let first_sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let second_sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let second_closes = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(ScriptedProvider::new(vec![
        OpenScript::Ok(
            first_event_rx,
            Box::new(AutoDoneSession {
                sent: first_sent.clone(),
                event_tx: first_event_tx.clone(),
                done_after_n: 1,
            }),
        ),
        OpenScript::DelayOk {
            opened: second_event_rx,
            session: Box::new(CloseCountingSession {
                sent: second_sent.clone(),
                closes: second_closes.clone(),
            }),
            open_started: open_started_tx,
            release: open_release_rx,
        },
    ]));

    let state = StateStore::new();
    let (_, mut state_rx) = state.subscribe_with_snapshot();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), true, 500);
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();

    let engine_task = tokio::spawn(drive_engine_with_vad_frames(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::VadPause,
        VecDeque::from([VadFrame::Speech, VadFrame::Speech]),
    ));

    pcm_tx.send(tone_frame(512, 1_000)).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    pcm_tx.send(tone_frame(512, 1_500)).unwrap();
    open_started_rx
        .await
        .expect("second resume open should have started");
    drain_state_events(&mut state_rx);

    control.request_cancel();
    pcm_tx.send(tone_frame(800, 2_500)).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        drain_audio_meter_count(&mut state_rx),
        0,
        "cancel during resume open must stop draining capture and emitting meter"
    );
    let _ = open_release_tx.send(());
    drop(pcm_tx);
    drop(second_event_tx);

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(outcome.cancel_requested);
    assert!(outcome.terminal_error.is_none());
    assert!(
        second_sent.lock().unwrap().is_empty(),
        "cancel during resume open must not replay PCM into newly opened session"
    );
    assert_eq!(
        second_closes.load(Ordering::SeqCst),
        1,
        "cancel during resume open must close a session that opened after cancel"
    );
}

#[tokio::test]
async fn stop_during_vad_pause_resume_open_does_not_install_new_session() {
    let (first_event_tx, first_event_rx) = mpsc::channel(8);
    let (second_event_tx, second_event_rx) = mpsc::channel(8);
    let (open_started_tx, open_started_rx) = oneshot::channel();
    let (open_release_tx, open_release_rx) = oneshot::channel();
    let first_sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let second_sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let second_closes = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(ScriptedProvider::new(vec![
        OpenScript::Ok(
            first_event_rx,
            Box::new(AutoDoneSession {
                sent: first_sent.clone(),
                event_tx: first_event_tx.clone(),
                done_after_n: 1,
            }),
        ),
        OpenScript::DelayOk {
            opened: second_event_rx,
            session: Box::new(CloseCountingSession {
                sent: second_sent.clone(),
                closes: second_closes.clone(),
            }),
            open_started: open_started_tx,
            release: open_release_rx,
        },
    ]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), true, 500);
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();

    let engine_task = tokio::spawn(drive_engine_with_vad_frames(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::VadPause,
        VecDeque::from([VadFrame::Speech, VadFrame::Speech]),
    ));

    pcm_tx.send(tone_frame(512, 1_000)).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    pcm_tx.send(tone_frame(512, 1_500)).unwrap();
    open_started_rx
        .await
        .expect("second resume open should have started");

    control.request_stop();
    pcm_tx.send(tone_frame(800, 2_500)).unwrap();
    let _ = open_release_tx.send(());
    drop(pcm_tx);
    drop(second_event_tx);

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(!outcome.cancel_requested);
    assert!(outcome.terminal_error.is_none());
    assert!(
        second_sent.lock().unwrap().is_empty(),
        "stop during resume open must not replay PCM into newly opened session"
    );
    assert_eq!(
        second_closes.load(Ordering::SeqCst),
        1,
        "stop during resume open must close a session that opened after stop"
    );
}

/// VadPause 初始 Idle 静音可以持续超过 first-audio watchdog；
/// 之后第一次 speech 不应被旧 deadline 误判成 no_audio。
#[tokio::test]
async fn vad_pause_speech_after_long_initial_idle_does_not_trip_no_audio_watchdog() {
    let (event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent: sent.clone() });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), true, 200);
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();

    let engine_task = tokio::spawn(drive_engine_with_vad_frames(
        provider.clone(),
        params,
        control.clone(),
        rec,
        RecordingMode::VadPause,
        VecDeque::from([
            VadFrame::Silence,
            VadFrame::Silence,
            VadFrame::Silence,
            VadFrame::Silence,
            VadFrame::Speech,
        ]),
    ));

    for _ in 0..4 {
        pcm_tx.send(silence_frame(512)).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert_eq!(provider.opens(), 0);

    pcm_tx.send(signal_frame(512)).unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(provider.opens(), 1);

    control.request_stop();
    drop(pcm_tx);
    event_tx
        .send(AsrEvent::Segment {
            text: "late".into(),
            started_at: Instant::now(),
            ended_at: Instant::now(),
        })
        .await
        .unwrap();
    event_tx.send(AsrEvent::Done).await.unwrap();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(
        outcome.terminal_error.is_none(),
        "late first speech in VadPause must not trip no_audio watchdog: {:?}",
        outcome.terminal_error
    );
    assert_eq!(outcome.sessions.len(), 1);
    assert!(sent.lock().unwrap().iter().any(|(_, last)| !*last));
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
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider.clone(),
        params,
        control.clone(),
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(50)).await;
    control.request_cancel();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(outcome.cancel_requested);
    assert!(outcome.terminal_error.is_none());
}

#[tokio::test]
async fn cancel_during_active_preserves_visible_partial_text() {
    let (event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent: sent.clone() });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), false, 200);
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider.clone(),
        params,
        control.clone(),
        rec,
        RecordingMode::Continuous,
    ));

    event_tx
        .send(AsrEvent::Partial {
            text: "visible text".into(),
            seq: 1,
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    control.request_cancel();

    let outcome = engine_task.await.unwrap().expect("engine returned None");
    assert!(outcome.cancel_requested);
    assert_eq!(outcome.sessions.len(), 1);
    assert_eq!(
        crate::voice::capture::session_text(&outcome.sessions[0]),
        "visible text"
    );
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
    let control = SessionControl::new();
    let (pcm_tx, pcm_rx) = mpsc::unbounded_channel();
    let (rec, finish_rx) = RecordingStream::for_test_observe(pcm_rx);
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(50)).await;
    control.request_cancel();

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
    let control = SessionControl::new();
    let (_pcm_tx, pcm_rx) = mpsc::unbounded_channel();
    let (rec, finish_rx) = RecordingStream::for_test_observe(pcm_rx);
    // 不喂任何 PCM；立即取消（biased select 让 Cancel 抢在 1s watchdog 之前）。

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(20)).await;
    control.request_cancel();

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
    let control = SessionControl::new();
    let (pcm_tx, pcm_rx) = mpsc::unbounded_channel();
    let (rec, finish_rx) = RecordingStream::for_test_observe(pcm_rx);
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(50)).await;
    control.request_stop();
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

/// Stop residual drain 是保尾字的优化，不能因为 capture 后端不关闭 PCM
/// 通道而阻塞 ASR finalize / post 流水线。
#[tokio::test]
async fn stop_drain_timeout_still_finalizes_session() {
    let (event_tx, event_rx) = mpsc::channel(8);
    let sent: SentLog = Arc::new(Mutex::new(Vec::new()));
    let session = Box::new(RecordingSession { sent: sent.clone() });
    let provider = Arc::new(ScriptedProvider::new(vec![OpenScript::Ok(
        event_rx, session,
    )]));

    let state = StateStore::new();
    let (overlay, _overlay_rx) = OverlayHandle::channel();
    let params = make_params(state, Some(overlay), false, 200);
    let control = SessionControl::new();
    let (pcm_tx, pcm_rx) = mpsc::unbounded_channel();
    let (rec, _finish_rx) = RecordingStream::for_test_observe(pcm_rx);
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::Continuous,
    ));

    tokio::time::sleep(Duration::from_millis(50)).await;
    control.request_stop();
    // Deliberately keep pcm_tx open. A correct engine must bound residual drain
    // and still send is_last/finalize.
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
    assert!(sent.lock().unwrap().iter().any(|(_, last)| *last));
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
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();
    pcm_tx.send(signal_frame(480)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control.clone(),
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
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();
    pcm_tx.send(signal_frame(480)).unwrap();

    let outcome = drive_engine(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::Continuous,
    )
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
    let control = SessionControl::new();
    let (rec, _pcm_tx) = make_recorder();

    let outcome = drive_engine(
        provider.clone(),
        params,
        control.clone(),
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
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();

    // 全零帧低于 MIN_NONZERO_AMPLITUDE，无法触发 first_audio_seen。
    pcm_tx.send(vec![0i16; 480]).unwrap();
    pcm_tx.send(vec![0i16; 480]).unwrap();

    let outcome = drive_engine(
        provider.clone(),
        params,
        control.clone(),
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
    let control = SessionControl::new();
    let (rec, pcm_tx) = make_recorder();
    pcm_tx.send(signal_frame(480)).unwrap();
    pcm_tx.send(signal_frame(960)).unwrap();

    let engine_task = tokio::spawn(drive_engine(
        provider,
        params,
        control.clone(),
        rec,
        RecordingMode::Continuous,
    ));
    tokio::time::sleep(Duration::from_millis(60)).await;
    control.request_stop();
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
