# asr — ASR provider 抽象

**TL;DR**：`AsrProvider`/`AsrSession` 是硬边界，新增 provider 不改 trait（要改先重写设计并讨论）；单事件流；provider 私有配置各自 deserialize，voice 层永不见。

> **何时读**：新增/改 ASR provider、改 `AsrEvent` 语义。
> **不在这里**：voice 怎么消费事件流见 [voice](voice.md)；provider 各家协议见各自实现文件顶部链接（官方文档不扒本地）。
> **代码**：`src/asr/types.rs`(trait+AsrEvent+AsrError) / `providers/{apple,doubao,tencent}.rs` / `fake.rs`(测试)。

## 硬契约

接口按 voice 模块语义需要设计，不按某家协议反推。

- **流式 partial 必需**，不是可选 cap。不支持原生流式也无法包装成流式的 provider 不入选。
- **单事件流**：partial/segment/final/error/done 走同一根 channel（`AsrEvent` enum），voice 只 select 一臂。
- **codec 在 provider 实现里写死**，不暴露用户。codec 是工程权衡（CPU/带宽/server 兼容），provider 作者拍板。
- `send_pcm(is_last=true)` 后：正常结束发 `Done`；有最终文本用 `Segment` 或 `Final` 表达；无文本可直接 `Done`。
- provider 未发 `Done` 就关事件流、或 voice 发 PCM 失败 → 视为 terminal error：保留已确认 segment 到 error history，跳过 post/dispatch（截断文本当成功是 bug）。
- `Final` 是可选的 session 级最终全文；不支持时 voice 用 Segment 拼接 fallback。
- **session 与 `open()` 必须 drop-safe**：`close()` 是优雅路径，drop 是兜底；session 被 drop 而没走 `close()` 时，不得遗留子进程 / websocket 连接 / 后台任务。两条路径都得幂等，close 后再 drop 不能 double-kill / double-cancel 报错。

## AsrEvent 语义（契约 owner，消费方就照这个）

- `Partial{text,seq}`：当前 utterance 尾巴，被后续 Partial 覆盖。
- `Segment{text,started_at,ended_at}`：句末（server VAD 或 is_last 后）。直接 concat 即最终文本——provider 保证段间无需分隔符（Doubao 自带句末标点；不自带的在 adapter 内部补）。
- `Final{text}`：可选 session 最终全文。
- `Error{err}`：不混进 `Result`，让 voice 决定降级。`AsrError::Canceled` voice 静默处理，不报 stderr/不发 error overlay。
- `Done`：session 终结。

`AsrError` 用 thiserror 结构化（Auth/Network/Quota/Protocol/Timeout/Server/Canceled），voice/overlay/doctor 直接 match，零字符串解析。

## 配置实例模型

每个 ASR 配置是一个**文件 stem 实例**：

- 实例文件路径：`~/.config/shuohua/asr/<id>.toml`；文件 stem = 实例 ID。
- 文件内**必须**包含 `type = "apple" | "doubao" | "tencent"`；缺失或值不合法时 `resolve_instance` 报错，不回退默认。
- profile 通过 `[asr] instance = "<id>"` 引用实例 ID（不是实现名 apple/doubao）。
- resolver：`config::asr::instance::resolve_instance(id)` → `AsrInstance { id, kind: AsrKind, path, display_name }`；`kind_from_value` 读取 `type` 字段。
- `name` 是可选展示标签，不参与引用，不是引用键。

## 新增 ASR 实现配方

