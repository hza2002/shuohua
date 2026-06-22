# post — 后处理链

**TL;DR**：链不阻塞，最差产出 = raw；单步失败/超时跳过并推 overlay notice；profile 在 toggle ON 时取一次 frontmost app 选定，录音中途不重选。

> **何时读**：改 processor、链执行规则、profile 路由、prompt 变量。
> **不在这里**：pipeline[] / history 字段见 [schema](../schema.md)；overlay notice 通道见 [overlay](overlay.md)。
> **代码**：`src/post/`（`mod.rs` trait+run_chain / `zh_filter.rs` / `llm.rs`）；profile 路由在 `src/daemon/session_start.rs` + `src/config/profile.rs`，前台 App 查询在 `src/platform/desktop.rs`。

## 数据形态与 trait

`PipelineText { raw, segments, text }`（`raw` 整条链不变，作回退/记录；`text` 是 in-flight 版本；`segments` 是本次 recording 的 ASR session 文本列表）+ `AppContext { bundle_id, app_name }`（整条链共享）。`PostProcessor::process(input, ctx) -> Result<PipelineText, PostError>`，类型定义见 `mod.rs`。

## 链执行两条规则

- **链不阻塞，最差是 raw**：失败/超时跳过该步，下一个继续用 upstream 的 text。链路始终产出（最差 == raw）。不假设"后面会补"。
- **失败/超时都推 notice**：`run_chain` 返回 `steps`，caller（`finish`）遍历对每个非 Ok/Skipped 状态发 `OverlayCmd::Notice`，把"哪步失败"的 UI 决策跟链执行解耦。所有失败+时延写进 `pipeline[]`（history + UDS）。

## Profile 路由

toggle ON 时取一次 `frontmost_bundle_id`，按 `config.toml` 的 `[profile]` 表查包含该 bundle id 的 profile；没命中用 `default`。**命中多个 profile → 报配置错，不猜**。该 profile 决定本次 ASR provider、hotwords、provider 覆盖、post chain。toggle OFF 时只再取一次 AppContext 当 prompt 变量，**不重选 profile**（避免录音中切 App 导致配置中途变化）。

## 内置 processors

`build_processor` 只构造两种；透传 = 空 chain（无 processor 类型）：

| 名 | type | 作用 |
|---|---|---|
| `ZhFilter` | `rule` | 中文语音文本过滤：标点/空白/segment 边界/少量语气词 |
| `LlmCleanup` | `llm` | 调 OpenAI 兼容 / Anthropic native 一次性 API |

`LlmCleanup` prompt 变量替换：`{{app_name}}` / `{{bundle_id}}` / `{{text}}`。

## 本模块持有的不变量

- `frontmostApplication` 在 toggle OFF 瞬间取一次缓存，**不在 processor 内反复取**——pipeline 跑期间用户可能切走，会拿到错的 app。

## 不做的事

不做内容审查（敏感词/政治/隐私都不做，只清洗不审查）；粒度到 bundle_id 为止，不做 per-URL/字段匹配；不允许 processor 整段拒绝输出（失败只能跳过，链路始终产出）。
