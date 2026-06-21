# voice — 录音生命周期

**TL;DR**：`finish` 收尾、`engine` 运行期，两层不可互相反向依赖；控制信号走单一 `watch::Receiver<SessionControl>` + 显式 bool，别加第二条取消通道。

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

- **控制信号走单一 `watch::Receiver<SessionControl>`**（`Idle` / `Stop` / `Cancel`）：engine 在 `tokio::select!` 里和 PCM、ASR event 一起轮询，运行态用显式 bool（`active` / `stop_requested` / `cancel_requested`）推进。别再开第二条取消通道。
- **provider 主动 Done 用 `provider_done` 标志收口**：provider 自发 `AsrEvent::Done` 后 engine 不再发 `send_pcm(is_last=true)`、不再等第二个 Done，否则 VadPause 会把自发结束误判成 `asr_timeout`。
- **session 收口集中在 `finalize.rs`**：`Final` / `Segment` / `Done` / timeout / cancel 归一成 `FinalizeOutcome`；结果收齐到 `pending_segments` / `session_final_text`，`pending_overlay_segments` 计数避免重复推 overlay。别把收口逻辑散到 engine 主循环里。

## Continuous vs VadPause

`RecordingMode::{Continuous, VadPause}` 是固定模式，运行态由 engine 内的 Active/Idle 表达：

- **VadPause 仅当** `[voice.vad] backend = "silero"` **且** `asr/<provider>.toml.idle_pause = true` 同时成立，否则 Continuous。
- **Continuous**：始终向一个 provider session 发 PCM，不构造 Silero/timeline/pre-roll，不进 Idle，`sessions[]` ≤ 1。
- **VadPause**：Active↔Idle 自动切换。静音关 ASR（final 追加到 `pending_output`），有声开新 session（先 dump pre-roll 再喂当前帧）。段间无分隔符——provider 保证 emit 的 segment 直接 concat 即最终文本。

**provider 主动 Done**：任一模式下 provider 自发 `AsrEvent::Done` 都视为该 session 已结束；engine 不再发 `is_last`、不再等第二个 Done。VadPause 在此基础上转 Idle 等下一段，避免被当成 `asr_timeout`。

## 本模块持有的不变量

- **#3 停止必 drain residual + `stop_delay_ms`**（默认值见 `src/config/main.rs`），否则尾字被切。
- **#11 Idle 录音资源按模式分**：Continuous 未激活不持 cpal stream（避免 always-recording / 空闲 CPU 开销）；VadPause 的 Idle 继续持有并读 cpal stream 做 VAD/pre-roll，但 **Idle PCM 不发 provider**。
- **#12 麦克风可用性靠运行时 watchdog，不靠预检**：`recorder::start()` 同步段只校验 cpal build/play；真正判定是主 select 里一个 duration 首帧 watchdog（PCM 全 ≤ `MIN_NONZERO_AMPLITUDE` 判设备不可用，当前值见 `engine.rs`）。阈值不进配置——超过这个窗口几乎一定不是正常设备状态。
- **#13 Error/Timeout 路径不上屏不写剪贴板**：任意 terminal error → 跳过 post → 跳过 dispatch → history 写失败状态并保留累积 segments。一般 error 写 `status=Error`；ASR finalize 超时写 `status=Timeout` + `error.kind=asr_timeout`。无语音内容的早期失败（启动前失败、ASR 初连失败、watchdog 无音频）只走 overlay/UDS error/log，不写 history。半成品上屏是 bug。

## 终态与 history 触发

engine 返回后由 finish 决定写不写 history（schema 见 [schema §2](../schema.md)）。「有可归档内容」判据集中在 `voice::capture`，audio（engine）与 history record 构造（finish）共用以保持一致：喂过音频或有识别文本才落 history + retained audio；toggle 后立即 cancel、什么都没说则都不留，避免 TUI 无法关联的孤儿音频。

finish append 时只调用 `HistoryService::append`。append 成功后由 history 模块发布 history event 并维护内存统计；append 失败只走 schema 中的 `history_append` error/Notice 语义，不回滚已经完成的 dispatch。
