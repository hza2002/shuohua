# voice — 录音生命周期

**TL;DR**：`finish` 收尾、`engine` 运行期，两层不可互相反向依赖；控制信号是 `SessionControl`——两个 level-triggered 终态闩（`stop` / `cancel`，`tokio_util::CancellationToken`），按消费模式分发，别退回 watch 边沿语义。

> **何时读**：改录音状态机、VAD 暂停/多段 session、engine/finish 边界、终止/取消/超时处理。
> **不在这里**：UDS/history 字段格式见 [schema](../schema.md)；ASR provider 契约见 [asr](asr.md)。
> **代码**：`src/voice/`（`finish.rs` 公开入口，`engine.rs` 运行期）。

## 边界（不可越）

`finish::run_recording` 是唯一公开入口。两层分工是硬边界：

- **engine** = 录音运行期：PCM 路由、ASR event、stop drain、provider finalize、错误/取消、retained audio。返回 `EngineOutcome`。
- **finish** = 收尾：post chain、dispatch、构造 history record、调用 `HistoryService` append、最终 StateStore/Overlay。

engine **不调** `post::run_chain` / `voice::dispatch::dispatch` / history append。engine 对 post 的唯一接触面：用 `post::AppContext` 当数据载体、stop 时调一次 `frontmost_app()`、读 `SessionParams.post_chain.name` 当 overlay header 字符串。改动时别让 engine 反向依赖 post/dispatch/history。

voice 只负责把一次 recording 的 capture/ASR/post 结果翻译成 `HistoryRecord`。durable append、history event、pagination、stats/analytics、deletion、retained-audio association 都归 `HistoryService`；voice 不直接读写 JSONL store primitives，也不持有 history index。

`engine::run` 只负责 recorder 启动这一个 `!Send` 边界，其余在 `run_with_recorder`。测试用 `RecordingStream::for_test` 不依赖 cpal 即可驱动整条生命周期（`engine_lifecycle_tests.rs`）。

## 为什么这么写（别"简化"掉）

- **控制信号 = `SessionControl`，两个 level-triggered 终态闩**（`stop` / `cancel`，各是一个 `CancellationToken`），按消费模式分发，**别退回 `watch` + `borrow_and_update` 的边沿语义**：
  - `cancel` 广播给所有阶段（engine / finalize / drain / post-dispatch），但下游只拿 `cancel_signal()` 返回的只读 `CancelSignal` 视图——只能 await/查询，**既看不到 stop、也无法主动触发 cancel**（触发权留在持 `SessionControl` 的 daemon 侧）。这把「stop 引擎私有、下游对 cancel 只读」做成类型保证而非约定。
  - `stop` 只有 engine 的 `'active` / `'idle` 边界关心；finalize / drain / post **拿不到它**，从结构上杜绝把 Stop 边沿吞掉（这正是「按 trigger 停不下来、只能 ESC」卡死的根因——见 git history `fix/stop-signal-wedge`）。
  - 闩单调（仅 未置位→置位），**cancel 优先于 stop**：每个 `select!` 把 `cancelled()` 排在 `stopped()` 前（`biased`），电平复核也先查 cancel。
  - **正确性永远来自电平**（`is_stop_requested()` / `is_cancelled()` / `cancelled()` / `stopped()`），`select!` 的 await 只当唤醒。典型：finalize 只观察 cancel，所以进 Idle 前 engine 必须电平复核一次 stop，否则 finalize 窗口内发来的 stop 会被漏掉。运行态仍用显式 bool（`active` / `stop_requested` / `cancel_requested`）推进。别再开第三条信号或回退到一次性边沿。
  - **terminal latch 只适合「单向、一次性、不可回退」的 stop/cancel。** 未来若加 *可回退* 的控制（如 pause/resume、restart session），那是另一种语义，需要另配原语（如 level-read 的 `watch<bool>` 或 `Notify` 对），**不要**硬塞成第三个 `CancellationToken`——token 无法 un-cancel。
- **provider 主动 Done 用 `provider_done` 标志收口**：provider 自发 `AsrEvent::Done` 后 engine 不再发 `send_pcm(is_last=true)`、不再等第二个 Done，否则 VadPause 会把自发结束误判成 `asr_timeout`。
- **session 收口集中在 `finalize.rs`**：`Final` / `Segment` / `Done` / timeout / cancel 归一成 `FinalizeOutcome`；结果收齐到 `pending_segments` / `session_final_text`，`pending_overlay_segments` 计数避免重复推 overlay。别把收口逻辑散到 engine 主循环里。

## Continuous vs VadPause

`RecordingMode::{Continuous, VadPause}` 是固定模式，运行态由 engine 内的 Active/Idle 表达：

- **VadPause 仅当** `[voice.vad] backend = "silero"`、`asr/<provider>.toml.idle_pause = true`
  且当前平台已提供 Silero runtime 时同时成立，否则 Continuous。
- **Continuous**：始终向一个 provider session 发 PCM，不构造 Silero/timeline/pre-roll，不进 Idle，`sessions[]` ≤ 1。
- **VadPause**：Active↔Idle 自动切换。静音关 ASR（final 追加到 `pending_output`），有声开新 session（先 dump pre-roll 再喂当前帧）。段间无分隔符——provider 保证 emit 的 segment 直接 concat 即最终文本。

