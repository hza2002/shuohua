# post — 后处理链

**TL;DR**：链不阻塞，最差产出 = raw；单步失败/超时跳过并推 overlay notice；profile 在 toggle ON 时取一次 frontmost app 选定，录音中途不重选。

> **何时读**：改 processor、链执行规则、profile 路由、prompt 变量。
> **不在这里**：pipeline[] / history 字段见 [schema](../schema.md)；overlay notice 通道见 [overlay](overlay.md)。
> **代码**：`src/post/`（`mod.rs` trait+`run_step`（单步：超时/失败跳过）/ `zh_filter.rs` / `llm.rs` / `app_context.rs`）；整条 chain 的循环 + cancel + overlay notice 在 `src/voice/post_dispatch.rs`；profile 路由在 `src/daemon/session_start.rs` + `src/config/profile.rs`。

## 数据形态与 trait

`PipelineText { raw, segments, text }`（`raw` 整条链不变，作回退/记录；`text` 是 in-flight 版本；`segments` 是本次 recording 的 ASR session 文本列表）+ `AppContext { bundle_id, app_name }`（整条链共享）。`PostProcessor::process(input, ctx) -> Result<PipelineText, PostError>`，类型定义见 `mod.rs`。

## 链执行规则

- **链不阻塞，最差是 raw**：失败/超时跳过该步，下一个继续用 upstream 的 text。链路始终产出（最差 == raw）。不假设"后面会补"。
- **失败/超时都推 notice**：dispatch 边界遍历每个非 Ok/Skipped step 发 `OverlayCmd::Notice`，把"哪步失败"的 UI 决策跟 processor 实现解耦。所有失败+时延写进 `pipeline[]`（history + UDS）。
- **取消保留已完成步骤**：post 期间取消会把已完成的 in-memory pipeline steps 写进最终 `canceled` history；正在执行的步骤没有结果，不记录。对 `canceled` 来说这些 steps 只是观察数据，顶层 `text` 仍是 raw ASR，不从最后一个 Ok step 派生。history 仍只在 recording 结束时 append 一次，不做每步 JSONL 写入或旧记录修改。

## Profile 路由

toggle ON 时取一次 `frontmost_bundle_id`，按 `config.toml` 的 `[profile]` 表查包含该 bundle id 的 profile；没命中用 `default`。**命中多个 profile → 报配置错，不猜**。该 profile 决定本次 ASR provider、hotwords、provider 覆盖、post chain。toggle OFF 时只再取一次 AppContext 当 prompt 变量，**不重选 profile**（避免录音中切 App 导致配置中途变化）。

profile 可用 `[post.overrides.<id>]` 覆盖 chain 中某个 llm 组件的字段（`<id>` 是 chain 里的裸组件 id，目标须 `type = "llm"`）；覆盖表按 llm schema 校验，运行期合并进该组件、对 rule 组件忽略。约束与硬校验见 [config.md](config.md#配置实例契约)。

## 内置 processors

`build_processor` 只构造两种；透传 = 空 chain（无 processor 类型）：

| 名 | type | 作用 |
|---|---|---|
| `ZhFilter` | `rule` | 中文语音文本过滤：标点/空白/segment 边界/少量语气词 |
| `LlmCleanup` | `llm` | 调 OpenAI 兼容 / Anthropic native 一次性 API |

`LlmCleanup` prompt 变量替换：`{{app_name}}` / `{{bundle_id}}` / `{{text}}`。

配置期的 LLM 创建表单可用 provider Models API 拉取模型 ID：OpenAI 兼容走
`GET <base_url>/models`，Anthropic native 走 `GET <base_url>/v1/models`。这只影响
TUI 里的模型选择列表；`model` 仍是普通字符串配置，用户可手填自定义模型。

## 本模块持有的不变量

- `frontmostApplication` 在 toggle OFF 瞬间取一次缓存，**不在 processor 内反复取**——pipeline 跑期间用户可能切走，会拿到错的 app。

## 不做的事

不做内容审查（敏感词/政治/隐私都不做，只清洗不审查）；粒度到 bundle_id 为止，不做 per-URL/字段匹配；不允许 processor 整段拒绝输出（失败只能跳过，链路始终产出）。
