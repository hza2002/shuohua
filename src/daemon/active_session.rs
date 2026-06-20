use crate::voice::SessionControl;
use std::time::Duration;

pub(super) struct ActiveSession {
    control: tokio::sync::watch::Sender<SessionControl>,
    join: tokio::task::JoinHandle<()>,
}

impl ActiveSession {
    pub(super) fn new(
        control: tokio::sync::watch::Sender<SessionControl>,
        join: tokio::task::JoinHandle<()>,
    ) -> Self {
        Self { control, join }
    }

    pub(super) fn is_finished(&self) -> bool {
        self.join.is_finished()
    }

    pub(super) fn cancel(&self) {
        let _ = self.control.send(SessionControl::Cancel);
    }

    pub(super) fn stop(&self) {
        self.control.send_if_modified(|control| {
            if matches!(*control, SessionControl::Cancel | SessionControl::Stop) {
                return false;
            }
            *control = SessionControl::Stop;
            true
        });
    }

    pub(super) async fn stop_and_join(&mut self, timeout: Duration) -> ShutdownStopResult {
        self.stop();
        match tokio::time::timeout(timeout, &mut self.join).await {
            Ok(Ok(())) => ShutdownStopResult::Stopped,
            Ok(Err(error)) => ShutdownStopResult::JoinError(error),
            Err(_) => {
                self.join.abort();
                ShutdownStopResult::TimedOut
            }
        }
    }
}

pub(super) enum ShutdownStopResult {
    Stopped,
    JoinError(tokio::task::JoinError),
    TimedOut,
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::sync::Notify;

    use crate::post::{self, PipelineText, PostChain, PostError, PostProcessor};
    use crate::state::history::HistoryStatus;
    use crate::voice::post_dispatch::dispatch_with_post_chain;

    fn session() -> (ActiveSession, tokio::sync::watch::Receiver<SessionControl>) {
        let (control_tx, control_rx) = tokio::sync::watch::channel(SessionControl::Idle);
        let join = tokio::spawn(std::future::pending());
        (ActiveSession::new(control_tx, join), control_rx)
    }

    #[tokio::test]
    async fn stop_does_not_overwrite_cancel() {
        let (session, control_rx) = session();

        session.cancel();
        session.stop();

        assert_eq!(*control_rx.borrow(), SessionControl::Cancel);
    }

    #[tokio::test]
    async fn stop_from_idle_keeps_stop_semantics() {
        let (session, control_rx) = session();

        session.stop();

        assert_eq!(*control_rx.borrow(), SessionControl::Stop);
    }

    struct BlockingProcessor {
        started: Arc<Notify>,
    }

    #[async_trait]
    impl PostProcessor for BlockingProcessor {
        fn name(&self) -> &str {
            "blocking"
        }

        async fn process(
            &self,
            _input: PipelineText,
            _ctx: &post::AppContext,
        ) -> Result<PipelineText, PostError> {
            self.started.notify_one();
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn cancel_remains_sticky_during_post_when_stop_follows() {
        let started = Arc::new(Notify::new());
        let post_chain = PostChain {
            name: "test".into(),
            processors: vec![Box::new(BlockingProcessor {
                started: Arc::clone(&started),
            })],
        };
        let (session, mut control_rx) = session();
        let segment_texts = vec!["hello".into()];
        let app_context = post::AppContext::default();
        let future = dispatch_with_post_chain(
            &segment_texts,
            false,
            &app_context,
            &post_chain,
            60_000,
            None,
            &mut control_rx,
        );
        tokio::pin!(future);

        tokio::select! {
            _ = started.notified() => {}
            outcome = &mut future => panic!("post completed before cancel: {:?}", outcome.status),
        }
        session.cancel();
        session.stop();

        let outcome = tokio::time::timeout(Duration::from_millis(100), future)
            .await
            .expect("cancel must remain sticky after a later stop");
        assert_eq!(outcome.status, HistoryStatus::Canceled);
        assert_eq!(outcome.final_text, "hello");
        assert!(outcome.pipeline.is_empty());
        assert!(outcome.error.is_none());
    }
}
