# asr — ASR provider 抽象

**TL;DR**：`AsrProvider`/`AsrSession` 是硬边界，新增 provider 不改 trait（要改先重写设计并讨论）；单事件流；provider 私有配置各自 deserialize，voice 层永不见。

> **何时读**：新增/改 ASR provider、改 `AsrEvent` 语义。
> **不在这里**：voice 怎么消费事件流见 [voice](voice.md)；provider 各家协议见各自实现文件顶部链接（官方文档不扒本地）。
> **代码**：`src/asr/types.rs`(trait+AsrEvent+AsrError) / `providers/{apple,doubao}.rs` / `fake.rs`(测试)。

## 硬契约

接口按 voice 模块语义需要设计，不按某家协议反推。

- **流式 partial 必需**，不是可选 cap。不支持原生流式也无法包装成流式的 provider 不入选。
- **单事件流**：partial/segment/final/error/done 走同一根 channel（`AsrEvent` enum），voice 只 select 一臂。
- **codec 在 provider 实现里写死**，不暴露用户。codec 是工程权衡（CPU/带宽/server 兼容），provider 作者拍板。
- `send_pcm(is_last=true)` 后：正常结束发 `Done`；有最终文本用 `Segment` 或 `Final` 表达；无文本可直接 `Done`。
- provider 未发 `Done` 就关事件流、或 voice 发 PCM 失败 → 视为 terminal error：保留已确认 segment 到 error history，跳过 post/dispatch（截断文本当成功是 bug）。
- `Final` 是可选的 session 级最终全文；不支持时 voice 用 Segment 拼接 fallback。

## AsrEvent 语义（契约 owner，消费方就照这个）

- `Partial{text,seq}`：当前 utterance 尾巴，被后续 Partial 覆盖。
- `Segment{text,started_at,ended_at}`：句末（server VAD 或 is_last 后）。直接 concat 即最终文本——provider 保证段间无需分隔符（Doubao 自带句末标点；不自带的在 adapter 内部补）。
- `Final{text}`：可选 session 最终全文。
- `Error{err}`：不混进 `Result`，让 voice 决定降级。`AsrError::Canceled` voice 静默处理，不报 stderr/不发 error overlay。
- `Done`：session 终结。

`AsrError` 用 thiserror 结构化（Auth/Network/Quota/Protocol/Timeout/Server/Canceled），voice/overlay/doctor 直接 match，零字符串解析。

## 新增 provider 配方

1. impl `AsrProvider`（`name`/`caps`/`open`）+ `AsrSession`（`send_pcm`/`close`）。
2. 自己 deserialize `~/.config/shuohua/asr/<name>.toml`（文件名 == provider 名）；私有字段 voice/config 层不见。
3. canonical 输入是 `send_pcm(&[i16])`（16kHz mono s16le）；provider 内部自行转目标格式。
4. `finalize_timeout_ms` 是 provider 私有默认（Doubao 12s、Apple 5s）；正常 final < 1s。
5. `hotwords` 是 `Vec<String>`，provider 自由解释（Doubao 接词列表、Apple `contextualStrings`）；不支持就静默忽略（`caps().hotwords=false`，doctor 提示）。
6. 实现文件顶部 `//!` 放官方协议/SDK 链接，quirk 写代码注释，不进本文档。

## provider 映射

| Provider | Partial | Segment | Final | 备注 |
|---|---|---|---|---|
| Apple SpeechAnalyzer (macOS 26+) | `isFinal=false` | `isFinal=true` | 不发，voice fallback concat | 本地优先；Swift helper 桥接；26 以下 fail-fast |
| Doubao SAUC | `definite=false` | `definite=true` | last response `result.text` | 云端；codec 写死 raw PCM |
| 纯 batch ASR API | — | — | — | **不入选** |

## 错误处理策略

不自动重试（用户重新触发录音即可，自动重试隐藏失败、增加调试难度）。dispatch 只在拼完才写剪贴板，没收到末段不上屏。
