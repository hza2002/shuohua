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
- **运行期写入必须有界**：voice → session 命令队列、WebSocket Sink 和 Apple helper stdin 共用固定的内部 2s 写入上限；超时返回 `TransportTimeout`，按 terminal error 收口。WebSocket 写入被取消或超时后直接 drop transport，不等待无界的 graceful close，也不把连接放回复用池。

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
- 文件内**必须**包含 `type = "apple" | "aliyun" | "doubao" | "tencent"`；缺失或值不合法时 `resolve_instance` 报错，不回退默认。
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
| Aliyun Fun-ASR | `sentence_end=false` | `sentence_end=true` | 不发，voice fallback concat | 百炼 `/api-ws/v1/inference`；100ms PCM 帧；支持安全连接复用 |
| Doubao SAUC | `definite=false` | `definite=true` | last response `result.text` | 云端；codec 写死 raw PCM |
| Tencent realtime ASR | `slice_type=1` | `slice_type=2` | 不发，`final=1` 只收口 Done | 云端；URL query 签名；`voice_format=1` |
| 纯 batch ASR API | — | — | — | **不入选** |

## 阿里云百炼配置边界

`type = "aliyun"` 只接百炼实时 Fun-ASR 的 `/api-ws/v1/inference` 协议，不把同属
阿里云但协议不同的 Qwen Realtime、Omni、8k 电话模型或 HTTP 文件转写模型混入同一
provider。唯一受控预设是默认且推荐的 `fun-asr-realtime`：它在两个官方区域都可用、
用全量 Fun 语言集，无需任何跨字段联动。`model` 是 EditableSelect——可保持预设，
也可自由填写 custom 型号；custom 的模型存在性、区域与语言合法性一律交服务端判定并
正常上报错误。

字段控件由 schema/`field_view` 派生，新建（内存草稿）与编辑（已落盘文件）走同一套
（见 [config.md](config.md) 的「新建 = 编辑」）。暴露哪些字段以 `asr_aliyun_spec`
（`src/config/schema.rs`）为准；本节只讲代码读不出的边界与取舍。

`language_hints` 是单值（官方协议只读数组第一项），默认 `zh`，针对中文为主并夹
少量英文的 shuohua 用户。预设 `fun-asr-realtime` 下是 Fun 语言集 + 末尾 `auto` 的
Select（`auto` 生成空数组让服务端自动识别）；custom 型号无法可靠预知语言清单，改为
单值文本输入，填错由服务端报错。load 路径只做精简校验：恒限制至多一个 hint，且仅当
`model == "fun-asr-realtime"` 时才校验取值属于 Fun 语言集。配置 `vocabulary_id` 时，
官方只启用语言标记与 `language_hints` 匹配的热词；整段主要为英文时应显式选择 `en`。

`speech_noise_threshold` 官方没有默认值且要求充分测试后小步调整，因此未设置就不
下发该参数。

文件末尾单独放置 shuohua runtime 选项，避免与阿里云协议参数混淆：
`local_vad = "auto"`、`open_timeout_ms = 12000`、
`finalize_timeout_ms = 12000`。它们分别控制本地 VAD 覆写、建连/等待
`task-started` 的预算，以及 `finish-task` 后等待 `task-finished` 的预算。

以下官方字段不让用户配置：

- `format` / `sample_rate`：由 provider 固定为 canonical 16kHz mono s16le PCM。
- `task_group` / `task` / `function` / `streaming` / `task_id`：协议控制字段。
- `input.context`：支持它的 Fun-ASR 型号由 profile `hotwords` 自动映射，限制在官方
  400 字符预算内；其他型号通过 `vocabulary_id` 使用控制台热词表。
- `continue-task`：录音过程中 profile 上下文不变化，没有运行期更新需求。

连接使用 workspace 专属区域域名。正常 `task-finished` 后可在官方 60 秒 idle
期限内复用，shuohua 只保留一个连接并在 50 秒内取用；复用在发送音频前失败时
允许新建连接重试一次。任务开始后不自动重放音频，避免重复计费和重复文本。

## Doubao SAUC 配置取舍

当前实现接 `wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async`，不是旧版
appid/token/cluster 协议。配置文件应尽量显式写出非鉴权默认值，让用户只填
`app_key` / `access_key` 就能用；空值只用于必填密钥占位和真正可选的 ID。

字段与默认值以 schema（`src/config/asr/doubao.rs` / `schema.rs`）为准，这里只记代码
读不出的取舍：

- **开箱即用默认开** `enable_itn` / `enable_punc` / `enable_ddc`：让未经 post 的原始
  输出就适合直接上屏；`enable_ddc`（顺滑/口语词）与本地 post 链不冲突，故按体验优先。
- **`resource_id`** 默认 2.0（seed）`volc.seedasr.sauc.duration`；1.0 是
  `volc.bigasr.sauc.duration`。TUI 是 curated + 自由文本 EditableSelect，两版及各自
  并发版都可自填。
- **`language`** 缺省 `auto`：加载时解释为「不发送该字段」交服务端自动识别，显式语言
  （如 `zh-CN`）才下发。
- **`stream_mode`** 显式用官方推荐的双向流式优化模式，服务端兼容性变化再调整。
- **`ai_vad` 与 `local_vad` 是两回事**：`ai_vad` 是豆包服务端语义 VAD（云端断句/端点，
  不控制本地 idle、也不保证省费）；`local_vad` 是 shuohua 本地 VAD 覆写（非豆包协议字段，
  `auto/on/off` 跟随全局/强制开/强制关）。

以下官方字段 provider 固定或刻意不暴露：`format`/`sample_rate` 固定为 canonical PCM；
`reqid`/`sequence` 属协议帧机制；`show_utterances`/`result_type` 由 provider 固定以满足
`Segment` 契约；热词走 profile `hotwords` 映射为 `corpus.context`；旧版服务端 VAD 字段
（`vad_signal`/`start_silence_time`/`vad_silence_time` 等）不暴露，避免和 `ai_vad` /
`local_vad` 混淆。

## 错误处理策略

不自动重试（用户重新触发录音即可，自动重试隐藏失败、增加调试难度）。dispatch 只在拼完才写剪贴板，没收到末段不上屏。
