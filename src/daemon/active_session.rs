use crate::voice::SessionControl;
use std::time::Duration;

pub(super) struct ActiveSession {
    control: SessionControl,
    join: tokio::task::JoinHandle<()>,
}

impl ActiveSession {
    pub(super) fn new(control: SessionControl, join: tokio::task::JoinHandle<()>) -> Self {
        Self { control, join }
    }

    pub(super) fn is_finished(&self) -> bool {
        self.join.is_finished()
    }

    pub(super) fn cancel(&self) {
        self.control.request_cancel();
    }

    pub(super) fn stop(&self) {
        self.control.request_stop();
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

impl Drop for ActiveSession {
    fn drop(&mut self) {
        // 守网：会话被异常丢弃（既未 stop 也未 cancel）时请求 stop，让 engine 收尾。
        // 复刻旧 watch sender-drop → stop 语义。正常路径已显式 stop/cancel，此处 no-op。
        if !self.control.is_stop_requested() && !self.control.is_cancelled() {
            self.control.request_stop();
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

    use crate::history::HistoryStatus;
    use crate::post::{self, PipelineText, PostChain, PostError, PostProcessor};
    use crate::voice::post_dispatch::dispatch_with_post_chain;

    fn session() -> (ActiveSession, SessionControl) {
        let control = SessionControl::new();
        let join = tokio::spawn(std::future::pending());
        (ActiveSession::new(control.clone(), join), control)
    }

    #[tokio::test]
    async fn stop_after_cancel_stays_canceled() {
        // 两个独立的终态闩可以同时置位；观察方一律先查 cancel，所以 stop 不会把语义
        // 降级回 Stop —— cancel 永远优先。
        let (session, control) = session();

        session.cancel();
        session.stop();

        assert!(control.is_cancelled());
    }

    #[tokio::test]
    async fn stop_requests_stop_signal() {
        let (session, control) = session();

        session.stop();

        assert!(control.is_stop_requested());
        assert!(!control.is_cancelled());
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
        let (session, control) = session();
        let segment_texts = vec!["hello".into()];
        let app_context = post::AppContext::default();
        let future = dispatch_with_post_chain(
            &segment_texts,
            false,
            &app_context,
            &post_chain,
            60_000,
            None,
            control.cancel_signal(),
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
