//! Provider session 收口：发 `send_pcm([], true)`，等 `Final` / `Segment` /
//! `Done`、timeout、cancel、ASR error，把结果归一成 `FinalizeOutcome`。
//!
//! 不负责 dispatch、history、overlay 状态切换；只把 ASR session 的最后一段
//! 数据收齐到 `pending_segments` / `session_final_text`。

use std::time::Duration;
use std::time::Instant;

use tokio::sync::mpsc;

use crate::asr::types::{AsrEvent, AsrSession};
use crate::history::HistoryError;
use crate::overlay::{OverlayCmd, OverlayHandle, TextKind};
use crate::state::StateStore;
use crate::voice::capture::SegmentCapture;
use crate::voice::observer::instant_elapsed_ms;
use crate::voice::CancelSignal;

#[derive(Debug, Default)]
pub(crate) struct TranscriptDisplay {
    segments: Vec<String>,
    partial: String,
}

impl TranscriptDisplay {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn append_segment(&mut self, text: String) {
        self.segments.push(text);
        self.partial.clear();
    }

    pub(crate) fn replace_recent_segments(&mut self, segments: usize, text: String) {
        let keep = self.segments.len().saturating_sub(segments);
        self.segments.truncate(keep);
        if !text.is_empty() {
            self.segments.push(text);
        }
        self.partial.clear();
    }

    pub(crate) fn set_partial(&mut self, text: String) {
        self.partial = text;
    }

