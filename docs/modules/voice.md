# voice — 录音生命周期

**TL;DR**：`finish` 收尾、`engine` 运行期，两层不可互相反向依赖；控制信号是 `SessionControl`——两个 level-triggered 终态闩（`stop` / `cancel`，`tokio_util::CancellationToken`），按消费模式分发，别退回 watch 边沿语义。

> **何时读**：改录音状态机、VAD 暂停/多段 session、engine/finish 边界、终止/取消/超时处理。
> **不在这里**：UDS/history 字段格式见 [schema](../schema.md)；ASR provider 契约见 [asr](asr.md)。
> **代码**：`src/voice/`（`finish.rs` 公开入口，`engine.rs` 运行期）。

## 边界（不可越）

`finish::run_recording` 是唯一公开入口。两层分工是硬边界：

- **engine** = 录音运行期：PCM 路由、ASR event、stop drain、provider finalize、错误/取消、retained audio。返回 `EngineOutcome`。
- **finish** = 收尾：post chain、dispatch、构造 history record、调用 `HistoryService` append、最终 StateStore/Overlay。

engine **不调** post chain 执行（`voice::post_dispatch`，逐步复用 `post::run_step`）/ `voice::dispatch::dispatch` / history append。engine 对 post 的唯一接触面：用 `post::AppContext` 当数据载体、stop 时调一次 `frontmost_app()`、读 `SessionParams.post_chain.name` 当 overlay header 字符串。改动时别让 engine 反向依赖 post/dispatch/history。

voice 只负责把一次 recording 的 capture/ASR/post 结果翻译成 `HistoryRecord`。durable append、history event、pagination、stats/analytics、deletion、retained-audio association 都归 `HistoryService`；voice 不直接读写 JSONL store primitives，也不持有 history index。

`engine::run` 只负责 recorder 启动这一个 `!Send` 边界，其余在 `run_with_recorder`。测试用 `RecordingStream::for_test` 不依赖 cpal 即可驱动整条生命周期（`engine_lifecycle_tests.rs`）。

capture backend 只允许在 `engine::CaptureStream` 边界分叉。默认 backend 是 `webrtc`
（cpal 采集 + recorder 线程内插入 WebRTC APM）；`voice.preprocess.backend = "off"` 是
cpal 原始采集；`voice.preprocess.backend = "apple"` 走 Apple VP 采集。所有 backend 都必须向 engine 提供同一个
canonical 契约：16 kHz mono i16 PCM、terminal error 与正常 EOF 可区分、
stop 后可 drain residual。不要把 Apple helper、TCC、AVAudioEngine 等平台细节泄漏到
VAD、ASR、finalize、finish 或 post/history。

Apple backend 是纯 VP 采集，不做 raw→Apple bridge/handoff/mix；定位是音频环境差时的
兜底。Apple helper 启动或 start 失败时，本次录音可降级到 raw/off 完成并记录 warn；
Apple 启动成功后，后续 Apple EOF/error 属于该 capture 的正常 EOF 或 terminal error。

WebRTC backend 只启用 high-pass、noise suppression 和保守 digital AGC1（锁定 curated
preset，APM 固定 16k）；不做 AEC，不接 ScreenCaptureKit system audio，不加 ducking。它
的目标是保持接近 `off` 的启动路径：同步启动仍只做 cpal build/play 和轻量 processor 初始
化，10ms frame chunking 在 recorder 线程完成，engine 不感知 WebRTC 细节。参数取舍与刻意
不接的能力见 [webrtc_backend.md](webrtc_backend.md)。

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

- **VadPause 仅当** 当前 ASR 的 `local_vad` 解析结果启用本地 VAD 时成立：
  `auto` 跟随 `[voice.vad] backend`，`on` 强制使用 Silero，`off` 强制 Continuous。
