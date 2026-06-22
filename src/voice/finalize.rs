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
use crate::overlay::{OverlayCmd, OverlayHandle};
use crate::state::StateStore;
use crate::voice::capture::SegmentCapture;
use crate::voice::observer::{observe_asr_event, RecordingObserver};
use crate::voice::CancelSignal;

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
    pending_overlay_segments: &mut usize,
    finalize_timeout_ms: u64,
    cancel: CancelSignal<'_>,
    terminal_error: &mut Option<HistoryError>,
    trace: &mut RecordingObserver,
    recording_started_instant: Instant,
    state: &StateStore,
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
            _ = cancel.cancelled() => {
                return Ok(FinalizeOutcome::Canceled);
            }
            _ = &mut timeout => {
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
                        observe_asr_event(
                            trace,
                            recording_started_instant,
                            &AsrEvent::Final { text: text.clone() },
                        );
                        *session_final_text = Some(text.clone());
                        if let Some(overlay) = overlay {
                            overlay.send(OverlayCmd::ReplaceRecentSegments {
                                segments: *pending_overlay_segments,
                                text,
                            });
                        }
                        *pending_overlay_segments = 1;
                    }
                    Some(AsrEvent::Done) => {
                        observe_asr_event(trace, recording_started_instant, &AsrEvent::Done);
                        return Ok(FinalizeOutcome::Done);
                    }
                    Some(AsrEvent::Segment { text, started_at, ended_at }) => {
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
                        *pending_overlay_segments += 1;
                        pending_segments.push(SegmentCapture {
                            text,
                            started_at,
                            ended_at,
                        });
                    }
                    Some(AsrEvent::Partial { text, seq }) => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    use crate::asr::types::AsrError;
    use crate::voice::observer::TraceStart;

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

    #[tokio::test]
    async fn channel_close_before_done_is_an_error() {
        let mut session: Box<dyn AsrSession> = Box::new(NoopSession);
        let (event_tx, mut events) = mpsc::channel(1);
        drop(event_tx);
        let mut pending_segments = Vec::new();
        let mut session_final_text = None;
        let mut pending_overlay_segments = 0;
        let cancel = CancellationToken::new();
        let mut terminal_error = None;
        let started = Instant::now();
        let mut trace = RecordingObserver::start(TraceStart {
            enabled: false,
            recording_id: "test-recording".to_string(),
            provider: "test".to_string(),
            started_at: "2026-06-19T00:00:00Z".to_string(),
            started_instant: started,
        });

        let error = finalize_provider_session(
            &mut session,
            &mut events,
            &mut pending_segments,
            &mut session_final_text,
            &mut pending_overlay_segments,
            100,
            CancelSignal::new(&cancel),
            &mut terminal_error,
            &mut trace,
            started,
            &StateStore::new(),
            "test-recording",
            None,
        )
        .await
        .expect_err("channel close without Done must fail");

        assert_eq!(error.kind, "asr_stream_closed");
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
        let mut pending_overlay_segments = 0;
        let cancel = CancellationToken::new();
        let mut terminal_error = None;
        let started = Instant::now();
        let mut trace = RecordingObserver::start(TraceStart {
            enabled: false,
            recording_id: "test-recording".to_string(),
            provider: "test".to_string(),
            started_at: "2026-06-19T00:00:00Z".to_string(),
            started_instant: started,
        });

        let error = finalize_provider_session(
            &mut session,
            &mut events,
            &mut pending_segments,
            &mut session_final_text,
            &mut pending_overlay_segments,
            100,
            CancelSignal::new(&cancel),
            &mut terminal_error,
            &mut trace,
            started,
            &StateStore::new(),
            "test-recording",
            None,
        )
        .await
        .expect_err("ASR error during finalize must fail with original error");

        assert_eq!(error.kind, "asr_error");
        assert_eq!(error.msg, "server: provider failed");
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
        let mut pending_overlay_segments = 0;
        let cancel = CancellationToken::new();
        let mut terminal_error = None;
        let started = Instant::now();
        let mut trace = RecordingObserver::start(TraceStart {
            enabled: false,
            recording_id: "test-recording".to_string(),
            provider: "test".to_string(),
            started_at: "2026-06-19T00:00:00Z".to_string(),
            started_instant: started,
        });

        let outcome = finalize_provider_session(
            &mut session,
            &mut events,
            &mut pending_segments,
            &mut session_final_text,
            &mut pending_overlay_segments,
            100,
            CancelSignal::new(&cancel),
            &mut terminal_error,
            &mut trace,
            started,
            &StateStore::new(),
            "test-recording",
            None,
        )
        .await
        .unwrap();

        assert_eq!(outcome, FinalizeOutcome::Done);
        assert_eq!(*calls.lock().unwrap(), vec![(Vec::new(), true)]);
    }
}
