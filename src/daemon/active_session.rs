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
        let _ = self.control.send(SessionControl::Stop);
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
