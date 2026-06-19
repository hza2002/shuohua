use crate::voice::SessionControl;

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
}