    pub(crate) fn words(&self) -> u32 {
        let mut text = self.segments.join("");
        text.push_str(&self.partial);
        crate::text_stats::compute(&text).words as u32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FinalizeOutcome {
    Done,
    Canceled,
}

/// 把 voice 视角的 ASR session 收尾原子化。
///
/// 返回值：
///   - `Ok(Done)`：收到 `Done`。
///   - `Ok(Canceled)`：等待期间 `cancel` token 被取消；调用方负责清理。
///   - `Err(asr_send_last)`：`send_pcm(&[], true)` 失败。
///   - `Err(asr_timeout)`：`finalize_timeout_ms` 内未收到 `Done`。
///   - `Err(asr_stream_closed)`：provider 未发 `Done` 就关闭事件通道。
///   - `Err(asr_error)`：provider 在 finalize 阶段报告 terminal error。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn finalize_provider_session(
    session: &mut Box<dyn AsrSession>,
    events: &mut mpsc::Receiver<AsrEvent>,
    pending_segments: &mut Vec<SegmentCapture>,
    session_final_text: &mut Option<String>,
    // 与录音主循环的 `current.partial_text` 同源：finalize 阶段（send_last 之后）
    // 才到达的 tentative Partial 也要落进快照，否则「只发过 finalize 期 partial 就
    // asr_timeout」的记录 asr.text 为空、resume 无法恢复。Segment/Final 到达即清。
    session_partial_text: &mut String,
    pending_overlay_segments: &mut usize,
    finalize_timeout_ms: u64,
    cancel: CancelSignal<'_>,
    terminal_error: &mut Option<HistoryError>,
    recording_started_instant: Instant,
    observed_events: &mut Vec<(u64, AsrEvent)>,
    transcript: &mut TranscriptDisplay,
    state: &StateStore,
    recording_id: &str,
    overlay: Option<&OverlayHandle>,
) -> Result<FinalizeOutcome, HistoryError> {
    let timeout = tokio::time::sleep(Duration::from_millis(finalize_timeout_ms));
    tokio::pin!(timeout);
    let send_last = session.send_pcm(&[], true);
    tokio::pin!(send_last);
    tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            return Ok(FinalizeOutcome::Canceled);
        }
        _ = &mut timeout => {
            tracing::warn!(
                recording_id,
                timeout_ms = finalize_timeout_ms,
                pending_segments = pending_segments.len(),
                final_seen = session_final_text.is_some(),
                "ASR finalize timed out"
            );
            return Err(HistoryError {
                kind: "asr_timeout".to_string(),
                msg: "timeout waiting final".to_string(),
            });
        }
        result = &mut send_last => {
            if let Err(e) = result {
                return Err(HistoryError {
                    kind: "asr_send_last".to_string(),
                    msg: e.to_string(),
                });
            }
        }
    }

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                return Ok(FinalizeOutcome::Canceled);
            }
            _ = &mut timeout => {
                tracing::warn!(
                    recording_id,
                    timeout_ms = finalize_timeout_ms,
                    pending_segments = pending_segments.len(),
                    final_seen = session_final_text.is_some(),
                    "ASR finalize timed out"
                );
                return Err(HistoryError {
                    kind: "asr_timeout".to_string(),
                    msg: "timeout waiting final".to_string(),
                });
            }
            ev = events.recv() => {
                match ev {
                    None => {
                        return Err(HistoryError {
                            kind: "asr_stream_closed".to_string(),
                            msg: "ASR event stream closed before Done".to_string(),
                        });
                    }
                    Some(AsrEvent::Final { text }) => {
                        observed_events.push((
                            instant_elapsed_ms(recording_started_instant),
                            AsrEvent::Final { text: text.clone() },
                        ));
                        *session_final_text = Some(text.clone());
                        session_partial_text.clear();
                        transcript.replace_recent_segments(*pending_overlay_segments, text.clone());
                        if let Some(overlay) = overlay {
                            overlay.send(OverlayCmd::ReplaceRecentSegments {
                                segments: *pending_overlay_segments,
                                text,
                            });
                        }
                        *pending_overlay_segments = 1;
                        emit_stats(
                            transcript,
                            recording_started_instant,
                            state,
                            recording_id,
                            overlay,
                        );
                    }
                    Some(AsrEvent::Done) => {
                        observed_events.push((
                            instant_elapsed_ms(recording_started_instant),
                            AsrEvent::Done,
                        ));
                        return Ok(FinalizeOutcome::Done);
                    }
                    Some(AsrEvent::Segment { text, started_at, ended_at }) => {
                        observed_events.push((
                            instant_elapsed_ms(recording_started_instant),
                            AsrEvent::Segment {
                                text: text.clone(),
                                started_at,
                                ended_at,
                            },
                        ));
                        session_partial_text.clear();
                        state.segment(recording_id.to_string(), text.clone());
                        // finalize 阶段拿到的 definite segment 也要喂 overlay，
                        // 否则 Doubao 在 is_last 之后才"升级"出来的尾段全丢。
                        if let Some(overlay) = overlay {
                            overlay.send(OverlayCmd::AppendSegment { text: text.clone() });
                        }
                        transcript.append_segment(text.clone());
                        emit_stats(
                            transcript,
                            recording_started_instant,
                            state,
                            recording_id,
                            overlay,
                        );
                        *pending_overlay_segments += 1;
                        pending_segments.push(SegmentCapture {
                            text,
                            started_at,
                            ended_at,
                        });
                    }
                    Some(AsrEvent::Partial { text, seq }) => {
                        observed_events.push((
                            instant_elapsed_ms(recording_started_instant),
                            AsrEvent::Partial {
                                text: text.clone(),
                                seq,
                            },
                        ));
                        // finalize 阶段 partial 也要喂 overlay：跟录音中
                        // handle_asr_event 对齐，保证 stop 后 ASR 还有输出时逐字流式
                        // 可见（LLM 提交兜底是安全网，主力就是这里）。
                        *session_partial_text = text.clone();
                        transcript.set_partial(text.clone());
                        state.partial(recording_id.to_string(), text.clone());
                        if let Some(overlay) = overlay {
                            overlay.send(OverlayCmd::SetText {
                                text,
                                kind: TextKind::Partial,
                            });
                        }
                        emit_stats(
                            transcript,
                            recording_started_instant,
                            state,
                            recording_id,
                            overlay,
                        );
                    }
                    Some(AsrEvent::Error { err }) => {
                        tracing::error!(recording_id = %recording_id, error = %err, "ASR event error during final");
                        observed_events.push((
                            instant_elapsed_ms(recording_started_instant),
                            AsrEvent::Error { err: err.clone() },
                        ));
                        let error = HistoryError {
                            kind: "asr_error".to_string(),
                            msg: err.to_string(),
                        };
                        *terminal_error = Some(error.clone());
                        return Err(error);
                    }
                }
            }
        }
    }
}

