//! ASR trait + 数据类型。
//!
//! 设计依据见 docs/DESIGN.md §2.8。

use async_trait::async_trait;
use std::time::Instant;
use tokio::sync::mpsc;

#[async_trait]
pub trait AsrProvider: Send + Sync {
    fn name(&self) -> &str;
    fn caps(&self) -> Caps;
    /// 打开一个新 session。返回 (session handle, 事件流接收端)。
    /// 失败语义按 [`AsrError`] 分类（鉴权 / 网络 / 配额 / 协议）。
    async fn open(
        &self,
        ctx: SessionCtx,
    ) -> Result<(Box<dyn AsrSession>, mpsc::Receiver<AsrEvent>), AsrError>;
}

#[async_trait]
pub trait AsrSession: Send {
    /// 喂一帧 PCM。Canonical 格式 = 16kHz s16le mono（recorder 已做归一化）。
    ///
    /// is_last=true 表示后面没了。provider 必须在收到后**至少**吐一个
    /// `AsrEvent::Segment`，然后 `AsrEvent::Done`。
    async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<(), AsrError>;

    /// 主动关 session。多次调用应幂等。
    async fn close(self: Box<Self>) -> Result<(), AsrError>;
}

#[derive(Debug, Clone)]
pub struct Caps {
    /// false → SessionCtx.hotwords 静默忽略，doctor 会提示。
    pub hotwords: bool,
    /// provider 单 session 最长寿命（用于 voice 模块决定是否自动续 session）。
    pub max_session_secs: Option<u32>,
    /// 同 session 内是否支持 code-switch（中英混合）。
    pub multilingual: bool,
}

#[derive(Debug, Clone)]
pub struct SessionCtx {
    pub language: LanguageMode,
    /// 共享 hotwords，来自 `[asr] hotwords`。
    /// provider 自由解释：Doubao 直接塞 `corpus.context.hotwords`、
    /// Whisper 拼 `initial_prompt`、Apple SpeechAnalyzer 用 `contextualStrings`。
    pub hotwords: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum LanguageMode {
    /// "zh-CN" / "en-US"。
    Single(String),
    /// 中英混合走这个。hint = 主要可能语言列表。
    Multilingual { hint: Vec<String> },
}

/// 单事件流。voice 模块 select 这根 channel 就够了。
///
/// `started_at`/`ended_at` 用 [`Instant`]：在 daemon 进程内部做时长换算够用；
/// history.jsonl 写入时转 RFC3339（SCHEMA.md §2）由 history 模块自己映射。
#[derive(Debug, Clone)]
pub enum AsrEvent {
    /// 当前 utterance 最新猜测全文。会被后续 Partial 覆盖。
    Partial { text: String, seq: u64 },
    /// 句末（server VAD / definite=true / is_last 收尾）—— 不再变。
    Segment { text: String, started_at: Instant, ended_at: Instant },
    /// 非取消类错误。voice 模块决定降级策略。
    Error { err: AsrError },
    /// session 终结：is_last + 最后一段已发完。channel 应在此之后关闭。
    Done,
}

/// M3+ overlay 直接 `match err` 分发 toast 样式，零字符串解析。
/// `Canceled` 是 first-class：voice 模块静默处理、不报 stderr、不发 toast。
#[derive(thiserror::Error, Debug, Clone)]
pub enum AsrError {
    #[error("auth failed: {0}")]
    Auth(String),
    #[error("network: {0}")]
    Network(String),
    #[error("quota exceeded")]
    Quota,
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("timeout waiting final")]
    Timeout,
    #[error("server: {0}")]
    Server(String),
    #[error("canceled")]
    Canceled,
}