1. 在 `src/config/asr/instance.rs` 的 `AsrKind` 枚举添加新变体（如 `Whisper`），以及 `as_str`/`schema_id` 的对应分支。
2. 在 `kind_from_value` 的 match 添加新 `type` 字符串分支（如 `"whisper" => Ok(AsrKind::Whisper)`）。
3. 在 `src/config/schema.rs` 添加对应 `SchemaId`（如 `AsrWhisper`）并实现 spec builder，供 `spec_for_config_file` 路由。
4. 在 `src/asr/providers/` 实现 `AsrProvider`（`name`/`caps`/`open`）+ `AsrSession`（`send_pcm`/`close`）；provider 私有字段只在自身 deserialize，不暴露给 voice/config。
5. 在 `src/asr/providers/mod.rs` 的 `build_from_instance` 添加 `AsrKind::Whisper =>` 分支。
6. 在 `src/config/template/registry.rs` 注册一个 `kind: TemplateKind::Asr` 的模板（`asr/<name>`），否则 TUI 的 `asr_kind_ids()`（由 `asr_templates()` 生成）不会列出该 type，创建表单也无法选它。
7. canonical 输入是 `send_pcm(&[i16])`（16kHz mono s16le）；provider 内部自行转目标格式。
8. `open_timeout_ms` / `finalize_timeout_ms` 是 provider 私有默认（Doubao 12s/12s、Apple 5s/5s）；它们是 voice 层消费的 provider runtime option，不是协议字段。
9. `hotwords` 是 `Vec<String>`，provider 自由解释；不支持就静默忽略（`caps().hotwords=false`，doctor 提示）。
10. 实现文件顶部 `//!` 放官方协议/SDK 链接，quirk 写代码注释，不进本文档。

## provider 映射

| Provider | Partial | Segment | Final | 备注 |
|---|---|---|---|---|
| Apple SpeechAnalyzer (macOS 26+) | `isFinal=false` | `isFinal=true` | 不发，voice fallback concat | 本地优先；Swift helper 桥接；26 以下 fail-fast |
| Doubao SAUC | `definite=false` | `definite=true` | last response `result.text` | 云端；codec 写死 raw PCM |
| Tencent realtime ASR | `slice_type=1` | `slice_type=2` | 不发，`final=1` 只收口 Done | 云端；URL query 签名；`voice_format=1` |
| 纯 batch ASR API | — | — | — | **不入选** |

## Doubao SAUC 配置取舍

当前实现接 `wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async`，不是旧版
appid/token/cluster 协议。配置文件应尽量显式写出非鉴权默认值，让用户只填
`app_key` / `access_key` 就能用；空值只用于必填密钥占位和真正可选的 ID。

| 参数 | 官方/协议语义 | shuohua 默认 | 说明 |
|---|---|---|---|
| `resource_id` | Header 鉴权资源 ID | `volc.bigasr.sauc.duration` | 火山控制台资源标识；用户通常不改。 |
| `language` | `audio.language`；缺省让服务端自动识别 | `auto` | 加载时把 `auto` 解释为不发送该字段；显式语言如 `zh-CN` 会发送。 |
| `enable_itn` | 数字/文本归一化 | `true` | 开箱即用输出更适合上屏。 |
| `enable_punc` | 标点 | `true` | 开箱即用输出完整句子。 |
| `enable_ddc` | 顺滑/口语词处理 | `true` | 与本地 post 不冲突，按默认体验优先。 |
| `stream_mode` | 流式模式；官方推荐双向流式优化版本 | `2` | 显式使用优化模式；若服务端兼容性变化再调整。 |
| `ai_vad` | 豆包服务端语义 VAD | `false` | 云端断句/端点策略，不控制本地 idle，也不保证省费。 |
| `local_vad` | shuohua 本地 VAD 覆写 | `auto` | 非豆包协议字段；`auto/on/off` 分别为跟随全局/强制开/强制关。 |
| `open_timeout_ms` | 本地建连/初始化预算 | `12000` | shuohua runtime 选项。 |
| `finalize_timeout_ms` | 发末帧后的收口预算 | `12000` | shuohua runtime 选项。 |

官方通用 WebSocket 文档还列出 `reqid`、`sequence`、`nbest`、`confidence`、
`workflow`、`show_utterances`、`result_type`、`boosting_table_name`、
`correct_table_name`、`vad_signal`、`start_silence_time`、`vad_silence_time` 等字段。
其中 `reqid`/`sequence` 属协议帧机制，`show_utterances`/`result_type` 由 provider
固定以满足 `Segment` 契约；热词走 profile 的 `hotwords` 并映射为 `corpus.context`。
旧版服务端 VAD 字段暂不暴露，避免和当前 `ai_vad` / 本地 `local_vad` 混淆。

## 错误处理策略

不自动重试（用户重新触发录音即可，自动重试隐藏失败、增加调试难度）。dispatch 只在拼完才写剪贴板，没收到末段不上屏。