**provider 主动 Done**：任一模式下 provider 自发 `AsrEvent::Done` 都视为该 session 已结束；engine 不再发 `is_last`、不再等第二个 Done。VadPause 在此基础上转 Idle 等下一段，避免被当成 `asr_timeout`。

## VAD-only preprocessing

`voice::preprocess::VadPreprocessor` 只处理送给本地 VAD backend 的 PCM 副本。它不得改变：

- recorder 原始 PCM；
- 发给 ASR provider 的 PCM；
- history/audio 关联语义。

当前 Windows backend 在 VAD 副本上启用自适应增益：按 Silero 512-sample frame 估算 RMS/peak，低于噪声门限时让 gain 回落到 1x；有效语音帧按目标 RMS 计算 gain，并用 attack/release 平滑跨帧变化。这个处理是 Windows microphone-level calibration baseline，不是最终跨平台 audio-processing 结论。macOS 当前保持 passthrough，未来只有在 macOS A/B runtime 验证不退化后才考虑共用。

retained audio 是单独的发布路径：recorder 仍先写 16 kHz mono 原始临时 WAV；`voice::audio::finish` 在转成
FLAC/M4A 前会对临时 WAV 做一次保守 loudness normalization。该处理按有效样本 RMS 提升低电平录音，
同时用 peak headroom 防止削波；它只改善保存文件的回放电平，不回流到 VAD、ASR provider、状态机或
上屏文本。

Silero 概率由 `VadController` 做统一端点判定：从 Silence 进入 Speech 使用配置中的 `threshold`，已经处于 Speech 后使用派生的较低 exit threshold 累计静音，避免概率在阈值附近抖动导致过早 pause。`pause_silence_ms` 仍按显式配置解释，不做 Windows 隐式覆盖；诊断命令和 runtime 必须走同一套 controller。

不要在 `silero.rs` 里直接堆平台增益逻辑；替换 WebRTC APM、纯 Rust AGC/NS 或其他成熟 pipeline 时，应优先作为 `VadPreprocessor` backend 接入。

成熟语音输入产品通常不会把 frame threshold、pause window、gain 参数完整暴露给用户；产品配置最终应收敛到少量 sensitivity/mode 档位，底层 pipeline 自己处理设备电平差异。当前详细 `[voice.vad]` 字段仍是开发期可观测/可调试边界，不是最终 UX 形态。

已调研的替换方向：

- WebRTC Audio Processing Module 是算法成熟度最高的方向，覆盖 AGC、noise suppression、high-pass、AEC 等实时语音处理组件；但当前 Rust `webrtc-audio-processing` wrapper 默认动态链接系统库，`bundled` 静态路径在 Windows/MSVC 下依赖 Unix 风格 build tools，不能直接满足 shuohua 的三端单二进制分发约束。
- `sonora-agc2` 是纯 Rust WebRTC AGC2/RNN VAD 组件，Windows/MSVC 构建风险低，但它不是完整 APM，接入前需要重新设计 frame metadata、speech probability、noise/speech level 估计和 Silero 前处理关系。

因此主线暂时保留简单 `VadPreprocessor` baseline，直到某个候选方案同时通过 packaging、license、Windows/macOS/Linux build 和真实 VAD smoke。

## 本模块持有的不变量

- **停止必 drain residual + `stop_delay_ms`**（默认值见 `src/config/main.rs`），否则尾字被切。
- **Idle 录音资源按模式分**：Continuous 未激活不持 cpal stream（避免 always-recording / 空闲 CPU 开销）；VadPause 的 Idle 继续持有并读 cpal stream 做 VAD/pre-roll，但 **Idle PCM 不发 provider**。
- **麦克风可用性靠运行时 watchdog，不靠预检**：`recorder::start()` 同步段只校验 cpal build/play；真正判定是主 select 里一个 duration 首帧 watchdog（PCM 全 ≤ `MIN_NONZERO_AMPLITUDE` 判设备不可用，当前值见 `engine.rs`）。阈值不进配置——超过这个窗口几乎一定不是正常设备状态。
- **Error/Timeout 路径不上屏不写剪贴板**：任意 terminal error → 跳过 post → 跳过 dispatch → history 写失败状态并保留累积 segments。一般 error 写 `status=Error`；ASR finalize 超时写 `status=Timeout` + `error.kind=asr_timeout`。无语音内容的早期失败（启动前失败、ASR 初连失败、watchdog 无音频）只走 overlay/UDS error/log，不写 history。半成品上屏是 bug。

## 终态与 history 触发

engine 返回后由 finish 决定写不写 history（schema 见 [schema §2](../schema.md)）。「有可归档内容」判据集中在 `voice::capture`，audio（engine）与 history record 构造（finish）共用以保持一致：喂过音频或有识别文本才落 history + retained audio；toggle 后立即 cancel、什么都没说则都不留，避免 TUI 无法关联的孤儿音频。

finish append 时只调用 `HistoryService::append`。append 成功后由 history 模块发布 history event 并维护内存统计；append 失败只走 schema 中的 `history_append` error/Notice 语义，不回滚已经完成的 dispatch。