- **Continuous**：capture 与初始 provider session 同时启动，始终向一个 provider session 发 PCM，不构造 Silero/timeline/pre-roll，不进 Idle，`sessions[]` ≤ 1。
- **VadPause**：从 Idle-listening 开始：只启动 capture + Silero/timeline/pre-roll，不打开 provider session。VAD `SpeechStarted` 后才打开第一个/下一个 ASR session（先 dump pre-roll 再喂当前帧）并进入 Active；VAD `SilenceStarted` 或 provider `Done` 后关闭当前 session 回 Idle。段间无分隔符——provider 保证 emit 的 segment 直接 concat 即最终文本。

**provider 主动 Done**：任一模式下 provider 自发 `AsrEvent::Done` 都视为该 session 已结束；engine 不再发 `is_last`、不再等第二个 Done。VadPause 在此基础上转 Idle 等下一段，避免被当成 `asr_timeout`。

## 本模块持有的不变量

- **停止要 best-effort drain residual + `stop_delay_ms`**（默认值见 `src/config/main.rs`），否则尾字容易被切；但 residual drain 必须有内部上限，超时后丢 late residual 并继续 provider finalize / post，不能阻塞整条收尾流水线。
- **Continuous 启动期不丢麦克风已产出的 PCM**：capture 已 ready 但初始 ASR session
  仍在 open/connect/init 时，engine 必须持续读 PCM 到启动缓存；ASR open 成功后按原顺序先
  回放缓存再进入实时流。缓存不另设秒数上限，生命周期由同一个
  `open_timeout_ms` 约束；cancel 丢弃缓存，stop 则 drain residual 后继续等 open
  成功或超时，以保证用户按键后说出的开头音频不被静默丢掉。
- **Apple capture helper 复用进程，但不复用 engine**：helper server 可跨录音复用以省掉 spawn/TCC
  延迟；Swift `VoiceProcessedCapture.start()` 必须每次新建 `AVAudioEngine`/inputNode/converter，
  `stop()` 后释放，避免同一个 VPIO 反复 toggle voice processing。`RunningAppleVpSource`
  正常 stop 后保留 server；未正常 stop/drop/stop 超时会 abort server_loop → `Child(kill_on_drop)`
  被杀，并清掉 reusable slot，作为 wedge 兜底。
- **Apple stop/drain 与 cpal 等价**：`backend = "apple"` 虽然没有 raw tap，也不发布 retained
  audio，但 stop 阶段仍要把 helper 返回的 residual PCM 继续送给 ASR，不能把 stop 简化成
  kill helper 或丢弃队列。
- **Apple backend 不发布 retained audio**：需要完整原始音频留存时用
  `voice.preprocess.backend = "off"`。
- **VadPause 首段 resume 必须使用 startup evidence 防 late trigger**：首段 speech 可能在
  Silero `SpeechStarted` 前已经有有效人声电平，尤其 Apple VP 初始化期间更明显。Idle 阶段要在
  recording timeline 上记录首个有效信号样本；首段 resume 起点必须同时考虑
  `speech_start - pre_roll_ms` 和 `first_signal - startup_margin`，再受 overlap / oldest
  bounds 约束。这个规则不按 backend 特判；Apple/off 都从同一条 capture stream 取证据。
  Timeline retention 必须覆盖 startup signal lookback；若极端 late trigger 仍被 oldest bound
  截断，`VadPause resume replay window` 日志必须以 `replay_clamped=true` 暴露这个有界降级。