pub(crate) fn emit_stats(
    transcript: &TranscriptDisplay,
    recording_started_instant: Instant,
    state: &StateStore,
    recording_id: &str,
    overlay: Option<&OverlayHandle>,
) {
    let words = transcript.words();
    let dur_ms = recording_started_instant.elapsed().as_millis() as u64;
    state.stats(recording_id.to_string(), dur_ms, words);
    if let Some(overlay) = overlay {
        overlay.send(OverlayCmd::SetStats { dur_ms, words });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    use crate::asr::types::AsrError;

    struct NoopSession;

    #[async_trait]
    impl AsrSession for NoopSession {
        async fn send_pcm(&mut self, _pcm: &[i16], _is_last: bool) -> Result<(), AsrError> {
            Ok(())
        }

        async fn close(self: Box<Self>) -> Result<(), AsrError> {
            Ok(())
        }
    }

    type SendPcmCalls = Arc<Mutex<Vec<(Vec<i16>, bool)>>>;

    struct FinalizingSession {
        calls: SendPcmCalls,
        event_tx: mpsc::Sender<AsrEvent>,
    }

    #[async_trait]
    impl AsrSession for FinalizingSession {
        async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<(), AsrError> {
            self.calls.lock().unwrap().push((pcm.to_vec(), is_last));
            if is_last {
                self.event_tx.send(AsrEvent::Done).await.unwrap();
            }
            Ok(())
        }

        async fn close(self: Box<Self>) -> Result<(), AsrError> {
            Ok(())
        }
    }

    struct HangingFinalSendSession;

    #[async_trait]
    impl AsrSession for HangingFinalSendSession {
        async fn send_pcm(&mut self, _pcm: &[i16], is_last: bool) -> Result<(), AsrError> {
            if is_last {
                std::future::pending::<()>().await;
            }
            Ok(())
        }

        async fn close(self: Box<Self>) -> Result<(), AsrError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn channel_close_before_done_is_an_error() {
        let mut session: Box<dyn AsrSession> = Box::new(NoopSession);
        let (event_tx, mut events) = mpsc::channel(1);
        drop(event_tx);
        let mut pending_segments = Vec::new();
        let mut session_final_text = None;
        let mut session_partial_text = String::new();
        let mut pending_overlay_segments = 0;
        let cancel = CancellationToken::new();
        let mut terminal_error = None;
        let started = Instant::now();
        let mut observed_events = Vec::new();
        let mut transcript = TranscriptDisplay::new();

        let error = finalize_provider_session(
            &mut session,
            &mut events,
            &mut pending_segments,
            &mut session_final_text,
            &mut session_partial_text,
            &mut pending_overlay_segments,
            100,
            CancelSignal::new(&cancel),
            &mut terminal_error,
            started,
            &mut observed_events,
            &mut transcript,
            &StateStore::new(),
            "test-recording",
            None,
        )
        .await
        .expect_err("channel close without Done must fail");

        assert_eq!(error.kind, "asr_stream_closed");
    }

    #[tokio::test]
    async fn final_send_is_covered_by_finalize_timeout() {
        let mut session: Box<dyn AsrSession> = Box::new(HangingFinalSendSession);
        let (_event_tx, mut events) = mpsc::channel(1);
        let mut pending_segments = Vec::new();
        let mut session_final_text = None;
        let mut session_partial_text = String::new();
        let mut pending_overlay_segments = 0;
        let cancel = CancellationToken::new();
        let mut terminal_error = None;
        let started = Instant::now();
        let mut observed_events = Vec::new();
        let mut transcript = TranscriptDisplay::new();

        let error = tokio::time::timeout(
            Duration::from_millis(200),
            finalize_provider_session(
                &mut session,
                &mut events,
                &mut pending_segments,
                &mut session_final_text,
                &mut session_partial_text,
                &mut pending_overlay_segments,
                20,
                CancelSignal::new(&cancel),
                &mut terminal_error,
                started,
                &mut observed_events,
                &mut transcript,
                &StateStore::new(),
                "test-recording",
                None,
            ),
        )
        .await
        .expect("finalize timeout must cover final send")
        .unwrap_err();

        assert_eq!(error.kind, "asr_timeout");
    }

    #[tokio::test]
    async fn asr_error_during_finalize_is_returned_even_if_channel_closes() {
        let mut session: Box<dyn AsrSession> = Box::new(NoopSession);
        let (event_tx, mut events) = mpsc::channel(1);
        event_tx
            .send(AsrEvent::Error {
                err: AsrError::Server("provider failed".to_string()),
            })
            .await
            .unwrap();
        drop(event_tx);
        let mut pending_segments = Vec::new();
        let mut session_final_text = None;
        let mut session_partial_text = String::new();
        let mut pending_overlay_segments = 0;
        let cancel = CancellationToken::new();
        let mut terminal_error = None;
        let started = Instant::now();
        let mut observed_events = Vec::new();
        let mut transcript = TranscriptDisplay::new();

        let error = finalize_provider_session(
            &mut session,
            &mut events,
            &mut pending_segments,
            &mut session_final_text,
            &mut session_partial_text,
            &mut pending_overlay_segments,
            100,
            CancelSignal::new(&cancel),
            &mut terminal_error,
            started,
            &mut observed_events,
            &mut transcript,
            &StateStore::new(),
            "test-recording",
            None,
        )
        .await
        .expect_err("ASR error during finalize must fail with original error");

        assert_eq!(error.kind, "asr_error");
        assert_eq!(error.msg, "server: provider failed");
    }

    /// finalize 阶段（send_last 之后）才到达的 tentative Partial 也要落进
    /// `session_partial_text`，这样「只发过 finalize 期 partial 就 asr_timeout」的
    /// 记录仍带可恢复文本（供 resume 续写）。
    #[tokio::test]
    async fn finalize_partial_is_captured_for_recoverable_snapshot() {
        let mut session: Box<dyn AsrSession> = Box::new(NoopSession);
        let (event_tx, mut events) = mpsc::channel(4);
        event_tx
            .send(AsrEvent::Partial {
                text: "half said".to_string(),
                seq: 1,
            })
            .await
            .unwrap();
        let mut pending_segments = Vec::new();
        let mut session_final_text = None;
        let mut session_partial_text = String::new();
        let mut pending_overlay_segments = 0;
        let cancel = CancellationToken::new();
        let mut terminal_error = None;
        let started = Instant::now();
        let mut observed_events = Vec::new();
        let mut transcript = TranscriptDisplay::new();

        let error = finalize_provider_session(
            &mut session,
            &mut events,
            &mut pending_segments,
            &mut session_final_text,
            &mut session_partial_text,
            &mut pending_overlay_segments,
            30,
            CancelSignal::new(&cancel),
            &mut terminal_error,
            started,
            &mut observed_events,
            &mut transcript,
            &StateStore::new(),
            "test-recording",
            None,
        )
        .await
        .expect_err("no Done after finalize partial must time out");

        assert_eq!(error.kind, "asr_timeout");
        assert_eq!(
            session_partial_text, "half said",
            "finalize-phase partial must be captured for the recoverable snapshot"
        );
        assert!(session_final_text.is_none());
    }

    #[tokio::test]
    async fn stop_finalize_sends_last_and_waits_for_done() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (event_tx, mut events) = mpsc::channel(1);
        let mut session: Box<dyn AsrSession> = Box::new(FinalizingSession {
            calls: calls.clone(),
            event_tx,
        });
        let mut pending_segments = Vec::new();
        let mut session_final_text = None;
        let mut session_partial_text = String::new();
        let mut pending_overlay_segments = 0;
        let cancel = CancellationToken::new();
        let mut terminal_error = None;
        let started = Instant::now();
        let mut observed_events = Vec::new();
        let mut transcript = TranscriptDisplay::new();

        let outcome = finalize_provider_session(
            &mut session,
            &mut events,
            &mut pending_segments,
            &mut session_final_text,
            &mut session_partial_text,
            &mut pending_overlay_segments,
            100,
            CancelSignal::new(&cancel),
            &mut terminal_error,
            started,
            &mut observed_events,
            &mut transcript,
            &StateStore::new(),
            "test-recording",
            None,
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert_eq!(outcome, FinalizeOutcome::Done);
        assert!(matches!(observed_events.as_slice(), [(_, AsrEvent::Done)]));
        assert!(
            observed_events[0].0 < started.elapsed().as_millis() as u64,
            "observed ASR event timestamp must be captured during finalize, not recomputed later"
        );
        assert_eq!(*calls.lock().unwrap(), vec![(Vec::new(), true)]);
    }
}
