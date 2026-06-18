//! ASR Provider abstraction.
//!
//! 设计契约见 docs/DESIGN.md §2.8。要点：
//!   - 流式 partial 是硬要求；非流式 provider 不入选
//!   - 单事件流 [`AsrEvent`]：partial / segment / final / error / done 走同一根 channel
//!   - provider 私有配置由 provider impl 自己从 `asr/<provider>.toml` 加载，
//!     voice 模块永远不见
//!   - 收到 `send_pcm(is_last=true)` 后，provider 必须至少出一个 Segment 然后 Done
//!   - 音频 codec 在 provider 内部写死，不暴露给用户

pub mod providers;
pub mod types;

#[cfg(test)]
pub mod fake;

pub use types::AsrProvider;