- **Idle 录音资源按模式分**：Continuous 未激活不持 cpal stream（避免 always-recording / 空闲 CPU 开销）；VadPause 的 Idle 继续持有并读 cpal stream 做 VAD/pre-roll，但 **Idle PCM 不发 provider**，也不计入 `sessions[]`/provider audio。
- **麦克风可用性靠运行时 watchdog，不靠预检**：`recorder::start()` 同步段只校验 cpal build/play；Continuous 的真正判定是主 select 里一个 duration 首帧 watchdog（PCM 全 ≤ `MIN_NONZERO_AMPLITUDE` 判设备不可用，当前值见 `engine.rs`）。VadPause 初始 Idle 静音是合法状态，不走 no-audio watchdog；只有进入 Active、开始向 ASR 送音频后才适用同类运行时错误语义。阈值不进配置——超过这个窗口几乎一定不是正常设备状态。
- **Error/Timeout 路径不上屏不写剪贴板**：任意 terminal error → 跳过 post → 跳过 dispatch → history 写失败状态并保留累积 segments。一般 error 写 `status=Error`；ASR finalize 超时写 `status=Timeout` + `error.kind=asr_timeout`。无语音内容的早期失败（启动前失败、ASR 初连失败、watchdog 无音频）只走 overlay/UDS error/log，不写 history。半成品上屏是 bug。
- **Resume recording 不是 engine pause/resume**：resume 热键只在 daemon 启动一次全新的 recording 前读取最新一条 history，符合条件时把旧 ASR 文本作为 seed 交给 finish。engine 仍是普通 recording 生命周期；不要新增第三个控制信号，也不要在旧 history 上写 checkpoint。
- **Partial 快照按终态取舍（`PartialTextPolicy`）**：`current.partial_text` 在录音主循环（`handle_asr_event`）和 finalize（`finalize_provider_session`，send_last 之后才到的 partial）两处同源维护——`Partial` 写、`Segment`/`Final` 清。session 收口时只有可恢复终态（`cancel_requested` 或 `error.kind=asr_timeout`）把最后一个 tentative `Partial` 保留进 `SessionCapture.partial_text`；正常完成 / VadPause 中途轮转一律丢弃 tentative partial，不把它当 final ASR。取舍理由：误 ESC / finalize 超时时用户已经在 overlay 看到 partial，丢了会让 `asr.text` 为空、resume 无法恢复；而成功路径必须以 provider 的 `Segment/Final` 为准。`session_text` 的优先级固定 `final_text` > `segments + partial`，避免把已提交段和残余 partial 叠加重复。

## 终态与 history 触发

engine 返回后由 finish 决定写不写 history（schema 见 [schema §2](../schema.md)）。「有可归档内容」判据集中在 `voice::capture`，audio（engine）与 history record 构造（finish）共用以保持一致：喂过 provider 音频或有识别文本才落 history + retained audio；toggle 后立即 cancel、VadPause 启动后一直没说话就 stop/cancel，都不留 history 或 retained audio，避免 TUI 无法关联的孤儿音频。

resume seed 只参与本次 recording 的 post/dispatch/history 构造：有新的 ASR 文本时，post chain 从「旧 ASR 文本 + 新 ASR 文本」完整重跑；没有新的 ASR 文本时，不复用旧文本做 post/dispatch，也不写一条只含 seed 的 history。

**resume 录音「有内容」判据比普通录音更严**（`has_archivable_content_for`，engine retained audio 与 finish history 同源）：普通录音喂过音频或有识别文本即可；resume 录音必须有**新的** ASR 文本才写 history + 留 retained audio。只有音频、没识别出新文本（如环境噪音）时不写记录、不留音频——否则会 append 一条空记录盖在它想续写的那条可恢复记录上，让下一次 resume 只看最新一条时断链。取消 / 正常 / 超时路径同此判据。

录音起始的 overlay 提示由 engine（`apply_start_notice`）按 `RecordingStart` 三态发：`Seed` 续写时把旧 ASR 文本作为「已提交 segment」铺到 overlay / StateStore 并发「继续上一段」notice，让用户直观看到接着上次继续说；`NewFromResume`（按了 resume 热键但无可恢复记录）只发「新录音」notice，确认热键生效；`Fresh`（普通 trigger 开始）不发。seed 回显纯展示：不进 capture `sessions[]`、不占 provider audio、不影响 finish 的 seed 拼接逻辑。这些 notice 必须由 engine 在 `SetState(Connecting)` 清屏之后发——在 daemon 侧先发会被随后的 Connecting 清掉（这正是当初 daemon 侧发 notice 不显示的根因）。`SessionParams.start: RecordingStart` 由 daemon 决策后传入，是「本次是不是 resume / 有没有 seed」的唯一事实来源（finish 的 seed 拼接、archival 判据都读它）。

finish append 时只调用 `HistoryService::append`。append 成功后由 history 模块发布 history event 并维护内存统计；append 失败只走 schema 中的 `history_append` error/Notice 语义，不回滚已经完成的 dispatch。
