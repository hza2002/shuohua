//! 测试用 fake provider。给 voice 状态机单测使用（M2.f）。
//!
//! 行为：
//!   - 每次 send_pcm 立刻产 1 个 Partial
//!   - send_pcm(is_last=true) → 产 1 个 Segment 然后 Done
//!   - 可选：构造时塞个 ScriptedError 触发指定的 AsrError 路径
//!
//! 故意写得简单透明，不模拟真实 ASR 的延迟 / 部分丢字 / 重新识别。
//! 测的是 trait 契约的边界，不是真实识别质量。

use super::types::*;
use async_trait::async_trait;
use std::time::Instant;
use tokio::sync::mpsc;

pub struct FakeProvider {
    pub fail_on_open: Option<AsrError>,
}

impl FakeProvider {
    pub fn new() -> Self {
        Self { fail_on_open: None }
    }

    pub fn failing(err: AsrError) -> Self {
        Self { fail_on_open: Some(err) }
    }
}

#[async_trait]
impl AsrProvider for FakeProvider {
    fn name(&self) -> &str {
        "fake"
    }

    fn caps(&self) -> Caps {
        Caps { hotwords: true, max_session_secs: None, multilingual: true }
    }

    async fn open(
        &self,
        _ctx: SessionCtx,
    ) -> Result<(Box<dyn AsrSession>, mpsc::Receiver<AsrEvent>), AsrError> {
        if let Some(err) = &self.fail_on_open {
            return Err(err.clone());
        }
        let (tx, rx) = mpsc::channel(16);
        Ok((Box::new(FakeSession { evt_tx: tx, seq: 0, started_at: Instant::now() }), rx))
    }
}

struct FakeSession {
    evt_tx: mpsc::Sender<AsrEvent>,
    seq: u64,
    started_at: Instant,
}

#[async_trait]
impl AsrSession for FakeSession {
    async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<(), AsrError> {
        self.seq += 1;
        let text = format!("frame#{} ({} samples)", self.seq, pcm.len());
        self.evt_tx
            .send(AsrEvent::Partial { text: text.clone(), seq: self.seq })
            .await
            .map_err(|_| AsrError::Network("fake receiver dropped".into()))?;

        if is_last {
            let final_text = format!("final after {} frames", self.seq);
            self.evt_tx
                .send(AsrEvent::Segment {
                    text: final_text,
                    started_at: self.started_at,
                    ended_at: Instant::now(),
                })
                .await
                .ok();
            self.evt_tx.send(AsrEvent::Done).await.ok();
        }
        Ok(())
    }

    async fn close(self: Box<Self>) -> Result<(), AsrError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> SessionCtx {
        SessionCtx { language: LanguageMode::Single("zh-CN".into()), hotwords: vec![] }
    }

    #[tokio::test]
    async fn happy_path_emits_partial_then_segment_then_done() {
        let provider = FakeProvider::new();
        let (mut session, mut rx) = provider.open(ctx()).await.unwrap();

        // 喂两帧普通 PCM
        session.send_pcm(&[0i16; 100], false).await.unwrap();
        session.send_pcm(&[0i16; 100], false).await.unwrap();
        // 末帧
        session.send_pcm(&[0i16; 50], true).await.unwrap();

        // 期望事件序列：Partial, Partial, Partial, Segment, Done
        assert!(matches!(rx.recv().await, Some(AsrEvent::Partial { seq: 1, .. })));
        assert!(matches!(rx.recv().await, Some(AsrEvent::Partial { seq: 2, .. })));
        assert!(matches!(rx.recv().await, Some(AsrEvent::Partial { seq: 3, .. })));
        assert!(matches!(rx.recv().await, Some(AsrEvent::Segment { .. })));
        assert!(matches!(rx.recv().await, Some(AsrEvent::Done)));
    }

    #[tokio::test]
    async fn open_propagates_constructed_error() {
        let provider = FakeProvider::failing(AsrError::Auth("bad token".into()));
        match provider.open(ctx()).await {
            Err(AsrError::Auth(msg)) => assert!(msg.contains("bad token")),
            Err(other) => panic!("expected Auth, got {other}"),
            Ok(_) => panic!("expected Err"),
        }
    }

    #[tokio::test]
    async fn close_is_ok_after_done() {
        let provider = FakeProvider::new();
        let (mut session, _rx) = provider.open(ctx()).await.unwrap();
        session.send_pcm(&[0i16; 10], true).await.unwrap();
        session.close().await.unwrap();
    }
}
